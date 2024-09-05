use crate::generic_asserts;
use alloc::alloc::{alloc, dealloc};
use core::alloc::Layout;
use core::fmt::Debug;
use core::mem::{align_of, size_of};
use core::ptr::slice_from_raw_parts_mut;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

/// A Vector-like data structure that allows for concurrent access and insertion.
/// It has a fixed capacity and cannot be resized.
/// Once elements have been appended, they cannot be removed, unless it was the most recently inserted element.
pub struct ConcurrentArray<T> {
    inner: ConcurrentArena<T>,
    capacity: usize,
}

impl<T> ConcurrentArray<T> {
    pub fn new(capacity: usize) -> Self {
        generic_asserts!((T);
            NON_ZST: size_of::<T>() != 0;
            POWER_2_ALIGN: align_of::<T>().is_power_of_two();
        );
        Self {
            inner: ConcurrentArena::new(capacity),
            capacity,
        }
    }

    pub fn push(&self, item: T) -> Option<(&T, usize)> {
        let alloc_res = self.inner.push()?;
        let ptr = alloc_res.bytes.cast::<T>();
        unsafe {
            ptr.write(item);
        }
        Some((unsafe { &*ptr }, alloc_res.index / size_of::<T>()))
    }

    /// Remove an item from the arena at the given index. Will only remove the item if it was
    /// the most recently added item.
    pub fn remove(&self, index: usize) -> bool {
        if index >= self.capacity {
            return false;
        }
        self.inner.try_remove(index * size_of::<T>())
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.inner
            .get(index * size_of::<T>())
            .map(|ptr| unsafe { &*ptr.cast::<T>() })
    }
}

struct AllocResult {
    bytes: *mut u8,
    index: usize,
}

/// A very basic arena "allocator" that allows for lock-free concurrent allocation.
/// De-allocation will only succeed if the block being deallocated was the most recently allocated.
/// None of the functions are aware of `T`, it's just used to enforce correct memory layout.
struct ConcurrentArena<T> {
    bytes: *mut [u8],
    next: AtomicUsize,
    byte_capacity: usize,
    _marker: core::marker::PhantomData<T>,
}

impl<T> ConcurrentArena<T> {
    /// Construct a new `ConcurrentArena` which can hold `item_capacity` amount of T.
    pub fn new(item_capacity: usize) -> Self {
        let layout = Self::full_layout(item_capacity);
        let bytes_ptr = unsafe { alloc(layout) };
        let bytes_slice = slice_from_raw_parts_mut(bytes_ptr, layout.size());
        Self {
            bytes: bytes_slice,
            next: AtomicUsize::new(0),
            byte_capacity: layout.size(),
            _marker: core::marker::PhantomData,
        }
    }

    pub fn push(&self) -> Option<AllocResult> {
        self.next
            .fetch_update(Ordering::Release, Ordering::Acquire, |next| {
                if next >= self.byte_capacity {
                    return None;
                }
                Some(next + size_of::<T>())
            })
            .ok()
            .map(|next| AllocResult {
                bytes: unsafe { self.bytes.cast::<u8>().add(next) },
                index: next,
            })
    }

    pub fn get(&self, index: usize) -> Option<*mut u8> {
        if index >= self.next.load(Ordering::Acquire) {
            return None;
        }

        Some(unsafe { self.bytes.cast::<u8>().add(index) })
    }

    /// Remove an item from the arena at index. Will only succeed if the item was the most recently
    /// added item.
    pub fn try_remove(&self, index: usize) -> bool {
        let expected = index + size_of::<T>();
        self.next
            .compare_exchange(expected, index, Ordering::Release, Ordering::Acquire)
            .is_ok()
    }

    fn index_to_slice(&self, index: usize) -> *mut [u8] {
        unsafe { slice_from_raw_parts_mut(self.bytes.cast::<u8>().add(index), size_of::<T>()) }
    }

    fn elem_count(&self) -> usize {
        self.byte_capacity / size_of::<T>()
    }

    const fn full_layout(item_count: usize) -> Layout {
        unsafe { Layout::from_size_align_unchecked(size_of::<T>() * item_count, align_of::<T>()) }
    }
}

impl<T> Drop for ConcurrentArena<T> {
    fn drop(&mut self) {
        unsafe {
            self.next.store(usize::MAX, Ordering::Release);
            dealloc(self.bytes.cast(), Self::full_layout(self.elem_count()));
        }
    }
}

unsafe impl<T> Send for ConcurrentArena<T> {}
unsafe impl<T> Sync for ConcurrentArena<T> {}

impl<T> Debug for ConcurrentArena<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConcurrentArena")
            .field("bytes", &self.bytes)
            .field("next", &self.next)
            .field("byte_capacity", &self.byte_capacity)
            .finish()
    }
}

impl<T> Debug for ConcurrentArray<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConcurrentArray")
            .field("inner", &self.inner)
            .field("capcity", &self.capacity)
            .finish()
    }
}
