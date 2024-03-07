use fixedstr::zstr;
use folklore::nano::HashMap;

fn traits_check<T: Sized + Send + Sync + Unpin + Default>() {}

#[test]
fn correct_traits() {
    traits_check::<HashMap<String, u16>>();
}

#[test]
fn insert_get_one() {
    let map = HashMap::with_capacity(64);
    assert!(map.insert("test123".to_owned(), 26_u16));
    assert_eq!(map.get("test123"), Some(26_u16));
}

#[test]
fn insert_get_max() {
    let map = HashMap::<zstr<17>, u16>::with_capacity(i16::MAX as usize);
    for i in 0..(i16::MAX as u16) {
        let key = zstr::make(&format!("{i}test_test{i}"));
        assert!(map.insert(key, i));
        assert!(map.get(&key).is_some());
    }
    let key = zstr::make("test_overflow");
    assert!(!map.insert(key, 2077));
}

#[test]
fn insert_update_one() {
    let map = HashMap::with_capacity(64);
    assert!(map.insert("test123".to_owned(), 26_u16));
    assert_eq!(map.get("test123"), Some(26));
    assert_eq!(map.update("test123", 47), Some(26));
    assert_eq!(map.get("test123"), Some(47));
}

#[test]
fn insert_update_many() {
    let map = HashMap::<zstr<17>, u16>::with_capacity(i16::MAX as usize);
    for i in 0..(i16::MAX as u16) {
        let key = zstr::make(&format!("{i}test_test{i}"));
        assert!(map.insert(key, i));
        assert_eq!(map.get(&key), Some(i));
        assert_eq!(map.update(&key, i16::MAX as u16 - i), Some(i));
        assert_eq!(map.get(&key), Some(i16::MAX as u16 - i));
    }
    let key = zstr::make("test_overflow");
    assert!(!map.insert(key, 42));
}