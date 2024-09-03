//! The Endless Hole microkernel. See `spec/kernel.md` for the specification.
#![no_std]
#![no_main]
#![deny(missing_docs)]

core::arch::global_asm!(include_str!("./platform/start.S"));

mod platform;

use platform::device_tree::DeviceTree;

/// The main entry point for the kernel.
///
/// This function is called by `start.S` after it sets up virtual memory, the stack, etc.
/// The device tree blob is provided by U-Boot, see `u-boot/arch/arm/lib/bootm.c:boot_jump_linux(...)`.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn kmain(device_tree_blob: *mut u8) -> ! {
    unsafe {
        platform::bss::zero_bss_section();
    }

    let device_tree = unsafe { DeviceTree::from_memory(device_tree_blob) };

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
