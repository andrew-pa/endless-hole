# Developing in Cavern

## Tools
- Compilers (Rust, C) for host and `aarch64-linux-gnu-` targets
- QEMU (`qemu-system-aarch64`)
- [Just task runner](https://just.systems/)

## Running Tasks with Just
The `justfile` defines several recipes to help you build and test the system. Here's a brief overview of each recipe:
* `build`: Builds the Rust crates in the project.
* `fmt`: Runs code formatter.
* `check`: Checks formatting, types, and lints for the Rust code.
* `make-kernel-image`: Creates a U-Boot image for the kernel.
* `run-qemu`: Runs the system in QEMU for testing.
* `test`: Runs all of the unit tests.

See the `just` documentation for more info about running tasks.
