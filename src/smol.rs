use atomic::Atomic;
use bytemuck::NoUninit;
use core::{hash::Hash, mem::size_of, sync::atomic::AtomicU16};

use crate::{array::ConcurrentArray, generic_asserts, util};

/// Doesn't grow, 2-byte values, allows removal

type Size = u16;
type AtomicSize = AtomicU16;

const BUCKET_CAPACITY: usize = 4;
const LOAD_FACTOR: f64 = 0.6;
const MIGRATION_LOAD: f64 = 0.5;

pub struct HashMap<K, V>
where
    K: Hash + Eq,
    V: Copy + NoUninit,
{
    key_store: ConcurrentArray<K>,
    table: *mut Bucket<V>,
    size_mask: Size,
    capacity: Size,
    count: AtomicSize,
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
            count: AtomicSize::new(0),
        }
    }
}

fn create_table<V>(capacity: usize) -> *mut Bucket<V> {
    let bucket_count = capacity >> BUCKET_CAPACITY.ilog2();
    util::allocate_zeroed::<Bucket<V>>(bucket_count)
}

struct Bucket<V> {
    entries: [Atomic<Entry<V>>; BUCKET_CAPACITY],
}

#[derive(Clone, Copy)]
struct Entry<V> {
    key_offset: Size,
    value: V,
}

unsafe impl<V: Copy + NoUninit> NoUninit for Entry<V> {}
