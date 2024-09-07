//! PL011 UART driver.
//!
//! Documentation for the interface can be found [on ARM's website](https://developer.arm.com/documentation/ddi0183/latest/).

use core::fmt::Write;

use kernel_core::platform::{
    device_tree::{DeviceTree, Value},
    PhysicalPointer,
};

/// The PL011 UART object.
pub struct PL011 {
    base_address: *mut u8,
}

impl PL011 {
    /// Configure the driver using information from a device tree node.
    /// The node must follow the spec at [].
    pub fn from_device_tree(dt: &DeviceTree, path: &[u8]) -> Option<Self> {
        let mut base_address = None;
        for (name, value) in dt.iter_node_properties(path)? {
            if let (b"reg", Value::Reg(r)) = (name, value) {
                base_address = r.iter().next().map(|(r, _)| PhysicalPointer::from(r));
            }
        }
        base_address.map(|r| PL011 {
            base_address: r.into(),
        })
    }

    /// Create a UART for debugging purposes that will probably work but requires no configuration.
    pub fn from_platform_debug_best_guess() -> Self {
        PL011 {
            base_address: 0xffff_0000_0900_0000 as *mut u8,
        }
    }
}

impl Write for PL011 {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for ch in s.bytes() {
            unsafe {
                self.base_address.write_volatile(ch);
            }
        }
        Ok(())
    }
}
