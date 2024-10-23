//! Memory subsystem.
//!
//! The memory subsystem consists of:
//! - the global physical page allocator
//! - the MMU and the kernel page tables
//! - the Rust heap
use crate::running_image;
use core::{fmt::Write, ptr::addr_of_mut};
use itertools::Itertools as _;
use kernel_core::{
    memory::{
        page_table::{MapBlockSize, MemoryProperties},
        BuddyPageAllocator, HeapAllocator, PageAllocator, PageSize, PageTables, PhysicalAddress,
        PhysicalPointer, VirtualPointer, VirtualPointerMut,
    },
    platform::device_tree::DeviceTree,
};
use spin::{once::Once, Mutex};

extern "C" {
    // Root of the kernel page table (defined in `start.S`).
    static mut _kernel_page_table_root: u8;
}

type ChosenPageAllocator = BuddyPageAllocator;

/// The global physical page allocator.
static PAGE_ALLOCATOR: Once<ChosenPageAllocator> = Once::new();

#[global_allocator]
/// The Rust global heap allocator.
static ALLOCATOR: HeapAllocator<'static, ChosenPageAllocator> = HeapAllocator::new_uninit();

static KERNEL_PAGE_TABLES: Once<Mutex<PageTables<'static, ChosenPageAllocator>>> = Once::new();

/// Flush the TLB for everything in EL1.
///
/// # Safety
/// It is up to the caller to make sure that the flush makes sense in context.
pub unsafe fn flush_tlb_total_el1() {
    core::arch::asm!(
        "DSB ISHST",    // ensure writes to tables have completed
        "TLBI VMALLE1", // flush entire TLB. The programming guide uses the 'ALLE1'
        // variant, which causes a fault in QEMU with EC=0, but
        // https://forum.osdev.org/viewtopic.php?t=36412&p=303237
        // suggests using VMALLE1 instead, which appears to work
        "DSB ISH", // ensure that flush has completed
        "ISB",     // make sure next instruction is fetched with changes
    )
}

/// Initialize the memory subsystem.
pub fn init(dt: &DeviceTree<'_>, uart: &mut impl Write) {
    // create page allocator
    let page_size = PageSize::FourKiB;
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
    let memory_start = PhysicalAddress::from(memory_range.0);
    let mut memory_regions = kernel_core::memory::subtract_ranges(
        (memory_start.cast().into(), memory_range.1),
        reserved_regions.into_iter(),
    );
    writeln!(
        uart,
        "memory range = {memory_start:?}{memory_range:x?}, reserved = {reserved_regions:x?}"
    )
    .unwrap();

    let pa = PAGE_ALLOCATOR.call_once(|| unsafe {
        BuddyPageAllocator::new(page_size, memory_start.cast().into(), memory_range.1)
    });

    let first_region = memory_regions.next().expect("at least one memory region");
    writeln!(
        uart,
        "adding first memory region to physical page allocator ({:x?}, {:x})",
        first_region.0, first_region.1
    )
    .unwrap();
    unsafe {
        assert!(pa.add_memory_region(first_region.0, first_region.1));
    }

    // setup page tables
    KERNEL_PAGE_TABLES.call_once(|| unsafe {
        let root_table_address = addr_of_mut!(_kernel_page_table_root);
        let mut pt =
            //PageTables::from_existing(pa, PhysicalAddress::from(root_table_address.cast()), true);
            PageTables::empty(pa).unwrap();
        let block_size = MapBlockSize::largest_supported_block_size(pa.page_size());
        let memory_size_in_blocks = memory_range
            .1
            .div_ceil(block_size.length_in_pages(pa.page_size()).unwrap() * pa.page_size());
        writeln!(
            uart,
            "mapping RAM {memory_start:?}, {memory_size_in_blocks} {block_size:?}"
        )
        .unwrap();
        pt.map(
            memory_start.into(),
            memory_start,
            memory_size_in_blocks,
            block_size,
            &MemoryProperties {
                writable: true,
                executable: true,
                ..MemoryProperties::default()
            },
        )
        .expect("identity map RAM into kernel");
        writeln!(uart, "new page tables = {:?}", pt).unwrap();
        let vaddr =
            VirtualPointerMut::from(0xffff000107efe000 as *mut () /* uart as *mut _ */).cast();
        let paddr = pt.physical_address_of(vaddr);
        writeln!(uart, "uart addr {vaddr:?} maps to {paddr:?}").unwrap();
        Mutex::new(pt)
    });

    unsafe {
        flush_tlb_total_el1();
    }

    for (region_start, region_length) in memory_regions {
        writeln!(
            uart,
            "adding additional memory region to physical page allocator ({:x?}, {:x})",
            region_start, region_length
        )
        .unwrap();
        unsafe {
            assert!(pa.add_memory_region(region_start, region_length));
        }
    }

    writeln!(uart, "boo").unwrap();
    // initialize kernel heap
    ALLOCATOR.init(pa);

    writeln!(uart, "memory initialized").unwrap();
}

/// Returns a reference to the current global physical page allocator.
pub fn page_allocator() -> &'static impl PageAllocator {
    PAGE_ALLOCATOR.wait()
}
