//! Memory subsystem.
//!
//! The memory subsystem consists of:
//! - the global physical page allocator
//! - the MMU and the kernel page tables
//! - the Rust heap
use crate::running_image;
use core::fmt::Write;
use itertools::Itertools as _;
use kernel_core::{
    memory::{BuddyPageAllocator, HeapAllocator, PageAllocator, PhysicalPointer},
    platform::device_tree::DeviceTree,
};
use spin::once::Once;

/// The global physical page allocator.
static PAGE_ALLOCATOR: Once<BuddyPageAllocator> = Once::new();

#[global_allocator]
/// The Rust global heap allocator.
static ALLOCATOR: HeapAllocator<'static, BuddyPageAllocator> = HeapAllocator::new_uninit();

/// Initialize the memory subsystem.
pub fn init(dt: &DeviceTree<'_>, uart: &mut impl Write) {
    // create page allocator
    let page_size = 0x1000;
    let memory_node = dt
        .iter_nodes_named(b"/", b"memory")
        .expect("root")
        .exactly_one()
        .expect("device tree has memory node");
    let memory_range = memory_node
        .properties
        .clone()
        .find(|(name, _)| name == b"reg")
        .and_then(|(_, v)| v.into_reg())
        .expect("memory node has reg property")
        .iter()
        .exactly_one()
        .expect("memory has exactly one reg range");
    let reserved_regions = [
        unsafe { running_image::memory_region() },
        dt.memory_region(),
    ];
    writeln!(
        uart,
        "memory range = {memory_range:x?}, reserved = {reserved_regions:x?}"
    )
    .unwrap();
    let pa = PAGE_ALLOCATOR.call_once(|| unsafe {
        BuddyPageAllocator::new(
            page_size,
            PhysicalPointer::from(memory_range.0).into(),
            memory_range.1,
            reserved_regions.into_iter(),
        )
    });

    // setup page tables

    // initialize kernel heap
    ALLOCATOR.init(pa);

    writeln!(uart, "memory initialized").unwrap();
}

/// Returns a reference to the current global physical page allocator.
pub fn page_allocator() -> &'static impl PageAllocator {
    PAGE_ALLOCATOR.wait()
}
