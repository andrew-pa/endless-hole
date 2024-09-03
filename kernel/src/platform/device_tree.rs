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

/// The magic value expected in the device tree header.
const EXPECTED_MAGIC: u32 = 0xd00d_feed;

/// Values used to delimit structure in the flattened device tree.
#[repr(u32)]
enum FdtToken {
    BeginNode = 0x01,
    EndNode = 0x02,
    Prop = 0x03,
    Nop = 0x04,
    End = 0x09,
    Unknown(u32),
}

impl From<u32> for FdtToken {
    fn from(value: u32) -> Self {
        match value {
            0x01 => FdtToken::BeginNode,
            0x02 => FdtToken::EndNode,
            0x03 => FdtToken::Prop,
            0x04 => FdtToken::Nop,
            0x09 => FdtToken::End,
            _ => FdtToken::Unknown(value),
        }
    }
}

/// Device tree blob header.
#[derive(Copy, Clone)]
struct BlobHeader<'a> {
    buf: &'a [u8],
}

impl<'a> BlobHeader<'a> {
    /// Magic number. Should equal [`EXPECTED_MAGIC`].
    pub fn magic(&self) -> u32 {
        BigEndian::read_u32(&self.buf[0..])
    }
    /// Total size of the blob.
    pub fn total_size(&self) -> u32 {
        BigEndian::read_u32(&self.buf[4..])
    }
    /// Offset to the structs region of the blob.
    pub fn off_dt_struct(&self) -> u32 {
        BigEndian::read_u32(&self.buf[8..])
    }
    /// Offset to the strings region of the blob.
    pub fn off_dt_strings(&self) -> u32 {
        BigEndian::read_u32(&self.buf[12..])
    }
    /// Offset to the memory reservation block.
    pub fn off_mem_rsvmap(&self) -> u32 {
        BigEndian::read_u32(&self.buf[16..])
    }
    /// Blob version code.
    pub fn version(&self) -> u32 {
        BigEndian::read_u32(&self.buf[20..])
    }
    /// Last compatible version this device tree is compatible with.
    pub fn last_comp_version(&self) -> u32 {
        BigEndian::read_u32(&self.buf[24..])
    }
    /// Physical ID of the boot CPU.
    pub fn boot_cpuid_phys(&self) -> u32 {
        BigEndian::read_u32(&self.buf[28..])
    }
    /// Size of the strings region of the blob.
    pub fn size_dt_strings(&self) -> u32 {
        BigEndian::read_u32(&self.buf[32..])
    }
    /// Size of the structs region of the blob.
    pub fn size_dt_structs(&self) -> u32 {
        BigEndian::read_u32(&self.buf[36..])
    }
}

impl<'a> Debug for BlobHeader<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobHeader")
            .field("magic", &self.magic())
            .field("total_size", &self.total_size())
            .field("off_dt_struct", &self.off_dt_struct())
            .field("off_dt_strings", &self.off_dt_strings())
            .field("off_mem_rsvmap", &self.off_mem_rsvmap())
            .field("version", &self.version())
            .field("last_comp_version", &self.last_comp_version())
            .field("boot_cpuid_phys", &self.boot_cpuid_phys())
            .field("size_dt_strings", &self.size_dt_strings())
            .field("size_dt_structs", &self.size_dt_structs())
            .finish()
    }
}

/// A device tree blob in memory.
pub struct DeviceTree<'a> {
    header: BlobHeader<'a>,
    strings: &'a [u8],
    structure: &'a [u8],
    mem_map: &'a [u8],
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
    fn parse(name: &str, bytes: &'dt [u8]) -> Value<'dt> {
        // See Devicetree Specification section 2.3
        match name {
            "compatible" => Value::Strings(StringList {
                data: bytes,
                current_offset: 0,
            }),
            "model" | "status" => match CStr::from_bytes_until_nul(bytes) {
                Ok(s) => Value::String(s),
                Err(_) => Value::Bytes(bytes),
            },
            "phandle" => Value::Phandle(BigEndian::read_u32(bytes)),
            "#address-cells" | "#size-cells" | "virtual-reg" => {
                Value::U32(BigEndian::read_u32(bytes))
            }
            _ => Value::Bytes(bytes),
        }
    }
}

/// A tree structural item.
#[derive(Debug)]
pub enum StructureItem<'dt> {
    /// The beginning of a node in the tree, with a particular name.
    StartNode(&'dt str),
    /// The end of a node in the tree.
    EndNode,
    /// A property attached to some node.
    Property {
        /// The name of the property.
        name: &'dt str,
        /// The value associated with this property.
        data: &'dt [u8],
    },
}

/// A cursor through the device tree.
pub struct Cursor<'dt> {
    dt: &'dt DeviceTree<'dt>,
    current_offset: usize,
}

/// An iterator over reserved regions of memory.
pub struct MemRegionIter<'dt> {
    data: &'dt [u8],
    current_offset: usize,
}

/// A list of strings in the blob.
#[derive(Clone)]
pub struct StringList<'dt> {
    data: &'dt [u8],
    current_offset: usize,
}

// most of the time we really just want to know a single property or iterate over the properties at
// a certain path

impl DeviceTree<'_> {
    /// Create a [`DeviceTree`] struct that represents a device tree blob resident in memory.
    ///
    /// # Safety
    /// It is up to the caller to make sure that `ptr` actually points to a valid, mapped device
    /// tree blob, and that it will live for the `'a` lifetime at this address.
    pub unsafe fn from_memory<'a>(ptr: *mut u8) -> DeviceTree<'a> {
        use core::slice;
        // discover the actual size of the tree from the header
        let header = BlobHeader {
            buf: slice::from_raw_parts(ptr, core::mem::size_of::<u32>() * 2),
        };
        let buf = slice::from_raw_parts(ptr, header.total_size() as usize);
        Self::from_bytes(buf)
    }

    /// Create a [`DeviceTree`] that parses the blob present in `buf`.
    ///
    /// The buffer must be the same length as it claims in the blob header, or this function will panic.
    pub fn from_bytes(buf: &[u8]) -> DeviceTree {
        let header = BlobHeader { buf };
        assert_eq!(
            header.magic(),
            EXPECTED_MAGIC,
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
    fn header(&self) -> BlobHeader {
        self.header
    }

    /// Returns the total size of the blob in bytes.
    pub fn size_of_blob(&self) -> usize {
        self.header().total_size() as usize
    }

    /// Iterate over the tree structure.
    pub fn iter_structure(&self) -> Cursor {
        Cursor {
            current_offset: 0,
            dt: self,
        }
    }

    /// Find a property in the tree by path, if it is present.
    pub fn find_property(&self, path: &str) -> Option<Value> {
        if path.is_empty() || path.starts_with('/') {
            return None;
        }
        let mut index = 1;
        let mut cur = self.iter_structure();
        while let Some(item) = cur.next() {
            if index > path.len() {
                break;
            }

            match item {
                StructureItem::StartNode(name) => {
                    if path[index..].starts_with(name)
                        && path.len() < index + name.len()
                        && path[index + name.len()..].starts_with('/')
                    {
                        // enter the node and move to finding the next component in the path
                        index += name.len() + 1;
                    } else {
                        // skip this node and all of its children
                        let mut depth = 1;
                        for item in cur.by_ref() {
                            match item {
                                StructureItem::EndNode => depth -= 1,
                                StructureItem::StartNode(_) => depth += 1,
                                StructureItem::Property { .. } => {}
                            }
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                }
                StructureItem::EndNode => {
                    index = path[1..index].rfind('/')?;
                }
                StructureItem::Property { name, data } => {
                    if &path[index..] == name {
                        return Some(Value::parse(name, data));
                    }
                }
            }
        }
        None
    }

    /// Iterate over the system reserved memory regions.
    pub fn iter_reserved_memory_regions(&self) -> MemRegionIter {
        MemRegionIter::for_data(self.mem_map)
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

fn pad_end_4b(num_bytes: usize) -> usize {
    num_bytes
        + if num_bytes % 4 == 0 {
            0
        } else {
            4 - (num_bytes % 4)
        }
}

impl<'dt> Iterator for Cursor<'dt> {
    type Item = StructureItem<'dt>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.current_offset += 4;
            match FdtToken::from(BigEndian::read_u32(
                &self.dt.structure[(self.current_offset - 4)..],
            )) {
                FdtToken::BeginNode => {
                    let mut name_end = self.current_offset;
                    while self.dt.structure.get(name_end).map_or(false, |b| *b != 0) {
                        name_end += 1;
                    }
                    let name =
                        core::str::from_utf8(&self.dt.structure[self.current_offset..name_end])
                            .expect("device tree node name is utf8");
                    self.current_offset = pad_end_4b(name_end + 1);
                    return Some(StructureItem::StartNode(name));
                }
                FdtToken::EndNode => return Some(StructureItem::EndNode),
                FdtToken::Prop => {
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
                    let name = core::str::from_utf8(&self.dt.strings[name_offset..name_end])
                        .expect("device tree node name is utf8");
                    let data =
                        &self.dt.structure[self.current_offset..(self.current_offset + length)];
                    self.current_offset += pad_end_4b(length);
                    return Some(StructureItem::Property { name, data });
                }
                FdtToken::Nop => continue,
                FdtToken::End => return None,
                FdtToken::Unknown(x) => panic!("unknown device tree token: {x}"),
            }
        }
    }
}

impl<'dt> MemRegionIter<'dt> {
    /// Creates a memory region iterator for the data of an arbitrary property.
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

impl<'dt> Iterator for StringList<'dt> {
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

impl<'dt> core::fmt::Debug for StringList<'dt> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}
