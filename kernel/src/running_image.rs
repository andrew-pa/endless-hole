//! Definitions for the current running kernel image.
#![allow(clippy::cast_sign_loss)]
use core::ptr::{addr_of, addr_of_mut, write_bytes};

/// These are defined by the linker script so we know where the sections are.
/// Notably not the value but the *address* of these symbols is what is relevant.
mod markers {
    extern "C" {
        /// Beginning of the `bss` section.
        pub static mut __bss_start: u8;
        /// End of the `bss` section.
        pub static mut __bss_end: u8;
        /// Beginning of the entire kernel image.
        pub static mut __kernel_start: u8;
        /// End of the entire kernel image.
        pub static mut __kernel_end: u8;
    }
}

/// Zero the BSS section of the kernel as is expected by the ELF
///
/// # Safety
/// This function should only be called exactly *once* at the beginning of boot.
/// Also, it *must* be called, or else global constants will have undefined values instead of zero.
pub unsafe fn zero_bss_section() {
    let bss_start = addr_of_mut!(markers::__bss_start);
    let bss_end = addr_of_mut!(markers::__bss_end);
    let bss_size = bss_end.offset_from(bss_start) as usize;
    write_bytes(bss_start, 0, bss_size);
}

/// Find the region of memory that contains the kernel image.
///
/// # Safety
/// The validity of the returned region depends entirely on the correctness of the linker, linker
/// script and loader to make sure the marker symbols are defined in the correct places.
pub unsafe fn kernel_memory_region() -> (*const u8, usize) {
    let start = addr_of!(markers::__kernel_start);
    let end = addr_of!(markers::__kernel_end);
    (start, end.offset_from(start) as usize)
}
