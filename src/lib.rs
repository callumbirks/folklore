#![no_std]

extern crate alloc;

mod array;
#[cfg(test)]
mod test;
mod util;

use crate::array::ConcurrentArray;
use atomic::{Atomic, Ordering};
use bytemuck::NoUninit;
use core::borrow::Borrow;
use core::hash::Hash;
use core::mem::size_of;
use core::ptr::slice_from_raw_parts;
use core::sync::atomic::AtomicU16;
use hash32::FnvHasher;

type Size = u16;
type HashT = u32;

const DEFAULT_CAPACITY: usize = 64;
const BUCKET_CAPACITY: Size = 8;
const LOAD_FACTOR: f64 = 0.6;

/// A `HashMap` which doesn't allow any deletion, and only allows for 2-byte values
pub struct HashMap<K, V>
where
    K: Hash + Eq,
    V: Copy + NoUninit,
{
    table: *mut Bucket<V>,
    key_store: ConcurrentArray<K>,
    size_mask: Size,
    capacity: Size,
    count: AtomicU16,
}

impl<K, V> HashMap<K, V>
where
    K: Hash + Eq,
    V: Copy + NoUninit,
{
    ///
    /// # Panics
    /// If `capacity > i16::MAX`
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        // This assertion is only ran at compile time
        generic_asserts!((V);
            VALUE_SIZE: size_of::<V>() == size_of::<Size>();
            ONE_WORD: size_of::<Entry<V>>() == size_of::<u64>();
        );
        // Panic if capacity > i16::MAX
        assert!(i16::try_from(capacity).is_ok());
        // The allocated size of the table is larger than the capacity to allow for LOAD_FACTOR,
        // which improves performance. This also means there will always be empty entrys, which
        // means the unconditional loops in get/insert will never be infinite.
        #[allow(clippy::cast_sign_loss)]
        #[allow(clippy::cast_possible_truncation)]
        #[allow(clippy::cast_precision_loss)]
        let allocated_size = ((capacity as f64 / LOAD_FACTOR) as usize).next_power_of_two();
        // Ensure the highest possible offset won't overflow
        debug_assert!(allocated_size - 1 <= Size::MAX as usize);

        #[allow(clippy::cast_possible_truncation)]
        Self {
            table: create_table(allocated_size),
            key_store: ConcurrentArray::new(capacity.next_power_of_two()),
            size_mask: (allocated_size - 1) as Size,
            capacity: capacity as Size,
            count: AtomicU16::new(0),
        }
    }

    /// Insert a key-value pair into the map.
    /// Returns true if the key was inserted, false if the map is full or the key already exists.
    pub fn insert(&self, key: K, value: V) -> bool {
        if self.count.load(Ordering::Relaxed) >= self.capacity {
            return false;
        }

        let Some(entry) = self._find_empty_entry(&key) else {
            return false;
        };

        let key_hash = self._hash(&key);

        let Some((_, key_index)) = self.key_store.push(key) else {
            return false;
        };

        let key_offset = key_index as Size + constants::MIN_KEY;

        if entry
            .compare_exchange(
                Entry::EMPTY,
                Entry {
                    key_hash,
                    key_offset,
                    value,
                },
                Ordering::Release,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.count.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Get the value associated with a key. Returns None if the key doesn't exist.
    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self._find_entry(key)
            .map(|e| e.load(Ordering::Relaxed))
            .map(|e| e.value)
    }

    /// Get the key at the given index in the map's key store.
    /// Keys are stored in the order they were inserted.
    pub fn get_key(&self, index: usize) -> Option<&K> {
        self.key_store.get(index)
    }

    /// Get the count of key-value pairs in the map.
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed) as usize
    }

    /// Update the value associated with a key. Returns the previous value on success, or None on failure.
    pub fn update<Q: ?Sized>(&self, key: &Q, value: V) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self._fetch_update(key, |current| {
            Some(Entry {
                key_hash: current.key_hash,
                key_offset: current.key_offset,
                value,
            })
        })
        .map(|previous| previous.value)
    }

    /// Update the value associated with a key using an update function. Returns the previous value on success, or None on failure.
    /// The update function should return Some(V) with the new value, or None if the value should not be updated.
    /// # Errors
    /// If the key doesn't exist in the map, or the function `f` returned None.
    pub fn fetch_update<Q: ?Sized, F>(&self, key: &Q, mut f: F) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(V) -> Option<V>,
    {
        self._fetch_update(key, |current| {
            f(current.value).map(|value| Entry {
                key_hash: current.key_hash,
                key_offset: current.key_offset,
                value,
            })
        })
        .map(|previous| previous.value)
    }

    #[inline]
    fn _fetch_update<Q: ?Sized, F>(&self, key: &Q, f: F) -> Option<Entry<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(Entry<V>) -> Option<Entry<V>>,
    {
        let entry = self._find_entry(key)?;

        entry
            .fetch_update(Ordering::Release, Ordering::Acquire, f)
            .ok()
    }

    fn _find_entry<Q: ?Sized>(&self, key: &Q) -> Option<&Atomic<Entry<V>>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let (key_hash, mut index) = self._hash_and_index(key);

        let buckets = self._bucket_slice();

        for _ in 0..self.size_mask {
            let entry = get_entry(buckets, index, self.size_mask);
            match entry.load(Ordering::Relaxed) {
                Entry {
                    key_offset: constants::EMPTY_KEY,
                    ..
                } => return None,
                Entry {
                    key_offset,
                    key_hash: entry_hash,
                    ..
                } if key_hash == entry_hash => {
                    let key_offset = key_offset - constants::MIN_KEY;
                    if let Some(existing_key) = self.key_store.get(key_offset as usize) {
                        if key == existing_key.borrow() {
                            return Some(entry);
                        }
                    }
                }
                _ => {}
            }
            index = self._next_index(index);
        }
        unreachable!("There cannot be 0 empty entries, because the usable capacity is less than the allocated capacity.")
    }

    fn _find_empty_entry<Q: ?Sized>(&self, key: &Q) -> Option<&Atomic<Entry<V>>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let (key_hash, mut index) = self._hash_and_index(key);

        let buckets = self._bucket_slice();

        for _ in 0..self.size_mask {
            let entry = get_entry(buckets, index, self.size_mask);
            match entry.load(Ordering::Relaxed) {
                Entry {
                    key_offset: constants::EMPTY_KEY,
                    ..
                } => return Some(entry),
                Entry {
                    key_offset,
                    key_hash: entry_hash,
                    ..
                } if key_hash == entry_hash => {
                    let key_offset = key_offset - constants::MIN_KEY;
                    if let Some(existing_key) = self.key_store.get(key_offset as usize) {
                        if key == existing_key.borrow() {
                            return None;
                        }
                    }
                }
                _ => {}
            }
            index = self._next_index(index);
        }
        unreachable!("There cannot be 0 empty entries, because the usable capacity is less than the allocated capacity.")
    }

    fn _bucket_slice(&self) -> &[Bucket<V>] {
        unsafe { &*slice_from_raw_parts(self.table, self.size_mask as usize + 1) }
    }

    fn _next_index(&self, index: Size) -> Size {
        crate::wrap!(<Size>: index as usize + 1, self.size_mask as usize + 1)
    }

    /// Hash the key, returning a value of type [`HashT`].
    #[inline]
    fn _hash<Q: ?Sized>(&self, key: &Q) -> HashT
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        util::hash::<_, FnvHasher>(key)
    }

    /// Hash the key, and derive the table index from the hash.
    /// Return (hash, index).
    #[inline]
    fn _hash_and_index<Q: ?Sized>(&self, key: &Q) -> (HashT, Size)
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let hash = self._hash(key);
        (
            hash,
            crate::wrap!(<Size>: hash, self.size_mask as usize + 1),
        )
    }
}

impl<K: Hash + Eq, V: Copy + NoUninit> Drop for HashMap<K, V> {
    fn drop(&mut self) {
        util::deallocate(self.table, self.size_mask as usize + 1);
    }
}

impl<K: Hash + Eq, V: Copy + NoUninit> Default for HashMap<K, V> {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

unsafe impl<K: Hash + Eq, V: Copy + NoUninit> Send for HashMap<K, V> {}
unsafe impl<K: Hash + Eq, V: Copy + NoUninit> Sync for HashMap<K, V> {}

mod constants {
    use super::Size;

    pub const EMPTY_KEY: Size = 0;
    pub const MIN_KEY: Size = 1;
}

fn create_table<V>(capacity: usize) -> *mut Bucket<V> {
    let bucket_count = capacity >> BUCKET_CAPACITY.ilog2();
    util::allocate_zeroed::<Bucket<V>>(bucket_count)
}

const fn get_bucket_index(index: Size, size_mask: Size) -> Size {
    (index & size_mask) >> BUCKET_CAPACITY.ilog2()
}

const fn get_entry_index(index: Size) -> Size {
    index & (BUCKET_CAPACITY - 1)
}

const fn get_entry<V>(buckets: &[Bucket<V>], index: Size, size_mask: Size) -> &Atomic<Entry<V>> {
    let bucket_index = get_bucket_index(index, size_mask);
    let entry_index = get_entry_index(index);
    &buckets[bucket_index as usize].entries[entry_index as usize]
}

struct Bucket<V> {
    entries: [Atomic<Entry<V>>; BUCKET_CAPACITY as usize],
}

#[derive(Clone, Copy)]
// align(8) is necessary to enable the use of single-instruction atomic operations.
#[repr(align(8))]
struct Entry<V> {
    key_hash: u32,
    key_offset: Size,
    value: V,
}

unsafe impl<V: Copy + NoUninit> NoUninit for Entry<V> {}

impl<V> Entry<V> {
    pub const EMPTY: Self = unsafe { core::mem::zeroed() };
}
