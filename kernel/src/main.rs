#![no_std]
#![no_main]

core::arch::global_asm!(include_str!("./start.S"));

#[no_mangle]
pub extern "C" fn kmain(_device_tree_blob: *mut ()) -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}

#[panic_handler]
pub fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    #[allow(clippy::empty_loop)]
    loop {}
}