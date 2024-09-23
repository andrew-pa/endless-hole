//! The Endless Hole microkernel. See `spec/kernel.md` for the specification.
//!
//! This binary is the actual kernel, containing the entry point and implementing the mechanisms
//! necessary for executing the policies defined in [`kernel_core`].
#![no_std]
#![no_main]
#![deny(missing_docs)]

extern crate alloc;

core::arch::global_asm!(core::include_str!("./start.S"));

mod running_image;
mod uart;

use core::fmt::Write;

use itertools::Itertools;
use kernel_core::{
    memory::{BuddyPageAllocator, HeapAllocator, PhysicalPointer},
    platform::device_tree::{DeviceTree, Value as DTValue},
};

use spin::once::Once;

/// The global physical page allocator.
static PAGE_ALLOCATOR: Once<BuddyPageAllocator> = Once::new();

#[global_allocator]
/// The Rust global heap allocator.
static ALLOCATOR: HeapAllocator<'static, BuddyPageAllocator> = HeapAllocator::new_uninit();

/// Initialize the memory subsystem.
///
/// The memory subsystem consists of:
/// - the global physical page allocator
/// - the MMU and the kernel page tables
/// - the Rust heap
fn init_memory(dt: &DeviceTree<'_>, uart: &mut impl Write) {
    // create page allocator
    let page_size = 0x1000;
    let memory_node = dt
        .iter_nodes_named(b"/", b"memory")
        .expect("root")
        .exactly_one()
        .expect("device tree has memory node");
    let memory_range = memory_node
        .properties
        .clone()
        .find(|(name, _)| name == b"reg")
        .and_then(|(_, v)| v.into_reg())
        .expect("memory node has reg property")
        .iter()
        .exactly_one()
        .expect("memory has exactly one reg range");
    let reserved_regions = [
        unsafe { running_image::memory_region() },
        dt.memory_region(),
    ];
    writeln!(
        uart,
        "memory range = {memory_range:x?}, reserved = {reserved_regions:x?}"
    )
    .unwrap();
    let pa = PAGE_ALLOCATOR.call_once(|| unsafe {
        BuddyPageAllocator::new(
            page_size,
            PhysicalPointer::from(memory_range.0).into(),
            memory_range.1,
            reserved_regions.into_iter(),
        )
    });

    // setup page tables

    // initialize kernel heap
    ALLOCATOR.init(pa);

    writeln!(uart, "memory initialized").unwrap();
}

/// The main entry point for the kernel.
///
/// This function is called by `start.S` after it sets up virtual memory, the stack, etc.
/// The device tree blob is provided by U-Boot, see `u-boot/arch/arm/lib/bootm.c:boot_jump_linux(...)`.
///
/// # Panics
///
/// If something goes wrong during the boot process that is unrecoverable, a panic will occur.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn kmain(device_tree_blob: PhysicalPointer<u8>) -> ! {
    unsafe {
        running_image::zero_bss_section();
    }

    let device_tree = unsafe { DeviceTree::from_memory(device_tree_blob.into()) };

    let stdout_device_path = device_tree
        .find_property(b"/chosen/stdout-path")
        .and_then(DTValue::into_bytes)
        // the string is null terminated in the device tree
        // TODO: default to QEMU virt board UART for now, should be platform default
        .map_or(b"/pl011@9000000" as &[u8], |p| &p[0..p.len() - 1]);

    let mut uart =
        uart::PL011::from_device_tree(&device_tree, stdout_device_path).expect("init UART");

    writeln!(
        &mut uart,
        "Hello, world! Using `{}` for debug logs.",
        core::str::from_utf8(stdout_device_path).unwrap()
    )
    .unwrap();

    if let Some(board_model) = device_tree
        .find_property(b"/model")
        .and_then(DTValue::into_string)
    {
        writeln!(&mut uart, "Board model: {board_model:?}").unwrap();
    }

    writeln!(&mut uart, "kernel memory region {:x?}", unsafe {
        running_image::memory_region()
    })
    .unwrap();

    init_memory(&device_tree, &mut uart);

    #[allow(clippy::empty_loop)]
    loop {}
}

/// The main entry point for secondary cores in an SMP system.
///
/// This function is called by `start.S` after it sets up virtual memory, the stack, etc.
#[no_mangle]
pub extern "C" fn secondary_core_kmain() -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}

/// The kernel-wide panic handler.
///
/// Code here should not assume anything about the state of the kernel.
/// Currently this only writes to the platform defined debug UART.
#[panic_handler]
pub fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    unsafe {
        let mut uart = uart::PL011::from_platform_debug_best_guess();

        writeln!(&mut uart, "\x1b[31mpanic!\x1b[0m {info}").unwrap();
    }

    #[allow(clippy::empty_loop)]
    loop {}
}
