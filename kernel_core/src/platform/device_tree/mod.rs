//! Devicetree blob parser.
//!
//! Parser/search routines for the
//! [Devicetree Specification](https://github.com/devicetree-org/devicetree-specification)
//! to obtain hardware/boot parameters.
//!
//! This is designed to require no allocation/copying so that it can be used as soon as possible
//! during the boot process.
//! Individual device drivers are expected to make sense of the exact structure of the information
//! in their respective portion of the tree, but this module contains common structures and
//! iterators to make that easier.
use core::{ffi::CStr, fmt::Debug};

use byteorder::{BigEndian, ByteOrder};
use itertools::Itertools;

pub mod fdt;
pub mod iter;

/// A list of strings given as the value of a property.
#[derive(Clone)]
pub struct StringList<'dt> {
    /// The raw bytes of the list of strings.
    pub data: &'dt [u8],
}

impl<'dt> StringList<'dt> {
    /// Determine if the byte sequence `s` is in the list of strings.
    #[must_use]
    pub fn contains(&self, s: &[u8]) -> bool {
        self.data.windows(s.len()).any(|w| w == s)
    }

    /// Iterate over the strings present in the list as C strings.
    #[must_use]
    pub fn iter(&self) -> iter::StringListIter {
        iter::StringListIter {
            data: self.data,
            current_offset: 0,
        }
    }
}

impl<'a: 'dt, 'dt> IntoIterator for &'a StringList<'dt> {
    type IntoIter = iter::StringListIter<'dt>;
    type Item = &'dt core::ffi::CStr;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'dt> core::fmt::Debug for StringList<'dt> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// An array of (address, length) pairs representing the address space regions of a device's resources.
#[derive(Clone)]
pub struct Registers<'dt> {
    /// Raw bytes that make up the array.
    pub data: &'dt [u8],
    /// Number of u32 cells that make up an address.
    pub address_cells: u32,
    /// Number of u32 cells that make up a size.
    pub size_cells: u32,
}

impl<'dt> Registers<'dt> {
    /// Iterate over the (address, length) pairs contained in this array.
    ///
    /// These are yielded as `usize`.
    #[must_use]
    pub fn iter(&self) -> iter::RegistersIter {
        iter::RegistersIter {
            regs: self,
            offset: 0,
        }
    }
}

impl<'a: 'dt, 'dt> IntoIterator for &'a Registers<'dt> {
    type IntoIter = iter::RegistersIter<'a, 'dt>;
    type Item = (usize, usize);
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'dt> core::fmt::Debug for Registers<'dt> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// A property value in a device tree.
#[derive(Debug)]
pub enum Value<'dt> {
    /// A 32-bit integer.
    U32(u32),
    #[allow(unused)]
    /// A 64-bit integer.
    U64(u64),
    /// A `phandle` value that references another node.
    Phandle(u32),
    /// A single printable string.
    String(&'dt CStr),
    /// A list of strings.
    StringList(StringList<'dt>),
    /// A blob of bytes. This means the property has a device specific format.
    Bytes(&'dt [u8]),
    /// An array of (address, length) pairs representing the address space regions of a device's resources.
    Reg(Registers<'dt>),
}

impl<'dt> Value<'dt> {
    /// Parse bytes into a value based on the name and expected type.
    fn parse(name: &[u8], bytes: &'dt [u8], address_cells: u32, size_cells: u32) -> Value<'dt> {
        // See Devicetree Specification section 2.3
        match name {
            b"compatible" => Value::StringList(StringList { data: bytes }),
            b"model" | b"status" => match CStr::from_bytes_until_nul(bytes) {
                Ok(s) => Value::String(s),
                Err(_) => Value::Bytes(bytes),
            },
            b"phandle" => Value::Phandle(BigEndian::read_u32(bytes)),
            b"#address-cells" | b"#size-cells" | b"virtual-reg" => {
                Value::U32(BigEndian::read_u32(bytes))
            }
            b"reg" => Value::Reg(Registers {
                data: bytes,
                address_cells,
                size_cells,
            }),
            _ => Value::Bytes(bytes),
        }
    }

    /// If the value is of type `u32`, extract the value as a Rust `u32`.
    #[must_use]
    pub fn into_u32(self) -> Option<u32> {
        if let Self::U32(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// If the value is of type `u64`, extract the value as a Rust `u64`.
    #[must_use]
    pub fn into_u64(self) -> Option<u64> {
        if let Self::U64(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// If the value is of type `phandle`, extract the value as a Rust `u32`.
    #[must_use]
    pub fn into_phandle(self) -> Option<u32> {
        if let Self::Phandle(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// If the value is of type `string`, extract the value as a Rust [`CStr`].
    #[must_use]
    pub fn into_string(self) -> Option<&'dt CStr> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// If the value is of type `stringlist`, extract the value as [`StringList`].
    #[must_use]
    pub fn into_strings(self) -> Option<StringList<'dt>> {
        if let Self::StringList(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// If the value is unparsed, extract the value as a Rust `&[u8]`, returning the raw bytes.
    #[must_use]
    pub fn into_bytes(self) -> Option<&'dt [u8]> {
        if let Self::Bytes(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

/// A device tree blob in memory.
pub struct DeviceTree<'a> {
    header: fdt::BlobHeader<'a>,
    strings: &'a [u8],
    structure: &'a [u8],
    mem_map: &'a [u8],
}

impl DeviceTree<'_> {
    /// Create a [`DeviceTree`] struct that represents a device tree blob resident in memory of an
    /// unknown size, like one provided by U-boot.
    ///
    /// # Panics
    ///
    /// This function panics if the header present in `buf` is incorrect, in particular if:
    /// - the magic value is incorrect.
    /// - the total length reported in the header does not match the length of the slice.
    ///
    /// # Safety
    /// It is up to the caller to make sure that `ptr` actually points to a valid, mapped device
    /// tree blob, and that it will live for the `'a` lifetime at this address.
    ///
    pub unsafe fn from_memory<'a>(ptr: *mut u8) -> DeviceTree<'a> {
        use core::slice;
        // discover the actual size of the tree from the header
        let header = fdt::BlobHeader {
            buf: slice::from_raw_parts(ptr, fdt::HEADER_SIZE),
        };
        let buf = slice::from_raw_parts(ptr, header.total_size() as usize);
        Self::from_bytes_and_header(buf, header)
    }

    /// Create a [`DeviceTree`] that parses the blob present in `buf`.
    ///
    ///
    /// # Panics
    ///
    /// This function panics if the header present in `buf` is incorrect, in particular if:
    /// - the magic value is incorrect.
    /// - the total length reported in the header does not match the length of the slice.
    #[must_use]
    pub fn from_bytes(buf: &[u8]) -> DeviceTree {
        let header = fdt::BlobHeader { buf };
        Self::from_bytes_and_header(buf, header)
    }

    fn from_bytes_and_header<'a>(buf: &'a [u8], header: fdt::BlobHeader<'a>) -> DeviceTree<'a> {
        assert_eq!(
            header.magic(),
            fdt::HEADER_EXPECTED_MAGIC,
            "buffer header has incorrect magic value"
        );
        assert_eq!(
            header.total_size() as usize,
            buf.len(),
            "buffer incorrect size according to header"
        );

        let str_start = header.off_dt_strings() as usize;
        let str_end = str_start + header.size_dt_strings() as usize;
        let structs_start = header.off_dt_struct() as usize;
        let structs_end = structs_start + header.size_dt_structs() as usize;

        DeviceTree {
            header,
            strings: &buf[str_start..str_end],
            structure: &buf[structs_start..structs_end],
            mem_map: &buf[header.off_mem_rsvmap() as usize..header.off_dt_struct() as usize],
        }
    }

    /// Get the header for the blob.
    #[must_use]
    pub fn header(&self) -> fdt::BlobHeader {
        self.header
    }

    /// Iterate over the raw flattened tree blob structure.
    #[must_use]
    pub fn iter_structure(&self) -> iter::FlattenedTreeIter {
        iter::FlattenedTreeIter {
            current_offset: 0,
            dt: self,
        }
    }

    /// Iterate over the properties of a node in the tree given the path, if present.
    #[must_use]
    pub fn iter_node_properties(&self, path: &[u8]) -> Option<iter::NodePropertyIter> {
        let mut segments = path.split(|p| *p == b'/');
        let mut looking_for = segments.next()?;
        let mut tokens = self.iter_structure();
        let mut current_address_cells: Option<u32> = None;
        let mut current_size_cells: Option<u32> = None;
        while let Some(token) = tokens.next() {
            match token {
                fdt::Token::StartNode(name) => {
                    if name == looking_for {
                        // enter the node
                        match segments.next() {
                            None | Some(&[]) => {
                                return Some(iter::NodePropertyIter {
                                    cur: tokens,
                                    depth: 1,
                                    // defaults from the spec in section 2.3.5
                                    parent_address_cells: current_address_cells.unwrap_or(2),
                                    parent_size_cells: current_size_cells.unwrap_or(1),
                                });
                            }
                            Some(next_segment) => {
                                looking_for = next_segment;
                            }
                        }
                    } else {
                        // skip this node and all of its children
                        let mut depth = 1;
                        for token in tokens.by_ref() {
                            match token {
                                fdt::Token::EndNode => depth -= 1,
                                fdt::Token::StartNode(_) => depth += 1,
                                fdt::Token::Property { .. } => {}
                            }
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                }
                // because we skip everything that is not on the path, these will only be
                // encountered in parents of the node we are searching for
                fdt::Token::EndNode => return None,
                fdt::Token::Property { name, data } => match name {
                    b"#address-cells" => current_address_cells = Some(BigEndian::read_u32(data)),
                    b"#size-cells" => current_size_cells = Some(BigEndian::read_u32(data)),
                    _ => {}
                },
            }
        }
        None
    }

    /// Find a property in the tree by path, if it is present.
    #[must_use]
    pub fn find_property(&self, path: &[u8]) -> Option<Value> {
        let split = path.iter().rev().find_position(|p| **p == b'/')?.0;
        let (node_path, property_name) = path.split_at(path.len() - split);

        self.iter_node_properties(node_path)?
            .find(|(name, _)| *name == property_name)
            .map(|(_, value)| value)
    }

    /// Iterate over the system reserved memory regions.
    #[must_use]
    pub fn iter_reserved_memory_regions(&self) -> iter::MemRegionIter {
        iter::MemRegionIter::for_data(self.mem_map)
    }

    // Write the device tree to the system log at DEBUG level.
    // pub fn log(&self) {
    //     log::debug!("Device tree:");
    //     for item in self.iter_structure() {
    //         log::debug!("{item:x?}");
    //     }
    //     log::debug!("-----------");
    // }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This test tree blob was generated using QEMU:
    /// ```bash
    /// $ qemu-system-aarch64 -machine virt,dumpdtb=kernel_core/src/platform/device_tree/test-tree.fdt
    /// ```
    const TEST_TREE_BLOB: &[u8] = include_bytes!("test-tree.fdt");

    fn test_tree() -> DeviceTree<'static> {
        DeviceTree::from_bytes(TEST_TREE_BLOB)
    }

    #[test]
    fn find_property_at_root() {
        let tree = test_tree();
        match tree.find_property(b"/compatible") {
            Some(Value::StringList(ss)) => {
                assert!(ss.contains(b"linux,dummy-virt"));
            }
            c => panic!("unexpected value for /compatible: {:?}", c),
        }
    }

    #[test]
    fn find_property_in_child_of_root() {
        let tree = test_tree();
        match tree.find_property(b"/timer/compatible") {
            Some(Value::StringList(ss)) => {
                assert!(ss.contains(b"arm,armv7-timer"));
            }
            c => panic!("unexpected value for /compatible: {:?}", c),
        }
    }

    #[test]
    fn find_property_in_nested_child() {
        let tree = test_tree();
        match tree.find_property(b"/intc@8000000/v2m@8020000/phandle") {
            Some(Value::Phandle(v)) => {
                assert_eq!(v, 0x8003);
            }
            c => panic!(
                "unexpected value for /intc@8000000/v2m@8020000/phandle: {:?}",
                c
            ),
        }
    }

    #[test]
    fn cannot_find_nonexistent_property() {
        let tree = test_tree();
        let r = tree.find_property(b"/cpus/this/property/does/not/exist");
        assert!(r.is_none());
    }

    #[test]
    fn node_properties_exact() {
        let tree = test_tree();
        let mut properties = std::collections::HashSet::from([
            "clock-names",
            "clocks",
            "interrupts",
            "reg",
            "compatible",
        ]);

        for (name, _) in tree.iter_node_properties(b"/pl011@9000000").unwrap() {
            let name_str = core::str::from_utf8(name).unwrap();
            assert!(properties.remove(name_str));
        }

        assert!(
            properties.is_empty(),
            "did not find all properties: {properties:?}"
        );
    }

    #[test]
    fn reg_property() {
        let tree = test_tree();
        match tree.find_property(b"/intc@8000000/reg") {
            Some(Value::Reg(regs)) => {
                let mut i = regs.iter();
                assert_eq!(i.next(), Some((0x0800_0000, 0x1_0000)));
                assert_eq!(i.next(), Some((0x0801_0000, 0x1_0000)));
                assert_eq!(i.next(), None);
            }
            c => panic!("unexpected value for /intc@8000000/reg: {:?}", c),
        }
    }

    #[test]
    fn reg_property_nested() {
        let tree = test_tree();
        match tree.find_property(b"/cpus/cpu@0/reg") {
            Some(Value::Reg(regs)) => {
                assert_eq!(regs.address_cells, 1);
                assert_eq!(regs.size_cells, 0);
                let mut i = regs.iter();
                assert_eq!(i.next(), Some((0, 0)));
                assert_eq!(i.next(), None);
            }
            c => panic!("unexpected value for /cpus/cpu@0/reg: {:?}", c),
        }
    }
}
