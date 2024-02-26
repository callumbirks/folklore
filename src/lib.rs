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
    pub fn with_capacity(capacity: usize) -> Self {
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
    pub fn insert(&self, key: K, value: V) -> bool {
        if self.count.load(SeqCst) >= self.capacity {
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
        // Get just uses fetch_update but doesn't modify the cell
        self._fetch_update(Relaxed, Relaxed, key, Some)
            .ok()
            .map(|previous| previous.value)
    }

    /// Fetch the value with the given key and update the value using `f`.
    /// `f` should return `Some(new_value)` if it succeeds, or `None` if it fails.
    /// Returns a `Result` containing the previous value.
    ///
    /// # Errors
    /// This function will return an error if the key does not exist in the map, or if `f` failed
    /// (returned `None`).
    ///
    /// # Examples
    ///
    /// ```
    /// use folklore::Map;
    ///
    /// let mut map: Map<u64, u16> = Map::with_capacity(128);
    /// let key = 6446;
    /// map.insert(key, 42);
    /// // `f` will update the value to 76, only if it is currently 42
    /// let f = |current| {
    ///     if current == 42 {
    ///         Some(76)
    ///     } else {
    ///         None
    ///     }
    /// };
    /// let result = map.fetch_update(&key, f);
    /// let result2 = map.fetch_update(&key, f);
    /// assert_eq!(result, Ok(42));
    /// assert_eq!(result2, Err(76));
    /// assert_eq!(map.get(&key), Some(76));
    /// ```
    pub fn fetch_update<F, Q: ?Sized>(&self, key: &Q, mut f: F) -> Result<V, V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(V) -> Option<V>,
    {
        self._fetch_update(SeqCst, SeqCst, key, |current| {
            f(current.value).map(|value| Cell {
                key_offset: current.key_offset,
                value,
            })
        })
        .map(|previous| previous.value)
        .map_err(|previous| previous.value)
    }

    /// Remove the map entry associated with `key`, if it exists.
    pub fn remove<Q: ?Sized>(&self, key: &Q)
    where
        K: Borrow<Q>,
        Q: Eq + Hash,
    {
        if self
            ._fetch_update(SeqCst, SeqCst, key, |current| {
                Some(Cell {
                    key_offset: constants::DELETED_KEY,
                    value: current.value,
                })
            })
            .is_ok()
        {
            self.count.fetch_sub(1, SeqCst);
        }
    }

    /// Find the cell associated with `key` and update the cell using `f`.
    /// `f` should return an Option with either `None` if it does not want to update the cell
    /// (failed in some way), or `Ok(new_cell)` if it wants to update the cell.
    fn _fetch_update<Q: ?Sized, F>(
        &self,
        fetch_order: Ordering,
        update_order: Ordering,
        key: &Q,
        mut f: F,
    ) -> Result<Cell<V>, Cell<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(Cell<V>) -> Option<Cell<V>>,
    {
        let table = unsafe { &*self.table };
        let buckets = table.bucket_slice();
        let size_mask = table.size_mask;

        let hash = hash::<Q, SipHasher13>(key);
        let mut index = wrap_index(hash, self.capacity);

        let mut key_found = false;

        loop {
            let cell = get_cell(buckets, index, size_mask);
            let result = cell.fetch_update(fetch_order, update_order, |current| {
                match current.key_offset {
                    constants::EMPTY_KEY => Some(current),
                    constants::DELETED_KEY => None,
                    _ => {
                        let key_offset = current.key_offset - constants::MIN_KEY;
                        let found_key = self.keys.get(key_offset as usize)?;
                        if found_key.borrow() == key {
                            key_found = true;
                            f(current)
                        } else {
                            None
                        }
                    }
                }
            });

            if key_found {
                // If we found the key, we can return the result of `f`.
                return result;
            }

            if let Ok(previous) = result {
                // If we didn't find the key, but `fetch_update` returned Ok, that means we found
                // an empty key.
                return Err(previous);
            }

            // If we didn't find the key and the fetch update was not OK, the cell was a potential collision, so try the
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
        Self::with_capacity(2048)
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
