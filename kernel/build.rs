//! Build script for the kernel executable.
//!
//! Responsible for setting the linker script.

use vergen::{BuildBuilder, Emitter};
use vergen_git2::Git2Builder;

fn emit_info() {
    let build = BuildBuilder::default()
        .build_timestamp(true)
        .build()
        .unwrap();
    let git = Git2Builder::default()
        .sha(true)
        .branch(true)
        .build()
        .unwrap();
    Emitter::default()
        .add_instructions(&build)
        .unwrap()
        .add_instructions(&git)
        .unwrap()
        .emit()
        .unwrap()
}

fn main() {
    println!("cargo:rustc-link-arg=-T./kernel/link.ld");

    emit_info();
}
