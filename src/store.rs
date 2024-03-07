use core::{
    borrow::Borrow, cell::UnsafeCell, fmt::Debug, hash::Hash, hint::unreachable_unchecked,
    mem::MaybeUninit, sync::atomic::AtomicUsize,
};

use atomic::{Atomic, Ordering};
use bytemuck::NoUninit;
use hash32::FnvHasher;

use crate::{util, BUCKET_CAPACITY};

#[repr(u8)]
#[repr(align(8))]
#[derive(Clone, Copy)]
enum HashState {
    Empty = 0,
    Deleted,
    Inserting(u32),
    Occupied(u32),
}

unsafe impl NoUninit for HashState {}

struct Cell<T> {
    state: Atomic<HashState>,
    value: UnsafeCell<MaybeUninit<T>>,
}

struct Bucket<T> {
    cells: [Cell<T>; BUCKET_CAPACITY],
}

pub struct HashStore<T>
where
    T: Hash + Eq + Clone,
{
    buckets: *mut Bucket<T>,
    capacity: usize,
    count: AtomicUsize,
}

impl<T> HashStore<T>
where
    T: Hash + Eq + Clone,
{
    pub fn with_capacity(capacity: usize) -> Self {
        let bucket_count = capacity >> 4_usize.ilog2();
        // Buckets are all allocated as zero, so cells will be HashState::Empty
        let bucket_ptr = util::allocate_zeroed::<Bucket<T>>(bucket_count);
        Self {
            buckets: bucket_ptr,
            capacity,
            count: AtomicUsize::new(0),
        }
    }

    pub fn insert<const CHECK_UNIQUE: bool>(&self, value: T) -> Option<usize> {
        if CHECK_UNIQUE && self.find(&value).is_some() {
            return None;
        }

        if self.count.load(Ordering::Relaxed) >= self.effective_capacity() {
            return None;
        }

        let index_hash = util::hash::<T, FnvHasher>(&value);
        let mut index = crate::wrap_index(index_hash, self.effective_capacity());

        let buckets = self.buckets_slice();

        loop {
            let cell = self.get_cell(buckets, index);

            match cell
                .state
                .fetch_update(
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                    |current| match current {
                        HashState::Empty | HashState::Deleted => {
                            Some(HashState::Inserting(index_hash))
                        }
                        HashState::Inserting(_) | HashState::Occupied(_) => None,
                    },
                ) {
                Ok(_) => {
                    // Cell is empty or deleted; insert the value, set the state, increment count
                    let p = cell.value.get();
                    unsafe { p.write(MaybeUninit::new(value)) };
                    cell.state
                        .store(HashState::Occupied(index_hash), Ordering::SeqCst);
                    self.count.fetch_add(1, Ordering::Relaxed);
                    return Some(index);
                }
                Err(previous) => match previous {
                    HashState::Occupied(existing_hash) | HashState::Inserting(existing_hash) => {
                        // The hash of the value we're inserting is identical to an existing
                        // hash
                        if index_hash == existing_hash {
                            // Validate the values are actually equivalent, incase of hash collision
                            let existing_value = unsafe { (*cell.value.get()).assume_init_ref() };
                            if &value == existing_value {
                                return None;
                            }
                        }
                    }
                    _ => unsafe { unreachable_unchecked() },
                },
            }

            // If the cell was occupied but the hashes and/or values weren't equivalent, this is a collision.
            // Try the next cell.
            index = crate::wrap!(<usize>: index + 1, self.effective_capacity());
        }
    }

    pub fn get(&self, index: usize) -> Option<T> {
        let buckets = self.buckets_slice();
        let cell = self.get_cell(buckets, index);

        loop {
            match cell.state.load(Ordering::SeqCst) {
                HashState::Occupied(_) => {
                    return Some(unsafe { (*cell.value.get()).assume_init_read() })
                }
                HashState::Inserting(_) => {}
                _ => return None,
            }
        }
    }

    pub fn find<U: ?Sized>(&self, value: &U) -> Option<usize>
    where
        T: Borrow<U>,
        U: Hash + Eq,
    {
        let index_hash = util::hash::<U, FnvHasher>(value);
        let mut index = crate::wrap_index(index_hash, self.effective_capacity());
        let buckets = self.buckets_slice();

        loop {
            let cell = self.get_cell(buckets, index);
            match cell.state.load(Ordering::SeqCst) {
                HashState::Occupied(existing_hash) => {
                    if index_hash == existing_hash {
                        let existing = unsafe { (*cell.value.get()).assume_init_ref() };
                        if existing.borrow() == value {
                            return Some(index);
                        }
                    }
                }
                HashState::Inserting(existing_hash) => {
                    if index_hash == existing_hash {
                        continue;
                    }
                }
                HashState::Empty => return None,
                HashState::Deleted => {}
            }
            index = crate::wrap!(<usize>: index, self.effective_capacity());
        }
    }

    pub fn values_eq(&self, index1: usize, index2: usize) -> bool {
        let buckets = self.buckets_slice();
        let cell1 = self.get_cell(buckets, index1);
        let cell2 = self.get_cell(buckets, index2);

        loop {
            let state1 = cell1.state.load(Ordering::SeqCst);
            let state2 = cell2.state.load(Ordering::SeqCst);
            match (state1, state2) {
                (HashState::Occupied(hash1), HashState::Occupied(hash2)) => {
                    if hash1 != hash2 {
                        return false;
                    }
                    let value1 = unsafe { (*cell1.value.get()).assume_init_ref() };
                    let value2 = unsafe { (*cell2.value.get()).assume_init_ref() };
                    return value1 == value2;
                }
                // Inserting means another thread is currently inserting the value and it is
                // unsafe to access.
                (
                    HashState::Inserting(hash1) | HashState::Occupied(hash1),
                    HashState::Inserting(hash2) | HashState::Occupied(hash2),
                ) => {
                    // If the hashes are not equal, we can safely return.
                    if hash1 != hash2 {
                        return false;
                    }
                    // If the hashes are equal, the values might be, so spinlock
                    continue;
                }
                // One or both of the values was empty or deleted
                _ => {
                    return false;
                }
            }
        }
    }

    pub fn remove(&self, index: usize) -> bool {
        let buckets = self.buckets_slice();
        let cell = self.get_cell(buckets, index);

        loop {
            match cell
                .state
                .fetch_update(
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                    |current| match current {
                        HashState::Occupied(_) => Some(HashState::Deleted),
                        _ => None,
                    },
                ) {
                Ok(_) => {
                    self.count.fetch_sub(1, Ordering::Relaxed);
                    return true;
                }
                Err(previous) => match previous {
                    // spinlock if another thread is currently updating this
                    HashState::Inserting(_) => continue,
                    _ => return false,
                },
            }
        }
    }

    fn get_cell<'a>(&'a self, buckets: &'a [Bucket<T>], index: usize) -> &'a Cell<T> {
        let bucket_index = util::get_bucket_index(index, self.size_mask());
        let bucket = &buckets[bucket_index];
        let item_index = util::get_cell_index(index);
        &bucket.cells[item_index as usize]
    }

    fn buckets_slice(&self) -> &[Bucket<T>] {
        unsafe { core::slice::from_raw_parts(self.buckets, self.capacity) }
    }

    fn size_mask(&self) -> usize {
        self.capacity - 1
    }

    fn effective_capacity(&self) -> usize {
        self.capacity - crate::constants::MIN_KEY as usize
    }
}

impl<T: Hash + Eq + Clone> Drop for HashStore<T> {
    fn drop(&mut self) {
        util::deallocate(self.buckets, self.capacity);
    }
}

impl<T: Hash + Eq + Clone> Debug for HashStore<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HashStore")
            .field("buckets", &self.buckets)
            .field("capacity", &self.capacity)
            .field("count", &self.count)
            .finish()
    }
}
