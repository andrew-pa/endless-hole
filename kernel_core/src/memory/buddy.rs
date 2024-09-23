//! Buddy allocator for pages.

use core::{
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use snafu::{ensure, OptionExt as _};

use crate::memory::{subtract_ranges, InvalidSizeSnafu, OutOfMemorySnafu, UnknownPtrSnafu};

use super::PageAllocator;

#[repr(C)]
struct FreeHeader {
    next_block: AtomicPtr<FreeHeader>,
}

/// Page allocator that uses the buddy memory allocation algorithm to allocate pages of physical
/// memory.
///
/// `MAX_ORDER` is the largest power of two block of pages that will be managed by the allocator.
#[allow(clippy::module_name_repetitions)]
pub struct BuddyPageAllocator<const MAX_ORDER: usize = 12> {
    base_addr: *mut u8,
    end_addr: *mut u8,
    page_size: usize,
    free_blocks: [AtomicPtr<FreeHeader>; MAX_ORDER],
}

unsafe impl Send for BuddyPageAllocator {}
unsafe impl Sync for BuddyPageAllocator {}

impl<const MAX_ORDER: usize> BuddyPageAllocator<MAX_ORDER> {
    /// Create a new allocator that will allocate memory from the region at `memory_start` of length `memory_length` bytes.
    /// The memory start address must be page aligned, and must contain a whole number of pages.
    /// Any regions of `excluded_regions` will be reserved from the greater memory available to the
    /// allocator and will not be allocated.
    ///
    /// # Panics
    ///
    /// This function panics if the aformentioned invarients are not met.
    ///
    /// # Safety
    ///
    /// The memory region provided must be entirely valid memory that is safe to dereference, live for the lifetime of the allocator and not be shared
    /// outside of the allocator.
    /// Additionally the `excluded_regions` must be contained within the overall memory region, see [`subtract_ranges`] for details.
    pub unsafe fn new(
        page_size: usize,
        memory_start: *mut u8,
        memory_length: usize,
        excluded_regions: impl Iterator<Item = (*mut u8, usize)>,
    ) -> Self {
        assert!(memory_start.is_aligned_to(page_size));
        assert_eq!(memory_length % page_size, 0);

        let allocator = Self {
            base_addr: memory_start,
            end_addr: unsafe { memory_start.add(memory_length) },
            page_size,
            free_blocks: [const { AtomicPtr::new(null_mut()) }; MAX_ORDER],
        };

        for (region_start, region_length) in
            subtract_ranges((memory_start, memory_length), excluded_regions)
        {
            let start_alignment_padding = region_start.align_offset(page_size);
            if region_length < page_size || region_length - start_alignment_padding < page_size {
                continue;
            }
            let mut block_start = NonNull::new(region_start.add(start_alignment_padding)).unwrap();
            let mut remaining_bytes = region_length;
            let mut order = MAX_ORDER - 1;
            loop {
                let block_len = (1 << order) * page_size;
                if remaining_bytes >= block_len {
                    allocator.push_free(order, block_start.cast());
                    remaining_bytes -= block_len;
                    block_start = block_start.add(block_len);
                } else {
                    match order.checked_sub(1) {
                        Some(new_order) => order = new_order,
                        None => break,
                    }
                }
            }
        }

        allocator
    }

    /// Pop the next free block of order `order` if one exists.
    fn pop_free(&self, order: usize) -> Option<NonNull<FreeHeader>> {
        let mut head = NonNull::new(self.free_blocks[order].load(Ordering::Acquire))?;
        loop {
            let new_head = unsafe {
                // SAFETY: to become the head, this block must be correctly initialized
                head.as_ref().next_block.load(Ordering::Relaxed)
            };
            match self.free_blocks[order].compare_exchange(
                head.as_ptr(),
                new_head,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    #[cfg(test)]
                    std::println!("pop_free {order} {head:x?}");
                    return Some(head);
                }
                Err(h) => head = NonNull::new(h)?,
            }
        }
    }

    /// Push the newly freed block of order `order` into the free list.
    /// Automatically updates the `next_block` field of the header of the new block.
    ///
    /// # Safety
    ///
    /// We assume that `block` is *not* shared between threads, and that the header is initialized
    /// and reference-convertable.
    unsafe fn push_free(&self, order: usize, mut block: NonNull<FreeHeader>) {
        #[cfg(test)]
        std::println!("push_free {order} {block:x?}");
        assert!(block.is_aligned_to(self.page_size));
        let mut head = self.free_blocks[order].load(Ordering::Acquire);
        loop {
            block.as_mut().next_block.store(head, Ordering::Relaxed);
            match self.free_blocks[order].compare_exchange(
                head,
                block.as_ptr(),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(h) => head = h,
            }
        }
    }

    fn try_remove_buddy(&self, order: usize, buddy: NonNull<FreeHeader>) -> bool {
        let free_list = &self.free_blocks[order];
        // keep trying until successful or not found
        'retry: loop {
            let mut prev_ptr: Option<NonNull<FreeHeader>> = None;
            let mut current_ptr = NonNull::new(free_list.load(Ordering::Acquire));

            // step through the list
            while let Some(current) = current_ptr {
                let next_ptr = unsafe { current.as_ref().next_block.load(Ordering::Relaxed) };

                if current == buddy {
                    // Attempt to remove the buddy from the free list.
                    let success = if let Some(prev_ptr) = prev_ptr {
                        unsafe {
                            prev_ptr
                                .as_ref()
                                .next_block
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
                        free_list
                            .compare_exchange(
                                current.as_ptr(),
                                next_ptr,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_ok()
                    };

                    if success {
                        return true;
                    }

                    // Failed to remove; retry from the beginning of the list.
                    continue 'retry;
                }

                prev_ptr = current_ptr;
                current_ptr = NonNull::new(next_ptr);
            }

            // Buddy not found in the free list.
            return false;
        }
    }

    fn block_in_free_list(&self, order: usize, block: NonNull<FreeHeader>) -> bool {
        let mut cur = NonNull::new(self.free_blocks[order].load(Ordering::Acquire));
        while let Some(n) = cur {
            if n == block {
                return true;
            }
            cur = unsafe { NonNull::new(n.as_ref().next_block.load(Ordering::Relaxed)) };
        }
        false
    }

    #[cfg(test)]
    fn count_in_free_list(&self, order: usize) -> usize {
        let mut count = 0;
        let mut cur = NonNull::new(self.free_blocks[order].load(Ordering::Acquire));
        while let Some(n) = cur {
            count += 1;
            cur = unsafe { NonNull::new(n.as_ref().next_block.load(Ordering::Relaxed)) };
        }
        count
    }

    #[cfg(test)]
    fn total_pages_free(&self) -> usize {
        (0..MAX_ORDER)
            .map(|order| self.count_in_free_list(order) * (1 << order))
            .sum()
    }

    fn split_block_to_size(
        &self,
        block: NonNull<FreeHeader>,
        mut current_order: usize,
        desired_order: usize,
    ) -> NonNull<FreeHeader> {
        while current_order > desired_order {
            current_order -= 1;
            let new_size = 1 << current_order;
            unsafe {
                let new_block = block.cast::<u8>().add(new_size * self.page_size).cast();
                self.push_free(current_order, new_block);
            }
        }
        block
    }

    unsafe fn buddy_of(&self, block: NonNull<FreeHeader>, order: usize) -> NonNull<FreeHeader> {
        let offset: usize = unsafe { block.cast::<u8>().as_ptr().offset_from(self.base_addr) }
            .try_into()
            .unwrap();
        let buddy_offset = offset ^ (self.page_size * (1 << order));
        let ptr = unsafe { self.base_addr.add(buddy_offset) };
        NonNull::new(ptr).unwrap().cast()
    }
}

impl<const MAX_ORDER: usize> PageAllocator for BuddyPageAllocator<MAX_ORDER> {
    fn page_size(&self) -> usize {
        self.page_size
    }

    fn allocate(&self, num_pages: usize) -> super::Result<NonNull<u8>> {
        ensure!(num_pages > 0, InvalidSizeSnafu);

        let block_size = num_pages
            .checked_next_power_of_two()
            .context(OutOfMemorySnafu)?;
        let order = block_size.ilog2() as usize;

        let mut actual_order = order;
        let free_block = loop {
            ensure!(actual_order < MAX_ORDER, OutOfMemorySnafu);
            if let Some(free) = self.pop_free(actual_order) {
                break free;
            }
            actual_order += 1;
        };

        let block = self.split_block_to_size(free_block, actual_order, order);

        Ok(block.cast())
    }

    fn free(&self, pages: NonNull<u8>, num_pages: usize) -> super::Result<()> {
        let block = pages.cast();
        ensure!(num_pages > 0, InvalidSizeSnafu);
        ensure!(
            pages.as_ptr() >= self.base_addr && pages.as_ptr() < self.end_addr,
            UnknownPtrSnafu
        );

        let block_size = num_pages
            .checked_next_power_of_two()
            .context(InvalidSizeSnafu)?;
        let order = block_size.ilog2() as usize;
        ensure!(order < MAX_ORDER, InvalidSizeSnafu);

        let buddy = unsafe { self.buddy_of(block, order) };

        #[cfg(test)]
        std::println!("free block={block:x?} order={order} buddy={buddy:x?}");

        if self.try_remove_buddy(order, buddy) {
            #[cfg(test)]
            std::println!("removed buddy");
            unsafe {
                self.push_free(order + 1, block);
            }
        } else {
            // prevent double frees
            ensure!(!self.block_in_free_list(order, block), UnknownPtrSnafu);
            ensure!(!self.block_in_free_list(order + 1, block), UnknownPtrSnafu);
            ensure!(!self.block_in_free_list(order + 1, buddy), UnknownPtrSnafu);
            #[cfg(test)]
            std::println!("buddy is allocated");
            unsafe {
                self.push_free(order, block);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::alloc::Layout;

    use super::*;
    use crate::test_page_allocator;

    struct TestContext {
        memory: *mut u8,
        layout: Layout,
        num_pages_free_at_end: usize,
    }

    fn setup_allocator() -> (TestContext, BuddyPageAllocator) {
        let page_size = 4096;
        let total_pages = 512;
        let total_size = total_pages * page_size;
        let layout = Layout::from_size_align(total_size, page_size).unwrap();
        let memory = unsafe { std::alloc::alloc(layout) };
        assert!(!memory.is_null());

        (
            TestContext {
                memory,
                layout,
                num_pages_free_at_end: total_pages,
            },
            unsafe { BuddyPageAllocator::new(page_size, memory, total_size, core::iter::empty()) },
        )
    }

    fn setup_allocator_with_gap() -> (TestContext, BuddyPageAllocator) {
        let page_size = 4096;
        let total_pages = 513;
        let total_size = total_pages * page_size;
        let layout = Layout::from_size_align(total_size, page_size).unwrap();
        let memory = unsafe { std::alloc::alloc(layout) };
        assert!(!memory.is_null());

        (
            TestContext {
                memory,
                layout,
                // since one page will be reserved, it will remain unfree
                num_pages_free_at_end: total_pages - 1,
            },
            unsafe {
                BuddyPageAllocator::new(
                    page_size,
                    memory,
                    total_size,
                    [(memory.add(page_size * 256), 67)].into_iter(),
                )
            },
        )
    }

    fn cleanup_allocator(cx: TestContext, allocator: BuddyPageAllocator) {
        // every page should be free at the end
        assert_eq!(allocator.total_pages_free(), cx.num_pages_free_at_end);
        unsafe {
            std::alloc::dealloc(cx.memory, cx.layout);
        }
    }

    test_page_allocator!(BuddyPageAllocator, setup_allocator, cleanup_allocator);
    test_page_allocator!(
        BuddyPageAllocatorWithGap,
        setup_allocator_with_gap,
        cleanup_allocator
    );
}
