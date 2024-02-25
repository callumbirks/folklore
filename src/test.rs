use crate::Map;
use alloc::{string::String, format};
use fixedstr::zstr;

fn traits_check<T: Sized + Send + Sync + Unpin + Default>() {}

#[test]
fn correct_traits() {
    traits_check::<Map<String, u16>>();
}

#[test]
fn insert_get_one() {
    let map: Map<zstr<17>, u16> = Map::new(128);
    let key: zstr<17> = zstr::make("Answer");
    assert!(map.insert(key, 42));
    assert_eq!(map.get(&key), Some(42));
}

#[test]
fn insert_remove_one() {
    let map: Map<zstr<17>, u16> = Map::new(128);
    let key: zstr<17> = zstr::make("Answer");
    assert!(map.insert(key, 42));
    map.remove(&key);
    assert_eq!(map.get(&key), None);
}

#[test]
fn insert_duplicate() {
    let map: Map<zstr<17>, u16> = Map::new(128);
    let key: zstr<17> = zstr::make("Answer");
    assert!(map.insert(key, 42));
    assert!(!map.insert(key, 76));
}

#[test]
fn insert_duplicate_removed() {
    let map: Map<zstr<17>, u16> = Map::new(128);
    let key: zstr<17> = zstr::make("Answer");
    assert!(map.insert(key, 42));
    map.remove(&key);
    assert!(map.insert(key, 76));
}

#[test]
fn max_capacity() {
    let map: Map<zstr<17>, u16> = Map::new(128);
    #[allow(clippy::cast_possible_truncation)]
    // Capacity may be slightly more than the requested capacity
    let capacity = map.capacity as u16;
    for i in 0..capacity {
        let f = format!("Answer{i}");
        let key: zstr<17> = zstr::make(f.as_str());
        assert!(map.insert(key, i));
    }
    assert!(!map.insert(zstr::make("Overflow"), 77));
}