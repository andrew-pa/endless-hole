//! Memory subsystem.
//!
//! The memory subsystem consists of:
//! - the global physical page allocator
//! - the MMU and the kernel page tables
//! - the Rust heap
use crate::running_image;
use bitfield::bitfield;
use core::{arch::asm, ptr::addr_of_mut};
use itertools::Itertools as _;
use kernel_core::{
    memory::{
        page_table::{MapBlockSize, MemoryKind, MemoryProperties},
        AddressSpaceId, BuddyPageAllocator, HeapAllocator, PageAllocator, PageSize, PageTables,
        PhysicalAddress,
    },
    platform::device_tree::DeviceTree,
};
use log::{debug, info, trace};
use spin::{once::Once, Mutex};

extern "C" {
    // Root of the kernel page table (defined in `start.S`).
    static mut _kernel_page_table_root: u8;
}

pub type PlatformPageAllocator = BuddyPageAllocator;

/// The global physical page allocator.
static PAGE_ALLOCATOR: Once<PlatformPageAllocator> = Once::new();

#[global_allocator]
/// The Rust global heap allocator.
static ALLOCATOR: HeapAllocator<'static, PlatformPageAllocator> = HeapAllocator::new_uninit();

/// The kernel's own page tables.
///
/// Map addresses in TTBR1, matching `0xffff_????_????_????`.
static KERNEL_PAGE_TABLES: Once<Mutex<PageTables<'static>>> = Once::new();

/// Flush the TLB for everything in EL1.
///
/// # Safety
/// It is up to the caller to make sure that the flush makes sense in context.
unsafe fn flush_tlb_total_el1() {
    asm!(
        "DSB ISHST",    // ensure writes to tables have completed
        "TLBI VMALLE1", // flush entire TLB. The programming guide uses the 'ALLE1'
        // variant, which causes a fault in QEMU with EC=0, but
        // https://forum.osdev.org/viewtopic.php?t=36412&p=303237
        // suggests using VMALLE1 instead, which appears to work
        "DSB ISH", // ensure that flush has completed
        "ISB",     // make sure next instruction is fetched with changes
    );
}

/// Flush the TLB for a specific out-going ASID.
///
/// # Safety
/// It is up to the caller to make sure that the flush makes sense in context.
unsafe fn flush_tlb_for_asid(asid: u16) {
    core::arch::asm!(
        "DSB ISHST", // ensure writes to tables have completed
        "TLBI ASIDE1, {asid:x}", // flush TLB entries associated with ASID
        "DSB ISH", // ensure that flush has completed
        "ISB", // make sure next instruction is fetched with changes
        asid = in(reg) asid
    );
}

/// Write the `MAIR_EL1` register.
unsafe fn write_mair(value: u64) {
    asm!(
        "msr MAIR_EL1, {val}",
        val = in(reg) value
    );
}

bitfield! {
    /// A value for a TTBR*_EL* register, holding the base address for the current page translation table.
    pub struct TranslationTableBaseRegister(u64);
    impl Debug;
    impl new;
    u16, asid, set_asid: 63, 48;
    u64, baddr, set_baddr: 47, 1;
    cnp, set_cnp: 0;
}

impl TranslationTableBaseRegister {
    /// Read the value of TTBR0_EL1 (D19.2.152).
    unsafe fn read_ttbr0_el1() -> Self {
        let mut value: u64;
        asm!("mrs {v}, TTBR0_EL1", v = out(reg) value);
        TranslationTableBaseRegister(value)
    }

    unsafe fn write_ttbr0_el1(&self) {
        asm!("msr TTBR0_EL1, {v}", v = in(reg) self.0);
    }
}

/// Switch to a new set of page tables in EL0.
/// If `full_flush` is true, then all EL0 TLB entries will be flushed, otherwise only entries for
/// the previous address space ID will be flushed.
pub unsafe fn switch_el0_context(
    new_page_tables: &PageTables,
    new_address_space_id: AddressSpaceId,
    full_flush: bool,
) {
    assert!(
        !new_page_tables.high_tag(),
        "page tables must map EL0 (0x0000_*) addresses!"
    );
    // compute new TTBR value
    let new_ttbr = TranslationTableBaseRegister::new(
        new_address_space_id.into(),
        usize::from(new_page_tables.physical_address()) as u64,
        false,
    );
    // read TTBR0
    let current_ttbr = TranslationTableBaseRegister::read_ttbr0_el1();
    // if TTBR0 == new TTBR value, then do nothing
    if new_ttbr.0 != current_ttbr.0 || full_flush {
        // write TTBR0
        new_ttbr.write_ttbr0_el1();
        if full_flush {
            flush_tlb_total_el1();
        } else {
            // flush cache for old ASID
            flush_tlb_for_asid(current_ttbr.asid());
        }
    }
}

/// Initialize the memory subsystem.
pub fn init(dt: &DeviceTree<'_>) {
    debug!("Initializing memory…");
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
    trace!(
        "memory range = {memory_start:?}{memory_range:x?}, reserved = {reserved_regions:x?}, page size = {page_size:?}"
    );

    let pa = PAGE_ALLOCATOR.call_once(|| unsafe {
        BuddyPageAllocator::new(page_size, memory_start.cast().into(), memory_range.1)
    });

    let first_region = memory_regions.next().expect("at least one memory region");
    trace!(
        "adding first memory region to physical page allocator ({:x?}, {:x})",
        first_region.0,
        first_region.1
    );
    unsafe {
        assert!(pa.add_memory_region(first_region.0, first_region.1));
    }

    // setup page tables
    KERNEL_PAGE_TABLES.call_once(|| unsafe {
        let root_table_address = addr_of_mut!(_kernel_page_table_root);
        let mut pt =
            PageTables::from_existing(pa, PhysicalAddress::from(root_table_address.cast()), true);
        let block_size = MapBlockSize::largest_supported_block_size(pa.page_size());
        let block_size_in_bytes = block_size.length_in_bytes(pa.page_size()).unwrap();
        let memory_size_in_blocks = memory_range.1.div_ceil(block_size_in_bytes);
        trace!("mapping RAM {memory_start:?}, {memory_size_in_blocks} {block_size:?}");
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

        trace!("mapping low addresses as MMIO");
        pt.map(
            0xffff_0000_0000_0000.into(),
            0.into(),
            usize::from(memory_start) / block_size_in_bytes,
            block_size,
            &MemoryProperties {
                writable: true,
                kind: MemoryKind::Device,
                ..MemoryProperties::default()
            },
        )
        .expect("identity map low address as MMIO");

        trace!("new kernel page table {pt:?}");

        Mutex::new(pt)
    });

    unsafe {
        // TODO: mess with TCR to make sure that page sizes and address sizes are as expected.
        // install MAIR value that corresponds to the [`MemoryKind`] enum encoding.
        write_mair(kernel_core::memory::page_table::MAIR_VALUE);

        // Flush the TLB to ensure that the new page table mapping takes effect.
        flush_tlb_total_el1();
    }

    for (region_start, region_length) in memory_regions {
        trace!(
            "adding additional memory region to physical page allocator ({region_start:x?}, {region_length:x})",
        );
        unsafe {
            assert!(pa.add_memory_region(region_start, region_length));
        }
    }

    // initialize kernel heap
    ALLOCATOR.init(pa);

    info!("Memory initialized!");
}

/// Returns a reference to the current global physical page allocator.
#[allow(unused)]
pub fn page_allocator() -> &'static impl PageAllocator {
    PAGE_ALLOCATOR.wait()
}
