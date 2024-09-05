use crate::HashMap;
use alloc::{
    format,
    string::{String, ToString},
};
use fixedstr::zstr;
use rayon::prelude::*;

fn traits_check<T: Sized + Send + Sync + Unpin + Default>() {}

#[test]
fn correct_traits() {
    traits_check::<HashMap<String, u16>>();
}

#[test]
fn insert_get_one() {
    let map: HashMap<zstr<17>, u16> = HashMap::default();
    let key: zstr<17> = zstr::make("Answer");
    assert!(map.insert(key, 42));
    assert_eq!(map.get(&key), Some(42));
}

#[test]
fn insert_update_one() {
    let map: HashMap<zstr<17>, u16> = HashMap::default();
    let key: zstr<17> = zstr::make("Answer");
    assert!(map.insert(key, 42));
    map.update(&key, 76);
    assert_eq!(map.get(&key), Some(76));
}

#[test]
fn insert_duplicate() {
    let map: HashMap<zstr<17>, u16> = HashMap::default();
    let key: zstr<17> = zstr::make("Answer");
    assert!(map.insert(key, 42));
    assert!(!map.insert(key, 76));
    assert!(map.contains_key(&key))
}

#[test]
fn full() {
    let map: HashMap<String, u16> = HashMap::default();
    let capacity = map.capacity;
    for i in 0..capacity {
        let key = format!("Answer{}", i);
        assert!(map.insert(key.clone(), i));
        assert_eq!(map.get(&key), Some(i));
    }
    assert!(!map.insert("Overflow".to_string(), 77));
}

#[test]
fn max_capacity() {
    let map: HashMap<zstr<17>, u16> = HashMap::with_capacity(i16::MAX as usize);
    #[allow(clippy::cast_possible_truncation)]
    let capacity = map.capacity;
    for i in 0..capacity {
        let f = format!("Answer{i}");
        let key: zstr<17> = zstr::make(f.as_str());
        assert!(map.insert(key, i));
    }
    assert!(!map.insert(zstr::make("Overflow"), 77));
}

#[test]
#[should_panic(expected = "assertion failed: i16::try_from(capacity).is_ok()")]
fn over_capacity() {
    let _ = HashMap::<u64, u16>::with_capacity(i16::MAX as usize + 1);
}

#[test]
fn iter() {
    let map = HashMap::<u64, u16>::with_capacity(100);
    for i in 0..100 {
        assert!(map.insert(i as u64, i));
    }

    for (i, (k, v)) in map.iter().enumerate() {
        assert_eq!(*k, i as u64);
        assert_eq!(*k, v as u64);
    }
}
