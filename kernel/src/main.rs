//! The Endless Hole microkernel. See `spec/kernel.md` for the specification.
//!
//! This binary is the actual kernel, containing the entry point.
#![no_std]
#![no_main]
#![deny(missing_docs)]

core::arch::global_asm!(core::include_str!("./start.S"));

mod bss;

use kernel_core::platform::device_tree::{DeviceTree, Value as DTValue};

/// The main entry point for the kernel.
///
/// This function is called by `start.S` after it sets up virtual memory, the stack, etc.
/// The device tree blob is provided by U-Boot, see `u-boot/arch/arm/lib/bootm.c:boot_jump_linux(...)`.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn kmain(device_tree_blob: *mut u8) -> ! {
    unsafe {
        bss::zero_bss_section();
    }

    let device_tree = unsafe { DeviceTree::from_memory(device_tree_blob) };

    let stdout_device_name = device_tree
        .find_property(b"/chosen/stdout-path")
        .and_then(DTValue::into_bytes)
        // TODO: default to QEMU virt board UART for now, should be platform default
        .unwrap_or(b"/pl011@9000000");

    // TODO: for now we assume that this device is a UART
    let stdout_uart_register_props = device_tree
        .iter_node_properties(stdout_device_name)
        .expect("debug stdout device tree node")
        .find_map(|p| (p.0 == b"reg").then_some(p.1))
        .expect("debug UART device has `reg` property");

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
#[panic_handler]
pub fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}
