//! Device Tree blob.
//!
//! Parser/search routines for the
//! [DeviceTree Specification](https://github.com/devicetree-org/devicetree-specification)
//! to obtain hardware/boot parameters.
//!
//! This is designed to require no allocation/copying so that it can be used as soon as possible
//! during the boot process.
//! Individual device drivers are expected to make sense of the exact structure of the information
//! in their respective portion of the tree, but this module contains common structures and
//! iterators to make that easier.
//!
use core::{ffi::CStr, fmt::Debug};

use byteorder::{BigEndian, ByteOrder};
use iter::NodePropertyIter;
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
    Strings(StringList<'dt>),
    /// A blob of bytes. This means the property has a device specific format.
    Bytes(&'dt [u8]),
}

impl<'dt> Value<'dt> {
    /// Parse bytes into a value based on the name and expected type.
    fn parse(name: &[u8], bytes: &'dt [u8]) -> Value<'dt> {
        // See Devicetree Specification section 2.3
        match name {
            b"compatible" => Value::Strings(StringList { data: bytes }),
            b"model" | b"status" => match CStr::from_bytes_until_nul(bytes) {
                Ok(s) => Value::String(s),
                Err(_) => Value::Bytes(bytes),
            },
            b"phandle" => Value::Phandle(BigEndian::read_u32(bytes)),
            b"#address-cells" | b"#size-cells" | b"virtual-reg" => {
                Value::U32(BigEndian::read_u32(bytes))
            }
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
        if let Self::Strings(v) = self {
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
    /// Create a [`DeviceTree`] struct that represents a device tree blob resident in memory.
    ///
    /// # Safety
    /// It is up to the caller to make sure that `ptr` actually points to a valid, mapped device
    /// tree blob, and that it will live for the `'a` lifetime at this address.
    pub unsafe fn from_memory<'a>(ptr: *mut u8) -> DeviceTree<'a> {
        use core::slice;
        // discover the actual size of the tree from the header
        let header = fdt::BlobHeader {
            buf: slice::from_raw_parts(ptr, core::mem::size_of::<u32>() * 2),
        };
        let buf = slice::from_raw_parts(ptr, header.total_size() as usize);
        Self::from_bytes(buf)
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
        assert_eq!(
            header.magic(),
            fdt::EXPECTED_MAGIC,
            "buffer header has incorrect magic value"
        );
        assert_eq!(
            header.total_size() as usize,
            buf.len(),
            "buffer incorrect size according to header"
        );
        // log::debug!("device tree at {:x}, header={:?}", addr as usize, header);
        DeviceTree {
            header,
            strings: &buf[header.off_dt_strings() as usize..header.size_dt_strings() as usize],
            structure: &buf[header.off_dt_struct() as usize..header.size_dt_structs() as usize],
            mem_map: &buf[header.off_mem_rsvmap() as usize..header.size_dt_structs() as usize],
        }
    }

    /// Get the header for the blob.
    fn header(&self) -> fdt::BlobHeader {
        self.header
    }

    /// Returns the total size of the blob in bytes.
    #[must_use]
    pub fn size_of_blob(&self) -> usize {
        self.header().total_size() as usize
    }

    /// Iterate over the tree structure.
    #[must_use]
    pub fn iter_structure(&self) -> iter::FlattenedTreeIter {
        iter::FlattenedTreeIter {
            current_offset: 0,
            dt: self,
        }
    }

    /// Iterate over the properties of a node in the tree given the path.
    #[must_use]
    pub fn iter_node_properties(&self, path: &[u8]) -> Option<iter::NodePropertyIter> {
        if path.is_empty() || path[0] != b'/' {
            return None;
        }
        let mut index = 1;
        let mut cur = self.iter_structure();
        while let Some(item) = cur.next() {
            match item {
                fdt::Token::StartNode(name) => {
                    if path[index..].starts_with(name)
                        && path.len() < index + name.len()
                        && path[index + name.len()] == b'/'
                    {
                        // enter the node and move to finding the next component in the path
                        index += name.len() + 1;

                        if index >= path.len() {
                            return Some(NodePropertyIter { cur, depth: 1 });
                        }
                    } else {
                        // skip this node and all of its children
                        let mut depth = 1;
                        for item in cur.by_ref() {
                            match item {
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
                fdt::Token::EndNode => {
                    index = path[1..index]
                        .iter()
                        .rev()
                        .find_position(|b| **b == b'/')?
                        .0;
                }
                fdt::Token::Property { .. } => {}
            }
        }
        None
    }

    /// Find a property in the tree by path, if it is present.
    #[must_use]
    pub fn find_property(&self, path: &[u8]) -> Option<Value> {
        let split = path.iter().rev().find_position(|p| **p == b'/')?.0;
        let (node_path, property_name) = path.split_at(split);

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
