//! CPU management

use alloc::borrow::ToOwned;
use core::ffi::CStr;
use log::{debug, info};

use snafu::{ensure, OptionExt, ResultExt, Snafu};

use crate::{
    memory::{PageAllocator, PhysicalAddress, PhysicalPointer},
    platform::device_tree::{
        DeviceTree, NodeNotFoundSnafu, OwnedParseError, ParseError, PropertyNotFoundSnafu,
        UnexpectedValueSnafu,
    },
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
pub trait PowerManager {
    /// Powers on a core that is currently off.
    /// The core will start executing at `entry_point_address`, with `arg` passed as the argument.
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
    fn enable_method_name() -> &'static CStr;
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

/// Power on all cores as described by the device tree.
pub fn boot_all_cores<'dt, PM: PowerManager>(
    device_tree: &'dt DeviceTree,
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
                            .as_string(name)
                            .map_err(|s| BootAllCoresError::DeviceTree {
                                cause: s.to_owned(),
                            })?;

                    ensure!(
                        enable_method == PM::enable_method_name(),
                        UnsupportedEnableMethodSnafu {
                            method: enable_method.to_string_lossy().into_owned()
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

        // allocate a new 4MiB stack for the core kernel thread.
        let stack_size = 4 * 1024 * 1024;
        let stack = page_allocator
            .allocate(stack_size / page_allocator.page_size())
            .context(MemorySnafu)?;

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
    use super::*;

    /// This test tree blob was generated using QEMU:
    ///
    /// ```bash
    /// $ qemu-system-aarch64 -machine virt,dumpdtb=kernel_core/src/platform/device_tree/test-tree.fdt
    /// ```
    const TEST_TREE_BLOB: &[u8] = include_bytes!("./device_tree/test-tree.fdt");

    /// This test tree blob was generated using QEMU:
    ///
    /// ```bash
    /// $ qemu-system-aarch64 -machine virt,dumpdtb=kernel_core/src/platform/device_tree/test-tree-smp8.fdt -smp 8
    /// ```
    ///
    /// This blob should be identical to the [`TEST_TREE_BLOB`], but have 8 cores.
    const TEST_TREE_BLOB_SMP8: &[u8] = include_bytes!("./device_tree/test-tree-smp8.fdt");

    fn test_tree() -> DeviceTree<'static> {
        DeviceTree::from_bytes(TEST_TREE_BLOB)
    }

    fn test_tree_smp8() -> DeviceTree<'static> {
        DeviceTree::from_bytes(TEST_TREE_BLOB)
    }

    #[test]
    fn boot_cores() {
        let dt = test_tree_smp8();
        let pa = crate::memory::tests::MockPageAllocator::new(
            crate::memory::PageSize::FourKiB,
            8 * 1024,
        );
        boot_all_cores(&dt, &pm, PhysicalPointer::from(0xbeef_feed), &pa).expect("boot all cores");
    }
}
