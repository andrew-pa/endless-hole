target_prefix := "aarch64-linux-gnu-"
build_profile := "debug"

out_dir := env("EH_OUTPUT_DIR", absolute_path("./.build"))
img_dir := out_dir / "image"
vendor_tool_dir := out_dir / "vendor"

host_target_triple := `rustc --version --verbose | grep "host" | awk '{print $2}'`

# Choose a task to run.
default:
    @just --choose

# Delete generated outputs.
clean:
    rm -rf {{out_dir}}

# Check formatting, types and lints.
check cargo_args="" clippy_args="":
    cargo fmt --check {{cargo_args}}
    cargo check --all-features {{cargo_args}}
    cargo clippy --all-features {{cargo_args}} -- -Dmissing_docs -Dclippy::all -Wclippy::pedantic {{clippy_args}}

# Test Rust crates that are testable on the host.
test cargo_args="":
    cargo test -p kernel_core --target {{host_target_triple}} {{cargo_args}}

# Build Rust crates.
build cargo_args="":
    cargo build {{ if build_profile == "release" { "--release" } else { "" } }} --target aarch64-unknown-none {{cargo_args}}

mkimage_bin := vendor_tool_dir / "u-boot/tools/mkimage"

binary_path := "target/aarch64-unknown-none" / build_profile
kernel_load_addr := "41000000"

# Create U-boot image for the kernel.
make-kernel-image kernel_elf_path=(binary_path / "kernel") mkimage_args="": build
    #!/bin/bash
    set -euxo pipefail
    mkdir -p {{img_dir}}
    if [ "{{img_dir / "kernel.img"}}" -nt "{{kernel_elf_path}}" ]; then
        echo "kernel image already up-to-date"
        exit 0
    fi
    flat_binary_path=$(mktemp -t kernel.XXXXXX.img)
    {{target_prefix}}objcopy -O binary {{kernel_elf_path}} $flat_binary_path
    {{mkimage_bin}} -A arm64 -O linux -T kernel -C none -a {{kernel_load_addr}} -e {{kernel_load_addr}} -n "endless-hole-kernel" -d $flat_binary_path {{mkimage_args}} {{img_dir / "kernel.img"}}
    rm $flat_binary_path

# Run the system in QEMU.
run-qemu qemu_args="" boot_args="{}": make-kernel-image
    #!/bin/sh
    set -euxo pipefail
    qemu-system-aarch64 \
        -machine virt -cpu cortex-a57 \
        -semihosting \
        -bios {{vendor_tool_dir / "u-boot/u-boot.bin"}} \
        -nographic \
        -drive if=none,file=fat:rw:{{img_dir}},id=kboot,format=raw \
        -device nvme,drive=kboot,serial=foo {{qemu_args}} \
    <<-END
        nvme scan
        fatload nvme 0 0x41000000 kernel.img
        env set bootargs '{{boot_args}}'
        bootm 41000000 - 40000000
    END

make_bin := `which make`

# Build U-Boot image and tools.
build_u-boot:
    mkdir -p {{vendor_tool_dir / "u-boot"}}
    CROSS_COMPILE={{target_prefix}} {{make_bin}} -C ./vendor/u-boot O={{vendor_tool_dir / "u-boot"}} qemu_arm64_defconfig
    CROSS_COMPILE={{target_prefix}} {{make_bin}} -C ./vendor/u-boot O={{vendor_tool_dir / "u-boot"}} -j all
