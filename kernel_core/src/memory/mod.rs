//! Memory managment algorithms and policies.

use core::marker::PhantomData;
use snafu::Snafu;

/// Errors that arise due to memory related operations.
#[derive(Debug, Snafu)]
pub enum Error {
    /// The system has run out of free memory.
    OutOfMemory,
    /// A size was provided that is not valid (i.e. is zero, or is too large).
    InvalidSize,
    /// A pointer was provided that is not known to the allocator.
    UnknownPtr,
}

type Result<T> = core::result::Result<T, Error>;

/// A 48-bit physical address pointer that is not part of a virtual address space.
///
/// Although in the kernel the virtual addresses are identity mapped, the high bits of the address
/// must be `0xffff` to select the kernel page tables, so a `*mut T` is not quite but very close to
/// the physical address of the `T`.
///
/// The type `T` is the type of the object pointed to. The default of `()` is given because often
/// physical addresses are given that don't point to concrete objects.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysicalPointer<T = ()>(usize, PhantomData<*mut T>);

impl<T> core::fmt::Debug for PhysicalPointer<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "p:0x{:x}", self.0)
    }
}

impl<T> From<usize> for PhysicalPointer<T> {
    fn from(value: usize) -> Self {
        assert!(value < 0xffff_0000_0000_0000);
        PhysicalPointer(value, PhantomData)
    }
}

impl<T> From<PhysicalPointer<T>> for usize {
    fn from(val: PhysicalPointer<T>) -> Self {
        val.0
    }
}

impl<T> From<*const T> for PhysicalPointer<T> {
    fn from(value: *const T) -> Self {
        PhysicalPointer(value as usize & 0x0000_ffff_ffff_ffff, PhantomData)
    }
}

impl<T> From<PhysicalPointer<T>> for *const T {
    fn from(val: PhysicalPointer<T>) -> Self {
        (val.0 | 0xffff_0000_0000_0000) as _
    }
}

impl<T> From<*mut T> for PhysicalPointer<T> {
    fn from(value: *mut T) -> Self {
        PhysicalPointer(value as usize & 0x0000_ffff_ffff_ffff, PhantomData)
    }
}

impl<T> From<PhysicalPointer<T>> for *mut T {
    fn from(val: PhysicalPointer<T>) -> Self {
        (val.0 | 0xffff_0000_0000_0000) as _
    }
}

/// A memory allocator that provides pages of physical memory.
///
/// Implementers of this trait must provide internal synchronization.
pub trait PageAllocator {
    /// The size in bytes of one page.
    ///
    /// This value must remain the same throughout the lifetime of the allocator.
    fn page_size(&self) -> usize;

    /// Allocate `num_pages` of memory, returning a pointer to the beginning which will be page-aligned.
    ///
    /// The pointer is a valid kernel space pointer, but can be translated to a raw physical
    /// address with [`PhysicalPointer`] if need be.
    ///
    /// # Errors
    /// - [`Error::OutOfMemory`] if there is not enough memory to allocate `num_pages`.
    /// - [`Error::InvalidSize`] if `num_pages` is zero.
    fn allocate(&self, num_pages: usize) -> Result<*mut u8>;

    /// Free the pages pointed to by `pages`. This pointer must have been returned at some point
    /// from [`PageAllocator::allocate`].
    ///
    /// # Errors
    /// - [`Error::UnknownPtr`] if `pages` is null or was not allocated by this allocator (including the null pointer).
    fn free(&self, pages: *mut u8) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use snafu::{ensure, OptionExt};

    use crate::memory::{InvalidSizeSnafu, OutOfMemorySnafu, UnknownPtrSnafu};

    use super::PageAllocator;

    #[macro_export]
    macro_rules! test_page_allocator {
        ($allocator_name:ident, $create_allocator:expr) => {
            paste::paste!{
                mod [<$allocator_name:snake:lower _implements_page_allocator>] {
                    use crate::memory::{PageAllocator, Error};
                    use super::$allocator_name;
                    // Test the page size is consistent
                    #[test]
                    fn page_size_consistency() {
                        let allocator = $create_allocator;
                        let page_size = allocator.page_size();
                        assert!(page_size > 0, "Page size should be greater than 0");
                        // Allocating multiple times should return the same page size
                        for _ in 0..10 {
                            assert_eq!(allocator.page_size(), page_size, "Page size should remain constant");
                        }
                    }

                    // Test allocating one page and freeing it
                    #[test]
                    fn allocate_one_page() {
                        let allocator = $create_allocator;
                        let page_size = allocator.page_size();
                        let ptr = allocator.allocate(1).expect("Failed to allocate a page");
                        assert!(!ptr.is_null(), "Pointer should not be null after allocation");
                        // Ensure the memory address is aligned to the page size
                        assert_eq!(ptr as usize % page_size, 0, "Pointer should be page-aligned");
                        allocator.free(ptr).expect("Failed to free allocated page");
                    }

                    // Test allocating multiple pages and freeing them
                    #[test]
                    fn allocate_multiple_pages() {
                        let allocator = $create_allocator;
                        let page_size = allocator.page_size();
                        let num_pages = 4;
                        let ptr = allocator.allocate(num_pages).expect("Failed to allocate multiple pages");
                        assert!(!ptr.is_null(), "Pointer should not be null after allocation");
                        // Ensure the memory address is aligned to the page size
                        assert_eq!(ptr as usize % page_size, 0, "Pointer should be page-aligned");
                        allocator.free(ptr).expect("Failed to free allocated pages");
                    }

                    // Test allocating zero pages (should fail)
                    #[test]
                    fn allocate_zero_pages() {
                        let allocator = $create_allocator;
                        let result = allocator.allocate(0);
                        assert!(matches!(result, Err(Error::InvalidSize)), "Allocating 0 pages should fail");
                    }

                    // Test double free (should fail or behave correctly)
                    #[test]
                    fn double_free() {
                        let allocator = $create_allocator;
                        let ptr = allocator.allocate(1).expect("Failed to allocate a page");
                        allocator.free(ptr).expect("Failed to free allocated page");
                        let result = allocator.free(ptr);
                        assert!(matches!(result, Err(Error::UnknownPtr)), "Double free should fail");
                    }

                    // Test freeing a null pointer (should fail)
                    #[test]
                    fn free_null_pointer() {
                        let allocator = $create_allocator;
                        let result = allocator.free(std::ptr::null_mut());
                        assert!(matches!(result, Err(Error::UnknownPtr)), "Freeing a null pointer should fail");
                    }

                    // Test allocating more pages than available (simulated out-of-memory condition)
                    #[test]
                    fn out_of_memory() {
                        let allocator = $create_allocator;
                        // Simulate by attempting to allocate a large number of pages
                        let result = allocator.allocate(usize::MAX);
                        assert!(matches!(result, Err(Error::OutOfMemory)), "Allocator should return an error when out of memory");
                    }

                    // Test stress: allocate and free multiple pages in a loop
                    #[test]
                    fn stress_allocation() {
                        let allocator = $create_allocator;
                        let num_pages = 8;
                        for _ in 0..100 {
                            let ptr = allocator.allocate(num_pages).expect("Failed to allocate pages");
                            assert!(!ptr.is_null(), "Pointer should not be null after allocation");
                            allocator.free(ptr).expect("Failed to free allocated pages");
                        }
                    }

                    // Test sequential allocate and free
                    #[test]
                    fn sequential_alloc_free() {
                        let allocator = $create_allocator;
                        let num_pages = 4;

                        let ptr1 = allocator.allocate(num_pages).expect("Failed to allocate pages");
                        assert!(!ptr1.is_null(), "Pointer 1 should not be null");
                        allocator.free(ptr1).expect("Failed to free pointer 1");

                        let ptr2 = allocator.allocate(num_pages).expect("Failed to allocate pages");
                        assert!(!ptr2.is_null(), "Pointer 2 should not be null");
                        allocator.free(ptr2).expect("Failed to free pointer 2");
                    }

                    // Test interleaved allocate/free pattern
                    #[test]
                    fn interleaved_alloc_free() {
                        let allocator = $create_allocator;

                        let ptr1 = allocator.allocate(2).expect("Failed to allocate pages");
                        let ptr2 = allocator.allocate(3).expect("Failed to allocate pages");
                        let ptr3 = allocator.allocate(1).expect("Failed to allocate pages");

                        // Free ptr2 first
                        allocator.free(ptr2).expect("Failed to free pointer 2");

                        // Now free ptr1 and ptr3
                        allocator.free(ptr1).expect("Failed to free pointer 1");
                        allocator.free(ptr3).expect("Failed to free pointer 3");
                    }

                    // Test allocation, freeing some, then reallocating more
                    #[test]
                    fn partial_free_then_allocate() {
                        let allocator = $create_allocator;

                        let ptr1 = allocator.allocate(4).expect("Failed to allocate 4 pages");
                        let ptr2 = allocator.allocate(2).expect("Failed to allocate 2 pages");

                        // Free the first allocation
                        allocator.free(ptr1).expect("Failed to free pointer 1");

                        // Allocate a new set of pages after freeing
                        let ptr3 = allocator.allocate(3).expect("Failed to allocate 3 pages");
                        assert!(!ptr3.is_null(), "Pointer 3 should not be null");

                        // Free remaining pointers
                        allocator.free(ptr2).expect("Failed to free pointer 2");
                        allocator.free(ptr3).expect("Failed to free pointer 3");
                    }

                    // Test allocating multiple regions and freeing in reverse order
                    #[test]
                    fn free_in_reverse_order() {
                        let allocator = $create_allocator;

                        let ptr1 = allocator.allocate(2).expect("Failed to allocate 2 pages");
                        let ptr2 = allocator.allocate(3).expect("Failed to allocate 3 pages");
                        let ptr3 = allocator.allocate(1).expect("Failed to allocate 1 page");

                        // Free in reverse order of allocation
                        allocator.free(ptr3).expect("Failed to free pointer 3");
                        allocator.free(ptr2).expect("Failed to free pointer 2");
                        allocator.free(ptr1).expect("Failed to free pointer 1");
                    }

                    // Test mixed small and large allocations
                    #[test]
                    fn mixed_small_and_large_allocations() {
                        let allocator = $create_allocator;

                        let small_ptr = allocator.allocate(1).expect("Failed to allocate 1 page");
                        let large_ptr = allocator.allocate(16).expect("Failed to allocate 16 pages");

                        assert!(!small_ptr.is_null(), "Small pointer should not be null");
                        assert!(!large_ptr.is_null(), "Large pointer should not be null");

                        // Free in reverse order of allocation
                        allocator.free(large_ptr).expect("Failed to free large pointer");
                        allocator.free(small_ptr).expect("Failed to free small pointer");
                    }

                    // Test allocate/free in a loop to simulate reuse of pages
                    #[test]
                    fn allocate_free_loop() {
                        let allocator = $create_allocator;
                        let num_pages = 2;

                        for _ in 0..100 {
                            let ptr = allocator.allocate(num_pages).expect("Failed to allocate pages");
                            assert!(!ptr.is_null(), "Pointer should not be null");
                            allocator.free(ptr).expect("Failed to free allocated pages");
                        }
                    }

                    // Test multiple allocations without freeing, then freeing all at once
                    #[test]
                    fn allocate_without_freeing_then_free_all() {
                        let allocator = $create_allocator;

                        let mut ptrs = std::vec::Vec::new();
                        for _ in 0..5 {
                            let ptr = allocator.allocate(2).expect("Failed to allocate 2 pages");
                            assert!(!ptr.is_null(), "Pointer should not be null");
                            ptrs.push(ptr);
                        }

                        // Now free all allocations
                        for ptr in ptrs {
                            allocator.free(ptr).expect("Failed to free allocated pages");
                        }
                    }

                    // Test multiple allocations without freeing, then freeing all at once in reverse
                    #[test]
                    fn allocate_without_freeing_then_free_all_rev() {
                        let allocator = $create_allocator;

                        let mut ptrs = std::vec::Vec::new();
                        for _ in 0..5 {
                            let ptr = allocator.allocate(2).expect("Failed to allocate 2 pages");
                            assert!(!ptr.is_null(), "Pointer should not be null");
                            ptrs.push(ptr);
                        }

                        // Now free all allocations
                        for ptr in ptrs.into_iter().rev() {
                            allocator.free(ptr).expect("Failed to free allocated pages");
                        }
                    }

                    #[test]
                    fn allocate_huge() {
                        let allocator = $create_allocator;

                        let size = 128;
                        let ptr = allocator.allocate(size).expect("Failed to allocate max pages");
                        assert!(!ptr.is_null(), "Pointer should not be null for max allocation");

                        allocator.free(ptr).expect("Failed to free max allocation");
                    }
                }
            }
        };
    }

    use core::alloc::Layout;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Mock implementation of a `PageAllocator` trait with page limits.
    pub struct MockAllocator {
        page_size: usize,
        max_pages: usize,
        allocated_pages: Arc<Mutex<HashMap<*mut u8, usize>>>, // Map from pointer to number of pages
        total_allocated: Arc<Mutex<usize>>,                   // Tracks total allocated pages
    }

    impl MockAllocator {
        /// Create a new `MockAllocator` with the given page size and maximum number of pages.
        pub fn new(page_size: usize, max_pages: usize) -> Self {
            assert!(page_size > 0);
            assert!(max_pages > 0);
            MockAllocator {
                page_size,
                max_pages,
                allocated_pages: Default::default(),
                total_allocated: Arc::new(Mutex::new(0)),
            }
        }
    }

    impl PageAllocator for MockAllocator {
        fn page_size(&self) -> usize {
            self.page_size
        }

        fn allocate(&self, num_pages: usize) -> super::Result<*mut u8> {
            ensure!(num_pages > 0, InvalidSizeSnafu);

            let mut total_allocated = self.total_allocated.lock().unwrap();

            // Check if there's enough space to allocate the requested number of pages
            ensure!(
                *total_allocated + num_pages <= self.max_pages,
                OutOfMemorySnafu
            );

            // Calculate the total size in bytes for the allocation
            let total_size = self
                .page_size
                .checked_mul(num_pages)
                .context(OutOfMemorySnafu)?;
            let layout = Layout::from_size_align(total_size, self.page_size).unwrap();

            unsafe {
                // Allocate the memory
                let ptr = std::alloc::alloc_zeroed(layout);
                ensure!(!ptr.is_null(), OutOfMemorySnafu);

                // Add the pointer and its allocated size to the map of allocated pages
                let mut allocated_pages = self.allocated_pages.lock().unwrap();
                allocated_pages.insert(ptr, num_pages);

                // Update the total allocated pages count
                *total_allocated += num_pages;

                Ok(ptr)
            }
        }

        fn free(&self, pages: *mut u8) -> super::Result<()> {
            ensure!(!pages.is_null(), UnknownPtrSnafu);

            let num_pages = {
                let mut allocated_pages = self.allocated_pages.lock().unwrap();

                // Ensure the pointer was allocated by this allocator and get the number of pages allocated
                allocated_pages.remove(&pages).context(UnknownPtrSnafu)?
            };

            // Deallocate the memory
            let total_size = self.page_size * num_pages;
            let layout = Layout::from_size_align(total_size, self.page_size).unwrap();

            unsafe {
                std::alloc::dealloc(pages, layout);
            }

            // Update the total allocated pages count
            let mut total_allocated = self.total_allocated.lock().unwrap();
            *total_allocated -= num_pages;

            Ok(())
        }
    }

    test_page_allocator!(MockAllocator, MockAllocator::new(4096, 1024));
}
