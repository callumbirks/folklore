extern crate std;
use std::thread;

use crate::Map;
use alloc::{format, string::String, sync::Arc, vec};
use fixedstr::zstr;
use rand::Rng;

//fn traits_check<T: Sized + Send + Sync + Unpin + Default>() {}
//
//#[test]
//fn correct_traits() {
//    traits_check::<Map<String, u16>>();
//}
//
//#[test]
//fn insert_get_one() {
//    let map: Map<zstr<17>, u16> = Map::with_capacity(128);
//    let key: zstr<17> = zstr::make("Answer");
//    assert!(map.insert(key, 42));
//    assert_eq!(map.get(&key), Some(42));
//}
//
//#[test]
//fn insert_remove_one() {
//    let map: Map<zstr<17>, u16> = Map::with_capacity(128);
//    let key: zstr<17> = zstr::make("Answer");
//    assert!(map.insert(key, 42));
//    map.remove(&key);
//    assert_eq!(map.get(&key), None);
//}
//
//#[test]
//fn insert_duplicate() {
//    let map: Map<zstr<17>, u16> = Map::with_capacity(128);
//    let key: zstr<17> = zstr::make("Answer");
//    assert!(map.insert(key, 42));
//    assert!(!map.insert(key, 76));
//}
//
//#[test]
//fn insert_duplicate_removed() {
//    let map: Map<zstr<17>, u16> = Map::with_capacity(128);
//    let key: zstr<17> = zstr::make("Answer");
//    assert!(map.insert(key, 42));
//    map.remove(&key);
//    assert!(map.insert(key, 76));
//}
//
//#[test]
//fn full() {
//    let map: Map<zstr<17>, u16> = Map::with_capacity(128);
//    #[allow(clippy::cast_possible_truncation)]
//    // Capacity may be slightly more than the requested capacity
//    let capacity = map.capacity as u16;
//    for i in 0..capacity {
//        let f = format!("Answer{i}");
//        let key: zstr<17> = zstr::make(f.as_str());
//        assert!(map.insert(key, i));
//    }
//    assert!(!map.insert(zstr::make("Overflow"), 77));
//}
//
//#[test]
//fn max_capacity() {
//    let map: Map<zstr<17>, u16> = Map::with_capacity(i16::MAX as usize);
//    #[allow(clippy::cast_possible_truncation)]
//    let capacity = map.capacity as u16;
//    for i in 0..capacity {
//        let f = format!("Answer{i}");
//        let key: zstr<17> = zstr::make(f.as_str());
//        assert!(map.insert(key, i));
//    }
//    assert!(!map.insert(zstr::make("Overflow"), 77));
//}
//
//#[test]
//#[allow(clippy::cast_possible_truncation)]
//fn insert_remove_many() {
//    let map: Map<u64, u16> = Map::with_capacity(i16::MAX as usize);
//    for _ in 0..1000 {
//        let mut rng = rand::thread_rng();
//        let mut bits: u64 = rng.gen();
//        let mut mask = 0u64;
//
//        for _ in 0..6 {
//            mask <<= 4;
//            mask |= bits & 0b0000_1111;
//            bits >>= 4;
//
//            for i in 0..1000 {
//                let key: u64 = rng.gen::<u64>() & mask;
//                map.insert(key, i);
//                let key: u64 = rng.gen::<u64>() & mask;
//                map.remove(&key);
//            }
//        }
//    }
//}
//
//const THREAD_COUNT: usize = 8;
//const NUM_KEYS: usize = 1 << 14;
//const NUM_OPS: u64 = i16::MAX as u64;
//
//#[test]
//#[allow(clippy::cast_possible_truncation)]
//fn parallel_insert_remove() {
//    let map = Arc::new(crate::Map::with_capacity(NUM_KEYS));
//    let mut handles = vec![];
//    for _ in 0..THREAD_COUNT {
//        let map = map.clone();
//        handles.push(thread::spawn(move || {
//            for _ in 0..100 {
//                let mut rng = rand::thread_rng();
//                let mut bits: u64 = rng.gen();
//                let mut mask = 0u64;
//
//                for _ in 0..6 {
//                    mask <<= 4;
//                    mask |= bits & 0b0000_1111;
//                    bits >>= 4;
//
//                    for i in 0..NUM_OPS {
//                        let key: u64 = rng.gen::<u64>() & mask;
//                        map.insert(key, i as u16);
//                        let key: u64 = rng.gen::<u64>() & mask;
//                        map.remove(&key);
//                    }
//                }
//            }
//        }));
//    }
//    for h in handles {
//        h.join().unwrap();
//    }
//}
