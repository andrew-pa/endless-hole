//! Page table API.

use super::{PhysicalAddress, VirtualAddress};

/// The properties given to a particular page mapping.
#[derive(Clone)]
pub struct PageProperties {}

/// The size of the granule used in a mapping operation.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum MapBlockSize {
    /// Maps by single pages.
    Page,
    /// Maps by small blocks in the Level 2 table. (2MiB with 4KiB pages)
    SmallBlock,
    /// Maps by big blocks in the Level 1 table. (1GiB with 4KiB pages)
    BigBlock,
}

/// Errors that could arise in page table operations.
pub enum Error {}

/// A page table data structure in memory.
pub struct PageTable<'pa, PA> {
    page_allocator: &'pa PA,
}

impl<'pa, PA> PageTable<'pa, PA> {
    /// Map a region of virtual addresses to a region of physical addresses in this page table with
    /// the given `properties`.
    ///
    /// Both start pointers must be aligned to the `size` required alignment.
    /// The region will be of `count * size.byte_length()` bytes.
    pub fn map(
        &self,
        physical_start: PhysicalAddress,
        virtual_start: VirtualAddress,
        count: usize,
        size: MapBlockSize,
        properties: &PageProperties,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Unmap a region of virtual addresses to a region of physical addresses in this page table.
    pub fn unmap(
        &self,
        virtual_start: VirtualAddress,
        count: usize,
        size: MapBlockSize,
    ) -> Result<(), Error> {
        todo!()
    }
}

#[cfg(test)]
mod tests {}
