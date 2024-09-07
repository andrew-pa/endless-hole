//! The Endless Hole microkernel. See `spec/kernel.md` for the specification.
#![no_std]
#![deny(missing_docs)]

#[cfg(all(test, not(target_os = "none")))]
#[macro_use]
extern crate std;

pub mod platform;

#[cfg(test)]
mod tests {
    use std::prelude::rust_2021::*;

    #[test]
    fn it_works() {
        println!("hello");
        assert_eq!(2 + 2, 4);
    }
}
