//! Definitions and drivers for the ARM platform.

use core::marker::PhantomData;

pub mod device_tree;

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
