#![no_std]

extern crate alloc;

mod array;
pub mod nano;
mod store;
#[cfg(test)]
mod test;
mod util;
mod smol;

use crate::store::HashStore;
use atomic::{Atomic, Ordering};
use bytemuck::NoUninit;
use core::borrow::Borrow;
use core::fmt::Debug;
use core::hash::Hash;
use core::mem::size_of;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::SeqCst;
use hash32::FnvHasher;

/// This library provides 3 different concurrent hashmaps.
/// `nano` is the simplest. It can't be grown, only allows 2-byte values, and does not allow removal.
/// `nano` is 100% lock-free.
/// `smol` is the next. It can't be grown, only allows 2-byte values, but does allow removal.
/// `smol` uses some small spinlocks when the table is migrated to clean up tombstones.
/// `medi` is the most complex. It can be grown, allows any-size values, and allows removals.
/// It uses spinlocks when migrating for growth or tombstone clean-up, and for value updating.
/// 
/// The reason `nano` and `smol` are different, while providing very similar functionality, is the
/// complexity of removals. In 'Concurrent Hash Maps: Fast and General', Maier et al. find that the 
/// initial `folklore` implementation suffers huge performance penalties after many removals, 
/// because tombstones fill up the table. The solution is to migrate to a new table when reaching a
/// certain number of tombstones, replacing would-be tombstones with empty cells (and shifting
/// elements around)

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
    K: Eq + Hash + Clone,
    V: NoUninit,
{
    table: *mut Table<V>,
    keys: HashStore<K>,
    count: AtomicUsize,
    capacity: usize,
}

impl<K, V> Map<K, V>
where
    K: Eq + Hash + Clone,
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
        let allocated_size = ((capacity as f64 / LOAD_FACTOR) as usize).next_power_of_two();
        // Ensure the highest possible offset won't overflow
        debug_assert!(allocated_size - 1 <= OffsetT::MAX as usize);
        let table = Self::create_table(allocated_size);
        Self {
            table,
            keys: HashStore::with_capacity(allocated_size),
            count: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Insert a key-value pair into the map. Returns a bool representing success.
    /// Will succeed unless the key is already in the map or the map is at max capacity.
    pub fn insert(&self, key: K, value: V) -> bool {
        if self.count.load(Ordering::Relaxed) >= self.capacity {
            return false;
        }

        let table = unsafe { &*self.table };
        let buckets = table.bucket_slice();
        let size_mask = table.size_mask;
        let allocated_size = size_mask + 1;

        // The index of the cell is the key's hash (wrapped to fit)
        let hash = util::hash::<K, FnvHasher>(&key);
        let mut index = wrap_index(hash, allocated_size);

        // Allocate a copy of the key in the keys array
        let Some(new_key_offset) = self.keys.insert::<true>(key) else {
            return false;
        };

        #[allow(clippy::cast_possible_truncation)]
        // Unconditional loop is OK, because there are always spare empty buckets allocated.
        loop {
            let cell = get_cell(buckets, index, size_mask);
            // Try to write the key offset to the cell at index
            if cell
                .fetch_update(SeqCst, SeqCst, |current| match current.key_offset {
                    // If the cell is empty or a tombstone, just write
                    constants::EMPTY_KEY | constants::DELETED_KEY => Some(Cell {
                        key_offset: new_key_offset as OffsetT + constants::MIN_KEY,
                        value,
                    }),
                    // If the cell is in use, don't write
                    _ => None,
                })
                .is_ok()
            {
                // cell was updated, increment count and return success.
                self.count.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            // If the cell we tried was already in use, and its associated key is not equivalent to
            // key, this is a collision, so try the next cell.
            index = wrap_index(index as u32 + 1, allocated_size);
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
        self._fetch_update(key, Some)
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
    /// ```
    pub fn fetch_update<F, Q: ?Sized>(&self, key: &Q, mut f: F) -> Result<V, Option<V>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(V) -> Option<V>,
    {
        self._fetch_update(key, |current| {
            f(current.value).map(|value| Cell {
                key_offset: current.key_offset,
                value,
            })
        })
        .map(|previous| previous.value)
        .map_err(|previous| previous.map(|v| v.value))
    }

    /// Remove the map entry associated with `key`, if it exists.
    pub fn remove<Q: ?Sized>(&self, key: &Q)
    where
        K: Borrow<Q>,
        Q: Eq + Hash,
    {
        if let Ok(previous) = self._fetch_update(key, |current| {
            Some(Cell {
                key_offset: constants::DELETED_KEY,
                value: current.value,
            })
        }) {
            let key_offset = previous.key_offset - constants::MIN_KEY;
            self.keys.remove(key_offset as usize);
            self.count.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Find the cell associated with `key` and update the cell using `f`.
    /// `f` should return an Option with either `None` if it does not want to update the cell
    /// (failed in some way), or `Ok(new_cell)` if it wants to update the cell.
    fn _fetch_update<Q: ?Sized, F>(&self, key: &Q, mut f: F) -> Result<Cell<V>, Option<Cell<V>>>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
        F: FnMut(Cell<V>) -> Option<Cell<V>>,
    {
        let table = unsafe { &*self.table };
        let buckets = table.bucket_slice();
        let size_mask = table.size_mask;
        let allocated_size = size_mask + 1;

        let hash = util::hash::<Q, FnvHasher>(key);
        let mut index = wrap_index(hash, allocated_size);

        let Some(find_offset) = self.keys.find(key) else {
            return Err(None);
        };

        loop {
            let cell = get_cell(buckets, index, size_mask);
            let mut loaded = cell.load(SeqCst);
            loop {
                match loaded.key_offset {
                    constants::EMPTY_KEY => return Err(None),
                    constants::DELETED_KEY => {
                        break;
                    }
                    offset => {
                        let key_offset = offset - constants::MIN_KEY;
                        if key_offset as usize == find_offset {
                            let Some(f_res) = f(loaded) else {
                                return Err(Some(loaded));
                            };
                            match cell.compare_exchange(loaded, f_res, SeqCst, SeqCst) {
                                Ok(previous) => return Ok(previous),
                                Err(previous) => {
                                    loaded = previous;
                                    continue;
                                }
                            }
                        }
                        break;
                    }
                }
            }

            // If we didn't find the key and the fetch update was not OK, the cell was a potential collision, so try the
            // next cell
            index = crate::wrap!(<usize>: index, allocated_size);
        }
    }

    fn create_table(capacity: usize) -> *mut Table<V> {
        let bucket_count = capacity >> BUCKET_CAPACITY.ilog2();
        let bucket_ptr = util::allocate_zeroed::<Bucket<V>>(bucket_count);
        let table_ptr = util::allocate::<Table<V>>(1);
        let table = unsafe { &mut *table_ptr };
        table.buckets = bucket_ptr;
        table.size_mask = capacity - 1;
        table_ptr
    }
}

impl<K, V> Drop for Map<K, V>
where
    K: Eq + Hash + Clone,
    V: NoUninit,
{
    fn drop(&mut self) {
        unsafe {
            if let Some(table) = self.table.as_mut() {
                let buckets = table.bucket_slice_mut();
                let bucket_count = buckets.len();
                let bucket_ptr = table.buckets;
                util::deallocate(bucket_ptr, bucket_count);
            }
            util::deallocate(self.table, 1);
        }
    }
}

impl<K, V> Default for Map<K, V>
where
    K: Eq + Hash + Clone,
    V: NoUninit + Eq,
{
    fn default() -> Self {
        Self::with_capacity(2048)
    }
}

impl<K, V> Debug for Map<K, V>
where
    K: Debug + Eq + Hash + Clone,
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

unsafe impl<K: Eq + Hash + Clone, V: NoUninit> Send for Map<K, V> {}

unsafe impl<K: Eq + Hash + Clone, V: NoUninit> Sync for Map<K, V> {}

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
fn wrap_index(index: u32, size: usize) -> usize {
    (index % size as u32) as usize
}

fn get_cell<V: NoUninit>(buckets: &[Bucket<V>], index: usize, size_mask: usize) -> &AtomicCell<V> {
    let bucket_index = util::get_bucket_index(index, size_mask);
    let cell_index = util::get_cell_index(index);
    &buckets[bucket_index].cells[cell_index as usize]
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
