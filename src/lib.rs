#![no_std]

extern crate alloc;

mod array;
#[cfg(test)]
mod test;
mod util;

use crate::array::ConcurrentArray;
use atomic::Atomic;
use bytemuck::NoUninit;
use core::borrow::Borrow;
use core::fmt::Debug;
use core::hash::{Hash, Hasher};
use core::mem::size_of;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{Relaxed, SeqCst};
// The same Hasher used by std::collections::HashMap (in Rust 1.76.0)
use siphasher::sip::SipHasher13;

const LOAD_FACTOR: f64 = 0.6;
const BUCKET_CAPACITY: usize = 4;

type OffsetT = u16;

mod constants {
    use crate::OffsetT;

    pub const EMPTY_KEY: OffsetT = 0;
    pub const DELETED_KEY: OffsetT = 1;
    pub const MIN_KEY: OffsetT = 2;
}

/// A lock-less concurrent hash map.
/// Has a maximum capacity of `isize::MAX`.
/// Values must be 2 bytes each.
/// Values are immutable once inserted.
pub struct Map<K, V>
where
    K: Eq + Hash,
    V: NoUninit,
{
    table: *mut Table<V>,
    keys: ConcurrentArray<K>,
    count: AtomicUsize,
    capacity: usize,
}

impl<K, V> Map<K, V>
where
    K: Eq + Hash,
    V: NoUninit + Eq,
{
    /// Creates a new [`Map<K, V>`] which can store `capacity` amount of K/V.
    ///
    /// # Panics
    /// Panics if capacity is greater than `i16::MAX`.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        // This assertion is only ran at compile time
        generic_asserts!((V);
            VALUE_SIZE: size_of::<V>() == size_of::<OffsetT>();
        );
        // Panic if capacity > i16::MAX
        assert!(i16::try_from(capacity).is_ok());
        #[allow(clippy::cast_sign_loss)]
        #[allow(clippy::cast_possible_truncation)]
        #[allow(clippy::cast_precision_loss)]
        // The allocated size of the table is larger than the capacity to allow for LOAD_FACTOR,
        // which improves performance. This also means there will always be empty cells, which
        // means the unconditional loops in get/insert will never be infinite.
        let table_size = ((capacity as f64 / LOAD_FACTOR) as usize).next_power_of_two();
        // Ensure the highest possible offset won't overflow
        debug_assert!(table_size - 1 <= OffsetT::MAX as usize);
        #[allow(clippy::cast_sign_loss)]
        #[allow(clippy::cast_possible_truncation)]
        #[allow(clippy::cast_precision_loss)]
        let capacity = (table_size as f64 * LOAD_FACTOR) as usize;
        let table = Self::create_table(table_size);
        Self {
            table,
            keys: ConcurrentArray::new(capacity),
            count: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Insert a key-value pair into the map. Returns a bool representing success.
    /// Will succeed unless the key is already in the map or the map is at max capacity.
    pub fn insert(&mut self, key: K, value: V) -> bool {
        if self.count.load(Relaxed) >= self.capacity {
            return false;
        }
        // The index of the cell is the key's hash (wrapped to fit)
        let hash = hash::<K, SipHasher13>(&key);
        let mut index = wrap_index(hash, self.capacity);

        // Allocate a copy of the key in the keys array
        let Some((alloc_key, new_key_offset)) = self.keys.push(key) else {
            return false;
        };

        let table = unsafe { &*self.table };
        let buckets = table.bucket_slice();
        let size_mask = table.size_mask;

        #[allow(clippy::cast_possible_truncation)]
        // Unconditional loop is OK, because there are always spare empty buckets allocated.
        loop {
            let cell = get_cell(buckets, index, size_mask);
            let mut existing_key_offset: OffsetT = 0;
            // Try to write the key offset to the cell at index
            if cell
                .fetch_update(SeqCst, SeqCst, |current| match current.key_offset {
                    // If the cell is empty or a tombstone, just write
                    constants::EMPTY_KEY | constants::DELETED_KEY => Some(Cell {
                        key_offset: new_key_offset as OffsetT + constants::MIN_KEY,
                        value,
                    }),
                    // If the cell is in use...
                    _ => {
                        existing_key_offset = current.key_offset - constants::MIN_KEY;
                        None
                    }
                })
                .is_ok()
            {
                // Increment count
                self.count.fetch_add(1, SeqCst);
                return true;
            }
            // Get the key associated with the occupied cell
            let existing_key = self.keys.get(existing_key_offset as usize);
            if existing_key.is_some_and(|existing| existing == alloc_key) {
                // If the key already exists in the map, revert our efforts
                self.keys.remove(new_key_offset);
                return false;
            }
            // If the cell we tried was already in use, and its associated key is not equivalent to
            // key, this is a collision, so try the next cell.
            index = wrap_index(index as u64 + 1, self.capacity);
        }
    }

    /// Update a value, only if it matches the expected value. Returns a Result containing the
    /// previous value.
    /// # Errors
    /// Returns an error if the key does not exist in the map, or if the value did not match the
    /// expected value.
    pub fn compare_exchange<Q: ?Sized>(&mut self, key: &Q, expected: V, new_val: V) -> Result<V, V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let table = unsafe { &*self.table };
        let buckets = table.bucket_slice();
        let size_mask = table.size_mask;

        let hash = hash::<Q, SipHasher13>(key);
        let mut index = wrap_index(hash, self.capacity);

        loop {
            let cell = get_cell(buckets, index, size_mask);
            if let Ok(previous) =
                cell.fetch_update(SeqCst, SeqCst, |current| match current.key_offset {
                    constants::EMPTY_KEY => Some(current),
                    constants::DELETED_KEY => None,
                    _ => {
                        let key_offset = current.key_offset - constants::MIN_KEY;
                        let found_key = self.keys.get(key_offset as usize)?;
                        if found_key.borrow() == key {
                            if current.value == expected {
                                // Key matches, value matches, update to new value
                                Some(Cell {
                                    key_offset: current.key_offset,
                                    value: new_val,
                                })
                            } else {
                                // Key matches, value doesn't, return without updating
                                Some(current)
                            }
                        } else {
                            // Key doesn't match, try the next cell incase of collision
                            None
                        }
                    }
                })
            {
                // If fetch_update was Ok, we either; updated successfully, or...
                if previous.value == expected {
                    return Ok(previous.value);
                }
                // Didn't, because the value didn't match expected
                return Err(previous.value);
            }
            // If the `fetch_update` was not_ok, the cell was a potential collision, so try the
            // next cell
            index = wrap_index(index as u64 + 1, self.capacity);
        }
    }

    /// Check whether a key already exists in the map.
    /// As expensive as `get`.
    pub fn contains_key<Q: ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.get(key).is_some()
    }

    /// Get the value from the map associated with the given key.
    pub fn get<Q: ?Sized>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        let table = unsafe { self.table.as_ref()? };
        let buckets = table.bucket_slice();
        let size_mask = table.size_mask;

        let hash = hash::<Q, SipHasher13>(key);
        let mut index = wrap_index(hash, self.capacity);
        // Unconditional loop is OK, because there are always spare empty buckets allocated.
        loop {
            let cell = get_cell(buckets, index, size_mask);
            let cell_loaded = cell.load(Relaxed);
            match cell_loaded.key_offset {
                constants::EMPTY_KEY => {
                    // An empty key has never been inserted to, we can safely return.
                    return None;
                }
                constants::DELETED_KEY => {
                    // A deleted key has been written to before, so collisions could have happened.
                    // So we must keep searching.
                }
                _ => {
                    // Check the key associated with the cell we found
                    let key_offset = cell_loaded.key_offset - constants::MIN_KEY;
                    let found_key = self.keys.get(key_offset as usize)?;
                    if found_key.borrow() == key {
                        // The key associated with this cell matches!
                        return Some(cell_loaded.value);
                    }
                }
            }
            // The key at this cell was not equivalent, try the next cell in case of collisions
            index = wrap_index(index as u64 + 1, self.capacity);
        }
    }

    /// Remove the entry from the map which contains `key`.
    pub fn remove<Q: ?Sized>(&mut self, key: &Q)
    where
        K: Borrow<Q>,
        Q: Eq + Hash,
    {
        let Some(table) = (unsafe { self.table.as_ref() }) else {
            return;
        };
        let buckets = table.bucket_slice();
        let size_mask = table.size_mask;

        let hash = hash::<Q, SipHasher13>(key);
        let mut index = wrap_index(hash, self.capacity);

        // Unconditional loop is OK, because there are always spare empty buckets allocated.
        loop {
            let cell = get_cell(buckets, index, size_mask);
            if let Ok(previous) =
                cell.fetch_update(SeqCst, SeqCst, |current| match current.key_offset {
                    // Empty means `key` cannot exist in the map
                    constants::EMPTY_KEY => Some(current),
                    // Deleted, `key` might be in the map but had a collision, try the next cell
                    constants::DELETED_KEY => None,
                    _ => {
                        let key_offset = current.key_offset as usize - constants::MIN_KEY as usize;
                        // Fetch the associated key to check if it's equivalent to `key`
                        let found_key = self.keys.get(key_offset)?;
                        if found_key.borrow() == key {
                            // We found the key! Mark the cell as deleted
                            Some(Cell {
                                key_offset: constants::DELETED_KEY,
                                value: current.value,
                            })
                        // The associated key of this cell doesn't match `key`, try the next cell
                        } else {
                            None
                        }
                    }
                })
            // The happy paths out of the above are: Cell was empty, or cell was found and
            // removed. We can return in either of those cases
            {
                if previous.key_offset != constants::EMPTY_KEY {
                    // If it was not empty, we removed it, so decrement count
                    self.count.fetch_sub(1, SeqCst);
                }
                return;
            }
            // If the `fetch_update` was not_ok, the cell was a potential collision, so try the
            // next cell
            index = wrap_index(index as u64 + 1, self.capacity);
        }
    }

    fn create_table(capacity: usize) -> *mut Table<V> {
        let bucket_count = capacity >> BUCKET_CAPACITY.ilog2();
        let bucket_ptr = allocate_zeroed::<Bucket<V>>(bucket_count);
        let table_ptr = allocate::<Table<V>>(1);
        let table = unsafe { &mut *table_ptr };
        table.buckets = bucket_ptr;
        table.size_mask = capacity - 1;
        table_ptr
    }
}

impl<K, V> Drop for Map<K, V>
where
    K: Eq + Hash,
    V: NoUninit,
{
    fn drop(&mut self) {
        unsafe {
            if let Some(table) = self.table.as_mut() {
                let buckets = table.bucket_slice_mut();
                let bucket_count = buckets.len();
                let bucket_ptr = table.buckets;
                deallocate(bucket_ptr, bucket_count);
            }
            deallocate(self.table, 1);
        }
    }
}

impl<K, V> Default for Map<K, V>
where
    K: Eq + Hash,
    V: NoUninit + Eq,
{
    fn default() -> Self {
        Self::new(2048)
    }
}

impl<K, V> Debug for Map<K, V>
where
    K: Debug + Eq + Hash,
    V: Debug + NoUninit,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Map")
            .field("table", &self.table)
            .field("keys", &self.keys)
            .field("count", &self.count)
            .field("capacity", &self.capacity)
            .finish()
    }
}

unsafe impl<K: Eq + Hash, V: NoUninit> Send for Map<K, V> {}
unsafe impl<K: Eq + Hash, V: NoUninit> Sync for Map<K, V> {}

struct Table<V: NoUninit> {
    buckets: *mut Bucket<V>,
    size_mask: usize,
    _marker: core::marker::PhantomData<V>,
}

impl<V: NoUninit> Table<V> {
    pub fn bucket_slice(&self) -> &[Bucket<V>] {
        unsafe { core::slice::from_raw_parts(self.buckets, self.size()) }
    }

    pub fn bucket_slice_mut(&mut self) -> &mut [Bucket<V>] {
        unsafe { core::slice::from_raw_parts_mut(self.buckets, self.size()) }
    }

    pub fn size(&self) -> usize {
        self.size_mask + 1
    }
}

#[allow(clippy::cast_possible_truncation)]
fn wrap_index(index: u64, size: usize) -> usize {
    (index % size as u64) as usize
}

fn hash<T: ?Sized, H>(key: &T) -> u64
where
    T: Hash,
    H: Hasher + Default,
{
    let mut hasher = H::default();
    key.hash(&mut hasher);
    hasher.finish()
}

fn get_cell<V: NoUninit>(buckets: &[Bucket<V>], index: usize, size_mask: usize) -> &AtomicCell<V> {
    let bucket_index = get_bucket_index(index, size_mask);
    let cell_index = get_cell_index(index);
    &buckets[bucket_index].cells[cell_index]
}

const fn get_bucket_index(index: usize, size_mask: usize) -> usize {
    (index & size_mask) >> BUCKET_CAPACITY.ilog2()
}

const fn get_cell_index(index: usize) -> usize {
    index & (BUCKET_CAPACITY - 1)
}

struct Bucket<V: NoUninit> {
    cells: [AtomicCell<V>; BUCKET_CAPACITY],
}

#[repr(align(4))]
#[derive(Copy, Clone)]
struct Cell<V: NoUninit> {
    key_offset: OffsetT,
    value: V,
}

unsafe impl<V: NoUninit> NoUninit for Cell<V> {}

/// A key-value pair which can be accessed and mutated concurrently.
/// It does not suffer from torn reads or writes, because atomic operations are applied to the entire struct in one instruction.
type AtomicCell<V> = Atomic<Cell<V>>;

fn allocate<T>(count: usize) -> *mut T {
    let layout = core::alloc::Layout::array::<T>(count).unwrap();
    unsafe { alloc::alloc::alloc(layout).cast::<T>() }
}

fn allocate_zeroed<T>(count: usize) -> *mut T {
    let layout = core::alloc::Layout::array::<T>(count).unwrap();
    unsafe { alloc::alloc::alloc_zeroed(layout).cast::<T>() }
}

fn deallocate<T>(ptr: *mut T, count: usize) {
    let layout = core::alloc::Layout::array::<T>(count).unwrap();
    unsafe { alloc::alloc::dealloc(ptr.cast::<u8>(), layout) }
}
