//

use core::{
    marker::PhantomData,
    num::NonZeroU32,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{boxed::Box, sync::Arc};

use super::HandleAllocator;

/// A handle allocated by a [`HandleAllocator`].
pub type Handle = NonZeroU32;

struct Table<T>([AtomicUsize; 256], PhantomData<Arc<T>>);

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self(
            core::array::from_fn(|_| AtomicUsize::default()),
            PhantomData,
        )
    }
}

impl<T> Table<T> {
    /// Get the `Arc<T>` stored at `index`, or `None` if there is no value at that index.
    ///
    /// # Safety
    /// Assumes that if there is a non-zero value at `index` then it is a value.
    unsafe fn get_value(&self, index: usize) -> Option<Arc<T>> {
        let v = self.0[index].load(Ordering::Acquire);
        if v == 0 {
            None
        } else {
            Arc::increment_strong_count(v as *mut T);
            Some(Arc::from_raw(v as _))
        }
    }

    /// Take the `Arc<T>` stored at `index`, or `None` if there is no value at that index.
    /// The index will have nothing stored at it after calling this function.
    ///
    /// # Safety
    /// Assumes that if there is a non-zero value at `index` then it is a value.
    unsafe fn take_value(&self, index: usize) -> Option<Arc<T>> {
        let v = self.0[index].swap(0, Ordering::AcqRel);
        if v == 0 {
            None
        } else {
            Some(Arc::from_raw(v as _))
        }
    }

    /// Store an `Arc<T>` at some `index` in the table.
    /// Returns whatever was in the table before, if anything.
    ///
    /// It is safe to [`Self::get_value()`] for `index` once this has been called for `index`.
    ///
    /// # Safety
    /// Assumes that if there is a non-zero value at `index` then it is a value.
    unsafe fn put_value(&self, index: usize, val: Arc<T>) -> Option<Arc<T>> {
        let v = self.0[index].swap(Arc::into_raw(val) as _, Ordering::AcqRel);
        if v == 0 {
            None
        } else {
            Some(Arc::from_raw(v as _))
        }
    }

    /// Get the `Table<T>` stored at `index`, or `None` if there is no table at that index.
    ///
    /// # Safety
    /// Assumes that if there is a non-zero value at `index` then it is a non-null table.
    unsafe fn get_table(&self, index: usize) -> Option<NonNull<Table<T>>> {
        let v = self.0[index].load(Ordering::Acquire);
        if v == 0 {
            None
        } else {
            NonNull::new(v as _)
        }
    }

    /// Attempt to store a new next-level table at `index`, assuming that the slot is empty.
    /// If it is not empty, then the table that is stored there is returned instead.
    ///
    /// # Safety
    /// Assumes that if there is an existing non-zero value at this index, then it is a table.
    unsafe fn new_next_level_table(&self, index: usize) -> NonNull<Table<T>> {
        let new_table = NonNull::new_unchecked(Box::into_raw(Box::new(Table::default())));
        match self.0[index].compare_exchange(
            0,
            new_table.as_ptr() as _,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => new_table,
            Err(v) => {
                // we didn't use the new table, so free it
                drop(Box::from_raw(new_table.as_ptr()));
                NonNull::new(v as _).expect("c/x for 0 returned Err(0) which is nonsense")
            }
        }
    }

    fn drop_children(&mut self, depth: usize) {
        // because we have an exclusive reference to the table, we know there are no other threads accessing the table.
        // Therefore, we can safely use `Relaxed` operations.
        for entry in &self.0 {
            let v = entry.swap(0, Ordering::Relaxed);
            // #[cfg(test)]
            // std::println!("drop: {v:x}, {depth}");
            match (v, depth) {
                (_, 0) => unreachable!(),
                (0, _) => {}
                (_, 1) => {
                    let val: Arc<T> = unsafe { Arc::from_raw(v as _) };
                    drop(val);
                }
                (_, _) => {
                    let mut tbl: Box<Table<T>> = unsafe { Box::from_raw(v as _) };
                    tbl.drop_children(depth - 1);
                    drop(tbl);
                }
            }
        }
    }
}

impl<T> Drop for Table<T> {
    fn drop(&mut self) {
        for entry in &self.0 {
            assert_eq!(
                entry.load(Ordering::Relaxed),
                0,
                "must call `drop_children` before a Table is dropped"
            );
        }
    }
}

/// An internally synchronized concurrent map from handles to atomically ref-counted values of type `T`.
pub struct HandleMap<T> {
    allocator: HandleAllocator,
    table: Table<T>,
    handle_zeros_prefix_bit_length: u32,
    depth: usize,
}

impl<T> HandleMap<T> {
    /// Create a new `HandleMap` that can have up to `max_handle` objects in it.
    #[must_use]
    pub fn new(max_handle: Handle) -> Self {
        let extra_bits = max_handle.leading_zeros() & !7;
        Self {
            allocator: HandleAllocator::new(max_handle),
            table: Table::default(),
            // compute the length of the zero prefix for all handles so we can skip some tables.
            handle_zeros_prefix_bit_length: extra_bits,
            depth: (32 - extra_bits).div_ceil(8) as usize,
        }
    }

    fn leaf_table_for_handle(&self, handle: Handle) -> Option<(&Table<T>, usize)> {
        let mut handle =
            ((u32::from(handle) - 1) << self.handle_zeros_prefix_bit_length).rotate_left(8);
        let mut table = &self.table;
        for _ in 0..(self.depth - 1) {
            let index = handle & 0xff;
            table = unsafe { table.get_table(index as usize)?.as_ref() };
            handle = handle.rotate_left(8);
        }
        Some((table, (handle & 0xff) as usize))
    }

    /// Get a new handle that refers to `value`.
    /// Calling this method twice with the same value may return two different handles.
    ///
    /// # Errors
    /// If there are no handles left, then the value is returned in `Err`.
    pub fn insert(&self, value: Arc<T>) -> Result<Handle, Arc<T>> {
        match self.insert_self_referential(|_| value.clone()) {
            Some((h, _)) => Ok(h),
            None => Err(value),
        }
    }

    /// Get a new handle that refers to the result of `make_value`.
    /// Returns the handle and the value that was made.
    ///
    /// # Errors
    /// If there are no handles left, then the value is returned in `Err`.
    ///
    /// # Panics
    /// If internal invariants are violated.
    pub fn insert_self_referential(
        &self,
        make_value: impl FnOnce(Handle) -> Arc<T>,
    ) -> Option<(Handle, Arc<T>)> {
        let handle = self.allocator.next_handle()?;
        let mut handle_ix =
            ((u32::from(handle) - 1) << self.handle_zeros_prefix_bit_length).rotate_left(8);
        let mut table = &self.table;
        for _ in 0..(self.depth - 1) {
            let index = handle_ix & 0xff;
            table = unsafe {
                match table.get_table(index as usize) {
                    Some(t) => t.as_ref(),
                    None => table.new_next_level_table(index as usize).as_ref(),
                }
            };
            handle_ix = handle_ix.rotate_left(8);
        }
        let index = (handle_ix & 0xff) as usize;
        let val = make_value(handle);
        unsafe {
            let res = table.put_value(index, val.clone());
            assert!(res.is_none(), "Because the allocator always returns a previously free handle, there should be nothing at this table position.");
        }
        Some((handle, val))
    }

    /// Returns a reference to the value associated with `handle`.
    /// If the handle is unknown, then `None` is returned.
    pub fn get(&self, handle: Handle) -> Option<Arc<T>> {
        let (table, leaf_index) = self.leaf_table_for_handle(handle)?;
        unsafe { table.get_value(leaf_index) }
    }

    /// Removes a value from the map by its handle.
    /// Returns a reference to the value associated with `handle`.
    /// If the handle is unknown, then `None` is returned.
    pub fn remove(&self, handle: Handle) -> Option<Arc<T>> {
        let (table, leaf_index) = self.leaf_table_for_handle(handle)?;
        let val = unsafe { table.take_value(leaf_index) };
        self.allocator.free_handle(handle).ok()?;
        val
    }
}

impl<T> Drop for HandleMap<T> {
    fn drop(&mut self) {
        self.table.drop_children(self.depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::seq::SliceRandom;
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex},
        thread,
        vec::Vec,
    };
    use test_case::{test_case, test_matrix};

    /// Test that inserting a value into the map works and that it can be retrieved.
    #[test]
    fn test_insert_and_get() {
        let max_handle = 10;
        let handle_map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(max_handle).unwrap());
        println!("created map");
        let value = Arc::new(42);
        println!("pre-insert");
        let handle = handle_map.insert(value.clone()).expect("Insert failed");
        println!("post-insert");
        let retrieved_value = handle_map.get(handle).expect("Value not found");
        println!("post get: {retrieved_value}");
        assert_eq!(*retrieved_value, 42);
    }

    #[test_case(16)]
    #[test_case(1024)]
    #[test_case(0xffff)]
    fn get_back_what_you_put_in(n: u32) {
        let map: HandleMap<usize> = HandleMap::new(NonZeroU32::new(n).unwrap());
        let mut handles = Vec::new();
        let mut values = HashSet::new();
        for i in 0..n {
            let value = (i as usize) * 1737;
            let handle = map.insert(value.into()).expect("insert");
            handles.push(handle);
            values.insert((handle, value));
        }
        let mut rng = rand::thread_rng();
        handles.shuffle(&mut rng);
        for handle in handles {
            let value = map.get(handle).expect("handle in map");
            assert!(values.remove(&(handle, *value)));
        }
        assert!(values.is_empty());
    }

    #[test_case(16)]
    #[test_case(1024)]
    #[test_case(0xffff)]
    fn remove_back_what_you_put_in(n: u32) {
        let map: HandleMap<usize> = HandleMap::new(NonZeroU32::new(n).unwrap());
        let mut handles = Vec::new();
        let mut values = HashSet::new();
        for i in 0..n {
            let value = (i as usize) * 7371;
            let handle = map.insert(value.into()).expect("insert");
            handles.push(handle);
            values.insert((handle, value));
        }
        let mut rng = rand::thread_rng();
        handles.shuffle(&mut rng);
        for handle in handles {
            let value = map.remove(handle).expect("handle in map");
            assert!(values.remove(&(handle, *value)));
        }
        assert!(values.is_empty());
    }

    /// Test that `get` returns `None` for an unknown handle.
    #[test]
    fn test_get_unknown_handle() {
        let max_handle = 10;
        let handle_map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(max_handle).unwrap());
        assert!(handle_map.get(NonZeroU32::new(999).unwrap()).is_none());
    }

    /// Test that `remove` removes the value and subsequent `get` returns `None`.
    #[test]
    fn test_remove() {
        let max_handle = 10;
        let handle_map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(max_handle).unwrap());
        let value = Arc::new(42);
        let handle = handle_map.insert(value.clone()).expect("Insert failed");
        let removed_value = handle_map.remove(handle).expect("Remove failed");
        assert_eq!(*removed_value, 42);
        assert!(handle_map.get(handle).is_none());
    }

    /// Test that removing an unknown handle returns `None`.
    #[test]
    fn test_remove_unknown_handle() {
        let max_handle = 10;
        let handle_map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(max_handle).unwrap());
        assert!(handle_map.remove(NonZeroU32::new(999).unwrap()).is_none());
    }

    /// Test that inserting more than `max_handle` values returns `Err`.
    #[test_case(1)]
    #[test_case(5)]
    #[test_case(10)]
    fn test_insert_max_handles(max_handle: u32) {
        let handle_map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(max_handle).unwrap());
        let value = Arc::new(42);
        let mut handles = Vec::new();
        for _ in 0..max_handle {
            let handle = handle_map.insert(value.clone()).expect("Insert failed");
            handles.push(handle);
        }
        // Next insert should fail
        let result = handle_map.insert(value.clone());
        assert!(result.is_err(), "Expected insert to fail when map is full");
    }

    /// Test that inserting the same value multiple times returns different handles.
    #[test]
    fn test_insert_same_value_different_handles() {
        let max_handle = 10;
        let handle_map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(max_handle).unwrap());
        let value = Arc::new(42);
        let handle1 = handle_map.insert(value.clone()).expect("Insert failed");
        let handle2 = handle_map.insert(value.clone()).expect("Insert failed");
        assert_ne!(
            handle1, handle2,
            "Handles should be different for the same value"
        );
    }

    /// Test concurrent inserts to ensure thread safety.
    #[test_matrix(
        [1,16],
        [1,10,100]
    )]
    fn test_concurrent_inserts(num_threads: usize, num_handles_per_thread: usize) {
        let max_handle = num_threads * num_handles_per_thread;
        let handle_map = Arc::new(HandleMap::new(NonZeroU32::new(max_handle as u32).unwrap()));
        let value = Arc::new(42);

        thread::scope(|s| {
            for _ in 0..num_threads {
                let handle_map = Arc::clone(&handle_map);
                let value = Arc::clone(&value);
                s.spawn(move || {
                    for _ in 0..num_handles_per_thread {
                        let _ = handle_map.insert(value.clone());
                    }
                });
            }
        });

        // Attempt to insert another value, which should fail if the map is full.
        let result = handle_map.insert(value.clone());
        assert!(result.is_err(), "Expected insert to fail when map is full");
    }

    /// Test concurrent inserts and gets to ensure consistent behavior.
    #[test_case(16, 1)]
    #[test_case(16, 8)]
    #[test_case(4096, 1)]
    #[test_case(4096, 16)]
    #[test_case(0xffff, 1)]
    #[test_case(0xffff, 32)]
    #[test_case(1234, 5)]
    fn test_concurrent_insert_and_get(n: u32, num_threads: usize) {
        let map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(n).unwrap());
        let mut test_vals = HashMap::new();
        for i in 0..(n / 3) {
            let h = map.insert(Arc::new(i * 10)).unwrap();
            test_vals.insert(h, i * 10);
        }
        let gen_hdls = Mutex::new(Vec::new());

        thread::scope(|s| {
            for _ in 0..(num_threads / 2) {
                s.spawn(|| {
                    let v = Arc::new(0);
                    let mut local_hdls = Vec::new();
                    for _ in 0..(n / (num_threads as u32 * 6)) {
                        let h = map.insert(v.clone()).unwrap();
                        assert!(!test_vals.contains_key(&h), "handle returned twice: {h}");
                        local_hdls.push(h);
                    }
                    gen_hdls.lock().unwrap().extend(local_hdls);
                });
            }

            for _ in 0..(num_threads / 2) {
                s.spawn(|| {
                    for _ in 0..9 {
                        for (h, v) in &test_vals {
                            assert_eq!(map.get(*h).as_deref(), Some(v));
                        }
                    }
                });
            }
        });

        for h in gen_hdls.into_inner().unwrap() {
            assert_eq!(map.get(h).as_deref(), Some(&0));
        }
    }

    /// Test concurrent inserts and removes to ensure the map remains consistent.
    #[test_case(16, 1)]
    #[test_case(16, 8)]
    #[test_case(4096, 1)]
    #[test_case(4096, 8)]
    #[test_case(4096, 16)]
    #[test_case(0xffff, 1)]
    #[test_case(0xffff, 32)]
    #[test_case(1234, 5)]
    fn test_concurrent_insert_and_remove(n: u32, num_threads: usize) {
        let map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(n).unwrap());
        let mut test_vals = HashMap::new();
        for i in 0..(n / 3) {
            let h = map.insert(Arc::new(i * 10)).unwrap();
            test_vals.insert(h, i * 10);
        }
        let gen_hdls = Mutex::new(Vec::new());

        thread::scope(|s| {
            for _ in 0..(num_threads / 2) {
                s.spawn(|| {
                    let v = Arc::new(0);
                    let mut local_hdls = Vec::new();
                    for _ in 0..(n / (num_threads as u32 * 6)) {
                        let h = map.insert(v.clone()).unwrap();
                        local_hdls.push(h);
                    }
                    gen_hdls.lock().unwrap().extend(local_hdls);
                });
            }

            s.spawn(|| {
                for (h, v) in &test_vals {
                    assert_eq!(map.remove(*h).as_deref(), Some(v));
                }
            });
        });

        for h in gen_hdls.into_inner().unwrap() {
            assert_eq!(map.get(h).as_deref(), Some(&0));
        }
    }

    #[test_case(8)]
    fn concurrent_independent_insert_remove(num_threads: usize) {
        let map: HandleMap<u32> = HandleMap::new(NonZeroU32::new(1024).unwrap());

        thread::scope(|s| {
            for n in 0..num_threads {
                let n = n as u32;
                let map = &map;
                s.spawn(move || {
                    let v = Arc::new(n);
                    for _ in 0..2048 {
                        let h = map.insert(v.clone()).unwrap();
                        assert_eq!(map.remove(h).as_deref(), Some(&n));
                    }
                });
            }
        });
    }

    /// Test that handles are unique across different inserts.
    #[test]
    fn test_handle_uniqueness() {
        let max_handle = 1000;
        let handle_map = HandleMap::new(NonZeroU32::new(max_handle).unwrap());
        let value = Arc::new(42);
        let mut handles = HashSet::new();

        for _ in 0..max_handle {
            let handle = handle_map.insert(value.clone()).expect("Insert failed");
            assert!(handles.insert(handle), "Handle was not unique");
        }
    }
}
