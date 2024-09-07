// these are defined by the linker script so we know where the BSS section is
// notably not the value but the *address* of these symbols is what is relevant
extern "C" {
    static mut __bss_start: u8;
    static mut __bss_end: u8;
    static mut __kernel_start: u8;
    static mut __kernel_end: u8;
}

/// Zero the BSS section of the kernel as is expected by the ELF
///
/// # Safety
/// This function should only be called *once* at the beginning of boot.
/// Also, it *must* be called, or else global constants will have undefined values.
pub unsafe fn zero_bss_section() {
    use core::ptr::{addr_of_mut, write_bytes};

    let bss_start = addr_of_mut!(__bss_start);
    let bss_end = addr_of_mut!(__bss_end);
    let bss_size = bss_end.offset_from(bss_start) as usize;
    write_bytes(bss_start, 0, bss_size);
}
