//! Rust heap allocator [`GlobalAlloc`] implementation.

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use spin::once::Once;

use super::PageAllocator;

#[repr(C)]
struct FreeHeader {
    size: usize,
    next: AtomicPtr<FreeHeader>,
}

#[repr(C)]
struct AllocatedHeader {
    size: usize,
    block: NonNull<u8>,
}

/// A heap allocator for arbitrary sized allocations that is usable as a Rust heap ([`GlobalAlloc`]).
///
/// The allocator uses a basic free list algorithm.
#[allow(clippy::module_name_repetitions)]
pub struct HeapAllocator<'pa, PA> {
    page_allocator: Once<&'pa PA>,
    free_list: AtomicPtr<FreeHeader>,
}

impl<'pa, PA: PageAllocator> HeapAllocator<'pa, PA> {
    /// Create a new allocator that creates a heap in pages allocated by `page_allocator`.
    pub fn new(page_allocator: &'pa PA) -> Self {
        Self {
            page_allocator: Once::initialized(page_allocator),
            free_list: AtomicPtr::default(),
        }
    }

    /// Create a new allocator that has not yet been assigned to a page allocator.
    /// Call [`Self::init`] to finish initialization.
    ///
    /// If used before initialization, the allocator will just return `null`.
    #[must_use]
    pub const fn new_uninit() -> Self {
        Self {
            page_allocator: Once::new(),
            free_list: AtomicPtr::new(null_mut()),
        }
    }

    /// Finish initializing an allocator constructed with [`Self::new_uninit`] by providing a page allocator.
    pub fn init(&self, page_allocator: &'pa PA) {
        self.page_allocator.call_once(|| page_allocator);
    }

    unsafe fn try_remove_fit(&self, desired_size: usize) -> Option<NonNull<FreeHeader>> {
        // keep trying until successful or not found
        'retry: loop {
            let mut prev_ptr: Option<NonNull<FreeHeader>> = None;
            let mut current_ptr = NonNull::new(self.free_list.load(Ordering::Acquire));

            // step through the list
            while let Some(current) = current_ptr {
                let next_ptr = current.as_ref().next.load(Ordering::Relaxed);

                let current_h = current.as_ref();
                if current_h.size >= desired_size {
                    // Attempt to remove the block from the free list.
                    let success = if let Some(prev_ptr) = prev_ptr {
                        unsafe {
                            prev_ptr
                                .as_ref()
                                .next
                                .compare_exchange(
                                    current.as_ptr(),
                                    next_ptr,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                )
                                .is_ok()
                        }
                    } else {
                        // Removing from head
                        self.free_list
                            .compare_exchange(
                                current.as_ptr(),
                                next_ptr,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_ok()
                    };

                    if success {
                        return Some(current);
                    }

                    // Failed to remove; retry from the beginning of the list.
                    continue 'retry;
                }

                prev_ptr = current_ptr;
                current_ptr = NonNull::new(next_ptr);
            }

            return None;
        }
    }

    unsafe fn push_free_block(&self, mut block: NonNull<FreeHeader>) {
        let mut head = self.free_list.load(Ordering::Acquire);
        loop {
            block.as_mut().next.store(head, Ordering::Relaxed);

            match self.free_list.compare_exchange_weak(
                head,
                block.as_ptr(),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(h) => head = h,
            }
        }
    }

    fn is_free(&self, ptr: NonNull<FreeHeader>) -> bool {
        let mut cursor = self.free_list.load(Ordering::Acquire);
        unsafe {
            while let Some(block) = cursor.as_ref() {
                if ptr.as_ptr() >= cursor && ptr.as_ptr() < cursor.byte_add(block.size) {
                    #[cfg(test)]
                    println!("{ptr:x?} in {cursor:x?}+{}", block.size);
                    return true;
                }
                cursor = block.next.load(Ordering::Acquire);
            }
            false
        }
    }
}

fn align_up(offset: usize, align: usize) -> usize {
    (offset + align - 1) & !(align - 1)
}

fn header_padding_max(layout: Layout) -> usize {
    let alloc_header_layout = Layout::new::<AllocatedHeader>();
    if layout.align() <= alloc_header_layout.align() {
        0
    } else if layout.align() == alloc_header_layout.size() {
        alloc_header_layout.align()
    } else {
        layout.align() - alloc_header_layout.size()
    }
}

/// Smallest number of pages that will be added to the heap when it runs out of free blocks.
/// Large allocations may request more pages than this.
const MIN_PAGE_ALLOCATION: usize = 4;

unsafe impl<'pa, PA: PageAllocator> GlobalAlloc for HeapAllocator<'pa, PA> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let alloc_header_layout = Layout::new::<AllocatedHeader>();
        let between_header_and_data_padding_max = header_padding_max(layout);
        let required_block_size =
            alloc_header_layout.size() + between_header_and_data_padding_max + layout.size();

        let block = self.try_remove_fit(required_block_size);

        let (total_block_size, block) = if let Some(block) = block {
            let block_size = block.as_ref().size;
            assert!(block_size >= required_block_size);

            (block_size, block)
        } else {
            let Some(pa) = self.page_allocator.poll() else {
                return null_mut();
            };
            let page_count = required_block_size
                .div_ceil(pa.page_size().into())
                .max(MIN_PAGE_ALLOCATION);
            assert!(
                layout.align() <= usize::from(pa.page_size()),
                "layout alignments greater than a page are unsupported, layout={layout:?}"
            );
            if let Ok(pages) = pa.allocate(page_count) {
                (
                    page_count * pa.page_size(),
                    NonNull::new(pages.cast().into()).unwrap(),
                )
            } else {
                return null_mut();
            }
        };

        let padding_required = block
            .cast::<u8>()
            .add(alloc_header_layout.size())
            .align_offset(layout.align());
        assert!(padding_required <= between_header_and_data_padding_max, "padding required {padding_required} <= max padding {between_header_and_data_padding_max}");

        let actual_block_size = alloc_header_layout.size() + padding_required + layout.size();

        // #[cfg(test)]
        // println!("alloc {total_block_size} ({actual_block_size} / {required_block_size}) {block:x?} padding_max={between_header_and_data_padding_max} padding_req={padding_required}");

        if total_block_size.saturating_sub(actual_block_size) > size_of::<FreeHeader>() {
            // put back the rest
            let rest_offset = align_up(actual_block_size, align_of::<FreeHeader>());
            let mut rest_block = block.byte_add(rest_offset);
            *rest_block.as_mut() = FreeHeader {
                size: total_block_size - rest_offset,
                next: AtomicPtr::default(),
            };
            self.push_free_block(rest_block);
        }

        // place the padding first, then the header right before the data so we can find it in `dealloc`.
        let mut header: NonNull<AllocatedHeader> = block.byte_add(padding_required).cast();
        *header.as_mut() = AllocatedHeader {
            size: actual_block_size,
            block: block.cast(),
        };
        header.add(1).cast().as_ptr()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(ptr) = NonNull::new(ptr) {
            let alloc_header_layout = Layout::new::<AllocatedHeader>();

            let header: NonNull<AllocatedHeader> = ptr.cast().offset(-1);
            let block_claimed_size = header.as_ref().size;
            let min_size = alloc_header_layout.size() + layout.size();
            let max_size = header_padding_max(layout) + min_size;
            assert!(min_size <= block_claimed_size && block_claimed_size <= max_size,
                "min_size<=block_claimed_size<=max_size! block_claimed_size={block_claimed_size}, min_size={min_size}, max_size={max_size}, layout={layout:?}");
            let mut free_block: NonNull<FreeHeader> = header.as_ref().block.cast();

            assert!(!self.is_free(free_block), "double free detected");

            *free_block.as_mut() = FreeHeader {
                size: block_claimed_size,
                next: AtomicPtr::default(),
            };
            self.push_free_block(free_block);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::alloc::Layout;
    use std::{thread, vec::Vec};

    use test_case::test_matrix;

    use crate::memory::tests::MockPageAllocator;

    use super::*;

    fn create_page_allocator() -> MockPageAllocator {
        MockPageAllocator::new(crate::memory::PageSize::FourKiB, 512)
    }

    fn allocate_batch<A: GlobalAlloc>(allocator: &A, layout: Layout, size: usize) -> Vec<*mut u8> {
        (0..size)
            .map(|_| unsafe {
                let p = allocator.alloc(layout);
                assert!(!p.is_null());
                assert!(
                    p.is_aligned_to(layout.align()),
                    "{p:x?} not aligned to {}",
                    layout.align()
                );
                p
            })
            .collect()
    }

    fn free_batch(
        allocator: &HeapAllocator<'_, MockPageAllocator>,
        layout: Layout,
        batch: Vec<*mut u8>,
    ) {
        for ptr in batch {
            unsafe { allocator.dealloc(ptr, layout) }
        }
    }

    fn free_batch_rev(
        allocator: &HeapAllocator<'_, MockPageAllocator>,
        layout: Layout,
        batch: Vec<*mut u8>,
    ) {
        for ptr in batch.into_iter().rev() {
            unsafe { allocator.dealloc(ptr, layout) }
        }
    }

    fn free_batch_interleave(
        allocator: &HeapAllocator<'_, MockPageAllocator>,
        layout: Layout,
        batch: Vec<*mut u8>,
    ) {
        for ptr in batch.iter().skip(1).step_by(2) {
            unsafe { allocator.dealloc(*ptr, layout) }
        }
        for ptr in batch.iter().step_by(2).rev() {
            unsafe { allocator.dealloc(*ptr, layout) }
        }
    }

    #[test_matrix(
        [free_batch, free_batch_rev, free_batch_interleave],
        [8, 27, 64, 67, 1111, 4096],
        [1, 2, 4, 8, 16, 128, 256, 1024],
        [128]
    )]
    fn seq_batch(
        free_fn: fn(
            allocator: &HeapAllocator<'_, MockPageAllocator>,
            layout: Layout,
            batch: Vec<*mut u8>,
        ),
        size: usize,
        alignment: usize,
        batch_size: usize,
    ) {
        let pa = create_page_allocator();
        let a = HeapAllocator::new(&pa);
        let layout = Layout::from_size_align(size, alignment).expect("create layout");
        let batch = allocate_batch(&a, layout, batch_size);
        free_fn(&a, layout, batch);
    }

    #[test_matrix(
        [3, 7],
        [free_batch, free_batch_rev, free_batch_interleave],
        [27,100],
        [8],
        [128]
    )]
    fn concurrent_batch(
        thread_count: usize,
        free_fn: fn(
            allocator: &HeapAllocator<'_, MockPageAllocator>,
            layout: Layout,
            batch: Vec<*mut u8>,
        ),
        size: usize,
        alignment: usize,
        batch_size: usize,
    ) {
        let pa = create_page_allocator();
        let a = HeapAllocator::new(&pa);
        let layout = Layout::from_size_align(size, alignment).expect("create layout");
        thread::scope(|s| {
            for _ in 0..thread_count {
                s.spawn(|| {
                    let batch = allocate_batch(&a, layout, batch_size);
                    free_fn(&a, layout, batch);
                });
            }
        });
    }

    #[test_matrix(
        [free_batch, free_batch_rev, free_batch_interleave],
        [8, 27, 64],
        [1, 8, 256],
        [128],
        [64]
    )]
    fn seq_batch_with_interlude(
        free_fn: fn(
            allocator: &HeapAllocator<'_, MockPageAllocator>,
            layout: Layout,
            batch: Vec<*mut u8>,
        ),
        size: usize,
        alignment: usize,
        batch_size: usize,
        interlude_batch_size: usize,
    ) {
        let pa = create_page_allocator();
        let a = HeapAllocator::new(&pa);
        let layout = Layout::from_size_align(size, alignment).expect("create layout");
        let batch_first_half = allocate_batch(&a, layout, batch_size / 2);
        let interlude_batch = allocate_batch(&a, layout, interlude_batch_size);
        free_fn(&a, layout, interlude_batch);
        let batch_second_half = allocate_batch(&a, layout, batch_size / 2);
        free_fn(&a, layout, batch_first_half);
        free_fn(&a, layout, batch_second_half);
    }

    #[test]
    #[should_panic]
    fn double_free() {
        let pa = create_page_allocator();
        let a = HeapAllocator::new(&pa);
        let layout = Layout::from_size_align(1024, 8).unwrap();
        unsafe {
            let p = a.alloc(layout);
            a.dealloc(p, layout);
            a.dealloc(p, layout);
        }
    }

    #[test]
    #[should_panic]
    fn double_free_tricky() {
        let pa = create_page_allocator();
        let a = HeapAllocator::new(&pa);
        let layout = Layout::from_size_align(1024, 8).unwrap();
        unsafe {
            let p = a.alloc(layout);
            let p2 = a.alloc(layout);
            a.dealloc(p, layout);
            a.dealloc(p2, layout);
            a.dealloc(p, layout);
        }
    }

    #[test]
    fn impossibly_large_allocation() {
        let pa = create_page_allocator();
        let a = HeapAllocator::new(&pa);
        let layout = Layout::from_size_align(isize::MAX as usize, 1).unwrap();
        unsafe {
            let p = a.alloc(layout);
            assert!(p.is_null());
        }
    }
}
