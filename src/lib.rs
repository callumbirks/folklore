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
use hash32::FnvHasher;
use core::mem::size_of;
use core::sync::atomic::AtomicU16;

type Size = u16;
const DEFAULT_CAPACITY: usize = 2048;
const BUCKET_CAPACITY: Size = 4;
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
            key_store: ConcurrentArray::new(allocated_size),
            size_mask: (allocated_size - 1) as Size,
            capacity: capacity as Size,
            count: AtomicU16::new(0),
        }
    }

    pub fn insert(&self, key: K, value: V) -> bool {
        if self.count.load(Ordering::SeqCst) >= self.capacity {
            return false;
        }

        let index_hash = util::hash::<_, FnvHasher>(&key);
        let mut index = crate::wrap!(<Size>: index_hash, self.size_mask as usize + 1);

        let Some((new_key, new_key_index)) = self.key_store.push(key) else {
            return false;
        };

        #[allow(clippy::cast_possible_truncation)]
        let key_offset = new_key_index as Size + constants::MIN_KEY;

        let buckets = self.bucket_slice();

        let mut duplicate = false;

        loop {
            let entry = get_entry(buckets, index, self.size_mask);
            match entry.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                // `fetch_update` may call this func multiple times, so reset duplicate
                duplicate = false;
                match current.key_offset {
                    constants::EMPTY_KEY => Some(Entry { key_offset, value }),
                    existing => {
                        if let Some(existing_key) = self.key_store.get(existing as usize) {
                            if new_key == existing_key {
                                duplicate = true;
                                None
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                }
            }) {
                Ok(_) => {
                    self.count.fetch_add(1, Ordering::SeqCst);
                    return true;
                }
                Err(_) => {
                    if duplicate {
                        // Most of the time this should remove successfully, unless another thread
                        // inserted a key after we did.
                        self.key_store.remove(new_key_index);
                        return false;
                    }
                    // If `fetch_update` failed but key wasn't duplicate, this was a collision
                }
            }
            index = crate::wrap!(<Size>: index + 1, self.size_mask as usize + 1);
        }
    }

    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self._fetch_update(key, Some).ok().map(|v| v.value)
    }

    pub fn update<Q: ?Sized>(&self, key: &Q, value: V) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self._fetch_update(key, |current| {
            Some(Entry {
                key_offset: current.key_offset,
                value,
            })
        })
        .ok()
        .map(|previous| previous.value)
    }

    ///
    /// # Errors
    /// If the key doesn't exist in the map, or the function `f` returned None.
    pub fn fetch_update<Q: ?Sized, F>(&self, key: &Q, mut f: F) -> Result<V, Option<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(V) -> Option<V>,
    {
        self._fetch_update(key, |current| {
            f(current.value).map(|value| Entry {
                key_offset: current.key_offset,
                value,
            })
        })
        .map(|previous| previous.value)
        .map_err(|previous| match previous.key_offset {
            constants::EMPTY_KEY => None,
            _ => Some(previous.value),
        })
    }

    fn _fetch_update<Q: ?Sized, F>(&self, key: &Q, mut f: F) -> Result<Entry<V>, Entry<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(Entry<V>) -> Option<Entry<V>>,
    {
        let index_hash = util::hash::<_, FnvHasher>(key);
        let mut index = crate::wrap!(<Size>: index_hash, self.size_mask as usize + 1);

        let buckets = self.bucket_slice();

        loop {
            let entry = get_entry(buckets, index, self.size_mask);
            match entry.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| match current
                .key_offset
            {
                constants::EMPTY_KEY => None,
                existing => {
                    let existing_offset = existing - constants::MIN_KEY;
                    let Some(existing_key) = self.key_store.get(existing_offset as usize) else {
                        return None;
                    };
                    if key == existing_key.borrow() {
                        f(current)
                    } else {
                        None
                    }
                }
            }) {
                Ok(previous) => return Ok(previous),
                Err(previous) => {
                    if matches!(previous.key_offset, constants::EMPTY_KEY) {
                        return Err(previous);
                    }
                }
            }
            index = crate::wrap!(<Size>: index + 1, self.size_mask as usize + 1);
        }
    }

    fn bucket_slice(&self) -> &[Bucket<V>] {
        unsafe { core::slice::from_raw_parts(self.table, self.size_mask as usize + 1) }
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
#[repr(align(4))]
struct Entry<V> {
    key_offset: Size,
    value: V,
}

unsafe impl<V: Copy + NoUninit> NoUninit for Entry<V> {}
