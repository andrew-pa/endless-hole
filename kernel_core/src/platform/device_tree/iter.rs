//! Device Tree iterators.
#![allow(clippy::module_name_repetitions)]
use core::ffi::CStr;

use byteorder::{BigEndian, ByteOrder as _};

use super::{fdt, Value};

fn pad_end_4b(num_bytes: usize) -> usize {
    num_bytes
        + if num_bytes % 4 == 0 {
            0
        } else {
            4 - (num_bytes % 4)
        }
}

/// Iterator over tree tokens in a device tree blob.
pub struct FlattenedTreeIter<'dt> {
    pub(super) dt: &'dt super::DeviceTree<'dt>,
    pub(super) current_offset: usize,
}

impl<'dt> Iterator for FlattenedTreeIter<'dt> {
    type Item = super::fdt::Token<'dt>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.current_offset += 4;
            match fdt::TokenType::from(BigEndian::read_u32(
                &self.dt.structure[(self.current_offset - 4)..],
            )) {
                fdt::TokenType::BeginNode => {
                    let mut name_end = self.current_offset;
                    while self.dt.structure.get(name_end).map_or(false, |b| *b != 0) {
                        name_end += 1;
                    }
                    let name = &self.dt.structure[self.current_offset..name_end];
                    self.current_offset = pad_end_4b(name_end + 1);
                    return Some(fdt::Token::StartNode(name));
                }
                fdt::TokenType::EndNode => return Some(fdt::Token::EndNode),
                fdt::TokenType::Prop => {
                    let length =
                        BigEndian::read_u32(&self.dt.structure[self.current_offset..]) as usize;
                    self.current_offset += 4;
                    let name_offset =
                        BigEndian::read_u32(&self.dt.structure[self.current_offset..]) as usize;
                    self.current_offset += 4;
                    let mut name_end = name_offset;
                    while self.dt.strings.get(name_end).map_or(false, |b| *b != 0) {
                        name_end += 1;
                    }
                    let name = &self.dt.strings[name_offset..name_end];
                    let data =
                        &self.dt.structure[self.current_offset..(self.current_offset + length)];
                    self.current_offset += pad_end_4b(length);
                    return Some(fdt::Token::Property { name, data });
                }
                fdt::TokenType::Nop => continue,
                fdt::TokenType::End => return None,
                fdt::TokenType::Unknown(x) => panic!("unknown device tree token: {x}"),
            }
        }
    }
}

/// An iterator over reserved regions of memory.
pub struct MemRegionIter<'dt> {
    data: &'dt [u8],
    current_offset: usize,
}

impl<'dt> MemRegionIter<'dt> {
    /// Creates a memory region iterator for the data of an arbitrary property.
    #[must_use]
    pub fn for_data(data: &'dt [u8]) -> Self {
        Self {
            data,
            current_offset: 0,
        }
    }
}

impl<'dt> Iterator for MemRegionIter<'dt> {
    type Item = (u64, u64);

    fn next(&mut self) -> Option<Self::Item> {
        let addr = BigEndian::read_u64(&self.data[self.current_offset..]);
        self.current_offset += 8;
        let size = BigEndian::read_u64(&self.data[self.current_offset..]);
        self.current_offset += 8;
        if addr == 0 && size == 0 {
            None
        } else {
            Some((addr, size))
        }
    }
}

/// Iterator over strings in a [`StringList`].
pub struct StringListIter<'dt> {
    pub(super) data: &'dt [u8],
    pub(super) current_offset: usize,
}

impl<'dt> Iterator for StringListIter<'dt> {
    type Item = &'dt CStr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_offset >= self.data.len() {
            None
        } else {
            match CStr::from_bytes_until_nul(&self.data[self.current_offset..]) {
                Ok(s) => {
                    self.current_offset += s.to_bytes_with_nul().len();
                    Some(s)
                }
                Err(_) => None,
            }
        }
    }
}

/// Iterator over properties of a single node in the tree.
pub struct NodePropertyIter<'a> {
    pub(super) cur: FlattenedTreeIter<'a>,
    pub(super) depth: usize,
}

impl<'a> Iterator for NodePropertyIter<'a> {
    type Item = (&'a [u8], Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.depth == 0 {
            return None;
        }
        for item in self.cur.by_ref() {
            match item {
                fdt::Token::StartNode(_) => {
                    // Increment depth if we enter another node
                    self.depth += 1;
                }
                fdt::Token::EndNode => {
                    // Decrement depth and stop if we've left the target node
                    self.depth -= 1;
                    if self.depth == 0 {
                        return None; // We're done once we exit the target node
                    }
                }
                fdt::Token::Property { name, data } => {
                    if self.depth == 1 {
                        // We're in the target node; yield the property
                        return Some((name, Value::parse(name, data)));
                    }
                }
            }
        }
        None
    }
}
