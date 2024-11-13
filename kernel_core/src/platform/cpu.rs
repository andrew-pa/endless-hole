//! CPU management

use log::{debug, info};

#[cfg(test)]
use mockall::automock;
use snafu::{ensure, OptionExt, ResultExt, Snafu};

use crate::{
    memory::{PageAllocator, PhysicalAddress, VirtualAddress},
    platform::device_tree::{DeviceTree, NodeNotFoundSnafu, OwnedParseError, UnexpectedValueSnafu},
};

/// A unique identifier for a single CPU core.
pub type Id = usize;

/// Errors that occur due to power management operations.
#[derive(Debug, Snafu)]
pub enum PowerManagerError {
    /// Target core ID is invalid.
    InvalidCoreId,
    /// Entry point address is invalid.
    InvalidAddress,
    /// The target core is already on.
    AlreadyOn,
    /// The target core is still booting.
    Pending,
    /// A miscellaneous internal error has occured.
    Internal,
}

/// Mechanism interface for managing CPU power state.
#[cfg_attr(test, automock)]
pub trait PowerManager {
    /// Powers on a core that is currently off.
    /// The core will start executing at `entry_point_address`, with `arg` passed as the argument.
    ///
    /// # Errors
    /// Returns an error if the underlying hardware interface fails to start the core or rejects
    /// the given parameters.
    ///
    /// # Safety
    /// The entry point address must be valid or else undefined behavior will occur on the target core.
    unsafe fn start_core(
        &self,
        target_core: Id,
        entry_point_address: PhysicalAddress,
        arg: usize,
    ) -> Result<(), PowerManagerError>;

    /// The string value of the "enable-method" device tree property that indicates that a core can
    /// be enabled with this interface.
    fn enable_method_name() -> &'static [u8];
}

/// Errors that can occur during SMP bring-up.
#[derive(Debug, Snafu)]
pub enum BootAllCoresError {
    /// Error parsing device tree information about CPUs.
    #[snafu(display("Parsing device tree: {cause}"))]
    DeviceTree {
        /// Cause of the error from the device tree.
        cause: OwnedParseError,
    },
    /// An "enable-method" was given that is unsupported.
    #[snafu(display("Unsupported CPU enable method: {method}"))]
    UnsupportedEnableMethod {
        /// The method that we don't support.
        method: alloc::string::String,
    },
    /// Error occured actually starting a core.
    Power {
        /// Underlying error.
        source: PowerManagerError,
    },
    /// Failed to allocate stack for new core thread.
    Memory {
        /// Underlying error.
        source: crate::memory::Error,
    },
}

/// Power on all cores as described by the device tree, and allocates stacks for them.
///
/// # Errors
/// Errors can come from parsing the device tree, finding an unsupported enable method, the power
/// management interface, or the memory allocator.
/// See [`BootAllCoresError`] for details.
pub fn boot_all_cores<PM: PowerManager>(
    device_tree: &DeviceTree,
    power: &PM,
    entry_point_address: PhysicalAddress,
    page_allocator: &impl PageAllocator,
) -> Result<(), BootAllCoresError> {
    let cpu_nodes = device_tree
        .iter_nodes_named(b"/cpus", b"cpu")
        .context(NodeNotFoundSnafu {
            path: "/cpus/cpu@*",
        })
        .map_err(|s| BootAllCoresError::DeviceTree {
            cause: s.to_owned(),
        })?;

    let mut successful = 0;

    for cpu_node in cpu_nodes {
        let mut cpu_id = None;

        for (name, value) in cpu_node.properties {
            match name {
                b"enable-method" => {
                    let enable_method =
                        value
                            .as_bytes(name)
                            .map_err(|s| BootAllCoresError::DeviceTree {
                                cause: s.to_owned(),
                            })?;

                    ensure!(
                        enable_method == PM::enable_method_name(),
                        UnsupportedEnableMethodSnafu {
                            method: core::str::from_utf8(enable_method).unwrap_or("unknown")
                        }
                    );
                }
                b"reg" => {
                    let ids = value
                        .as_reg(name)
                        .map_err(|s| BootAllCoresError::DeviceTree {
                            cause: s.to_owned(),
                        })?;
                    let id = ids
                        .iter()
                        .next()
                        .with_context(|| UnexpectedValueSnafu {
                            name,
                            value: value.clone(),
                            reason: "reg (CPU id) should have at least one element",
                        })
                        .map(|(a, _)| a)
                        .map_err(|s| BootAllCoresError::DeviceTree {
                            cause: s.to_owned(),
                        })?;
                    cpu_id = Some(id);
                }
                _ => {}
            }
        }

        let cpu_id = cpu_id.context(DeviceTreeSnafu {
            cause: OwnedParseError::PropertyNotFound { name: "reg" },
        })?;

        if cpu_id == 0 {
            // skip the boot cpu
            continue;
        }

        // allocate a new 4MiB stack for the core kernel thread.
        let stack_size = 4 * 1024 * 1024;
        let stack: VirtualAddress = page_allocator
            .allocate(stack_size / page_allocator.page_size())
            .context(MemorySnafu)?
            .byte_add(stack_size)
            .into();

        debug!(
            "starting cpu@{}, id=0x{cpu_id:x}, stack@{stack:?}",
            cpu_node
                .unit_address
                .and_then(|s| core::str::from_utf8(s).ok())
                .unwrap_or("?")
        );

        unsafe {
            power
                .start_core(cpu_id, entry_point_address, stack.into())
                .context(PowerSnafu)?;
        }

        successful += 1;
    }

    info!("Started {successful} SMP cores!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use mockall::predicate::{eq, function};

    use crate::memory::PageSize;

    use super::*;

    /// This test tree blob was generated using QEMU:
    ///
    /// ```bash
    /// $ qemu-system-aarch64 -machine virt,dumpdtb=kernel_core/src/platform/device_tree/test-tree-smp8.fdt -smp 8
    /// ```
    ///
    /// This blob should be identical to the [`TEST_TREE_BLOB`], but have 8 cores.
    const TEST_TREE_BLOB_SMP8: &[u8] = include_bytes!("./device_tree/test-tree-smp8.fdt");

    fn test_tree_smp8() -> DeviceTree<'static> {
        DeviceTree::from_bytes(TEST_TREE_BLOB_SMP8)
    }

    #[test]
    fn boot_cores() {
        env_logger::init();

        let dt = test_tree_smp8();
        let mut pa = crate::memory::MockPageAllocator::new();
        pa.expect_page_size().return_const(PageSize::FourKiB);
        pa.expect_allocate()
            .times(7)
            .with(eq(1024))
            .returning(|_| Ok(PhysicalAddress::from(0xee_0000)));

        let epa: usize = 0xbeef_feed;
        let mut pm = MockPowerManager::new();
        let cx = MockPowerManager::enable_method_name_context();
        cx.expect().return_const(b"psci\0" as &'static [u8]);
        for i in 1..8 {
            pm.expect_start_core()
                .once()
                .with(
                    eq(i),
                    function(move |x: &PhysicalAddress| usize::from(*x) == epa),
                    eq(0xffff_0000_00ee_0000usize + 4 * 1024 * 1024),
                )
                .returning(|_, _, _| Ok(()));
        }

        boot_all_cores(&dt, &pm, epa.into(), &pa).expect("boot all cores");
    }
}
