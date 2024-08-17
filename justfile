# Tasks for vendored dependencies and tools.
mod vendor

target_prefix := "aarch64-linux-gnu-"
build_profile := "debug"

vendor_tool_dir := absolute_path("./vendor/.build/")
out_dir := absolute_path("./.build")

# Build Rust crates.
build cargo_args="":
    cargo build {{ if build_profile == "release" { "--release" } else { "" } }} {{cargo_args}}

mkimage_bin := vendor_tool_dir / "u-boot/tools/mkimage"

# Create U-boot image for the kernel.
default_kernel_elf_path := "target/aarch64-unknown-none" / build_profile / "kernel"
kernel_load_addr := "41000000"
make_kernel_image kernel_elf_path=default_kernel_elf_path mkimage_args="": build
    #!/bin/bash
    set -euxo pipefail
    mkdir -p {{out_dir}}
    flat_binary_path=$(mktemp -t kernel.XXXXXX.img)
    {{target_prefix}}objcopy -O binary {{kernel_elf_path}} $flat_binary_path
    {{mkimage_bin}} -A arm64 -O linux -T kernel -C none -a {{kernel_load_addr}} -e {{kernel_load_addr}} -n "endless-hole-kernel" -d $flat_binary_path {{mkimage_args}} {{out_dir / "kernel.img"}}
    rm $flat_binary_path
