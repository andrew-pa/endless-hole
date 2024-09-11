//! Build script for the kernel executable.
//!
//! Responsible for setting the linker script.

fn main() {
    println!("cargo:rustc-link-arg=-T./kernel/link.ld");
}
