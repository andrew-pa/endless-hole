//! Rust heap allocator [`GlobalAlloc`] implementation.

use core::{
    alloc::{GlobalAlloc, Layout},
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use super::PageAllocator;

#[repr(C)]
struct FreeHeader {
    size: usize,
    next: AtomicPtr<FreeHeader>,
}

#[repr(C)]
struct AllocatedHeader {
    size: usize,
}

/// A heap allocator for arbitrary sized allocations that is usable as a Rust heap ([`GlobalAlloc`]).
///
/// The allocator uses a basic free list algorithm.
#[allow(clippy::module_name_repetitions)]
pub struct HeapAllocator<'pa, PA> {
    page_allocator: &'pa PA,
    free_list: AtomicPtr<FreeHeader>,
}

impl<'pa, PA: PageAllocator> HeapAllocator<'pa, PA> {
    /// Create a new allocator that creates a heap in pages allocated by `page_allocator`.
    pub fn new(page_allocator: &'pa PA) -> Self {
        Self {
            page_allocator,
            free_list: AtomicPtr::default(),
        }
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
            // Set the next pointer of the last block in the list to the current head.
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
}

fn align_up(offset: usize, align: usize) -> usize {
    (offset + align - 1) & !(align - 1)
}

unsafe impl<'pa, PA: PageAllocator> GlobalAlloc for HeapAllocator<'pa, PA> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let alloc_header_layout = Layout::new::<AllocatedHeader>();
        let between_header_and_data_padding =
            alloc_header_layout.padding_needed_for(layout.align());
        let required_block_size =
            alloc_header_layout.size() + between_header_and_data_padding + layout.size();

        let block = self.try_remove_fit(required_block_size);

        if let Some(block) = block {
            let block_size = block.as_ref().size;
            assert!(block_size >= required_block_size);

            if block_size.saturating_sub(required_block_size) > size_of::<FreeHeader>() {
                // put back the rest
                let rest_offset = align_up(required_block_size, align_of::<FreeHeader>());
                let mut rest_block = block.byte_add(rest_offset);
                *rest_block.as_mut() = FreeHeader {
                    size: block_size - rest_offset,
                    next: AtomicPtr::default(),
                };
                self.push_free_block(rest_block);
            }

            let mut header: NonNull<AllocatedHeader> = block.cast();
            *header.as_mut() = AllocatedHeader {
                size: required_block_size,
            };
            block
                .cast()
                .add(alloc_header_layout.size() + between_header_and_data_padding)
                .as_ptr()
        } else {
            let page_count = required_block_size
                .div_ceil(self.page_allocator.page_size())
                .max(4);
            assert!(
                layout.align() <= self.page_allocator.page_size(),
                "layout alignments greater than a page are unsupported"
            );
            if let Ok(pages) = self.page_allocator.allocate(page_count) {
                let mut header: NonNull<AllocatedHeader> = pages.cast();
                *header.as_mut() = AllocatedHeader {
                    size: required_block_size,
                };
                pages
                    .cast()
                    .add(alloc_header_layout.size() + between_header_and_data_padding)
                    .as_ptr()
            } else {
                null_mut()
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(ptr) = NonNull::new(ptr) {
            let alloc_header_layout = Layout::new::<AllocatedHeader>();

            // Trust the `layout` to locate the header.
            // This has the side-effect of checking the validity of the `dealloc` call in a sense.
            let between_header_and_data_padding =
                alloc_header_layout.padding_needed_for(layout.align());
            let header_offset = between_header_and_data_padding + size_of::<AllocatedHeader>();
            let header: NonNull<AllocatedHeader> = ptr
                .offset(-(isize::try_from(header_offset).unwrap()))
                .cast();
            let block_claimed_size = header.as_ref().size;
            let total_size =
                alloc_header_layout.size() + between_header_and_data_padding + layout.size();
            assert!(block_claimed_size >= total_size,
                "block_claimed_size<total_size! block_claimed_size={block_claimed_size}, total_size={total_size}, padding={between_header_and_data_padding}, layout={layout:?}");

            let mut free_block: NonNull<FreeHeader> = header.cast();
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
    use std::vec::Vec;

    use test_case::test_matrix;

    use crate::memory::tests::MockPageAllocator;

    use super::*;

    fn create_page_allocator() -> MockPageAllocator {
        MockPageAllocator::new(4096, 512)
    }

    fn allocate_batch<A: GlobalAlloc>(allocator: &A, layout: Layout, size: usize) -> Vec<*mut u8> {
        (0..size)
            .map(|_| unsafe {
                let p = allocator.alloc(layout);
                assert!(!p.is_null());
                assert!(p.is_aligned_to(layout.align()));
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
