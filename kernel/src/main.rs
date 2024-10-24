//! The Endless Hole microkernel. See `spec/kernel.md` for the specification.
//!
//! This binary is the actual kernel, containing the entry point and implementing the mechanisms
//! necessary for executing the policies defined in [`kernel_core`].
#![no_std]
#![no_main]
#![deny(missing_docs)]

extern crate alloc;

core::arch::global_asm!(core::include_str!("./start.S"));

mod exception;
mod memory;
mod running_image;
mod uart;

use core::fmt::Write;

use kernel_core::{
    memory::{PageAllocator, PhysicalPointer},
    platform::device_tree::{DeviceTree, Value as DTValue},
};

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
        exception::install_exception_vector();
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

    memory::init(&device_tree, &mut uart);

    writeln!(
        &mut uart,
        "page size = {:?}",
        memory::page_allocator().page_size()
    )
    .unwrap();

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
