//! The Endless Hole microkernel. See `spec/kernel.md` for the specification.
#![no_std]
#![no_main]
#![deny(missing_docs)]

core::arch::global_asm!(include_str!("./start.S"));

/// The main entry point for the kernel.
///
/// This function is called by `start.S` after it sets up virtual memory, the stack, etc.
/// The device tree blob is provided by U-Boot, see `u-boot/arch/arm/lib/bootm.c:boot_jump_linux(...)`.
#[no_mangle]
pub extern "C" fn kmain(_device_tree_blob: *mut ()) -> ! {
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
