#![allow(clippy::pedantic)]

use criterion::Criterion;
use criterion::Throughput;
use criterion::{criterion_group, criterion_main};
use rand::{thread_rng, Rng};
use std::hash::BuildHasherDefault;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

const NUM_KEYS: usize = 1 << 14;
const NUM_OPS: u64 = i16::MAX as u64;
const THREAD_COUNT: usize = 8;

type HashFn = std::collections::hash_map::DefaultHasher;

fn bench_folklore_hashmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("folklore_hashmap");
    group.throughput(Throughput::Elements(
        NUM_OPS * 6 * 2_u64 * THREAD_COUNT as u64,
    ));
    group.sample_size(10);
    group.bench_function("parallel_insert_remove", |b| {
        let map = Arc::new(folklore::HashMap::with_capacity(NUM_KEYS));
        b.iter_custom(|iters| {
            let mut handles = vec![];
            for _ in 0..THREAD_COUNT {
                let map = map.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..iters {
                        let mut rng = thread_rng();
                        let mut bits: u64 = rng.gen();
                        let mut mask = 0u64;

                        for _ in 0..6 {
                            mask <<= 4;
                            mask |= bits & 0b00001111;
                            bits >>= 4;

                            for i in 0..NUM_OPS {
                                let key: u64 = rng.gen::<u64>() & mask;
                                map.insert(key, i as u16);
                                let key: u64 = rng.gen::<u64>() & mask;
                                map.update(&key, i as u16);
                            }
                        }
                    }
                }));
            }
            let start = Instant::now();
            for h in handles {
                h.join().unwrap();
            }
            start.elapsed()
        });
    });
    group.finish();
}

fn bench_leapfrog_leapmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("leapfrog_leapmap");
    group.throughput(Throughput::Elements(
        NUM_OPS * 6 * 2_u64 * THREAD_COUNT as u64,
    ));
    group.sample_size(10);
    group.bench_function("parallel_insert_remove", |b| {
        let map = Arc::new(leapfrog::LeapMap::with_capacity_and_hasher(
            NUM_KEYS,
            BuildHasherDefault::<HashFn>::default(),
        ));

        b.iter_custom(|iters| {
            let mut handles = vec![];
            for _ in 0..THREAD_COUNT {
                let map = map.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..iters {
                        let mut rng = thread_rng();
                        let mut bits: u64 = rng.gen();
                        let mut mask = 0u64;

                        for _ in 0..6 {
                            mask <<= 4;
                            mask |= bits & 0b00001111;
                            bits >>= 4;

                            for i in 0..NUM_OPS {
                                let key: u64 = rng.gen::<u64>() & mask;
                                map.insert(key, i as u16);
                                let key: u64 = rng.gen::<u64>() & mask;
                                map.update(&key, i as u16);
                            }
                        }
                    }
                }));
            }
            let start = Instant::now();
            for h in handles {
                h.join().unwrap();
            }
            start.elapsed()
        });
    });
    group.finish();
}

fn bench_std_hashmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("std_hashmap_rw_lock");
    group.throughput(Throughput::Elements(
        NUM_OPS * 6 * 2_u64 * THREAD_COUNT as u64,
    ));
    group.sample_size(10);
    group.bench_function("parallel_insert_remove", |b| {
        let map = Arc::new(std::sync::RwLock::new(
            std::collections::HashMap::with_capacity_and_hasher(
                NUM_KEYS,
                BuildHasherDefault::<HashFn>::default(),
            ),
        ));

        b.iter_custom(|iters| {
            let mut handles = vec![];
            for _ in 0..THREAD_COUNT {
                let map = map.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..iters {
                        let mut rng = thread_rng();
                        let mut bits: u64 = rng.gen();
                        let mut mask = 0u64;

                        for _ in 0..6 {
                            mask <<= 4;
                            mask |= bits & 0b00001111;
                            bits >>= 4;

                            let mut map_write = map.write().unwrap();
                            for i in 0..NUM_OPS {
                                let key: u64 = rng.gen::<u64>() & mask;
                                map_write.insert(key, i as u16);
                                let key: u64 = rng.gen::<u64>() & mask;
                                map_write.insert(key, i as u16);
                            }
                        }
                    }
                }));
            }
            let start = Instant::now();
            for h in handles {
                h.join().unwrap();
            }
            start.elapsed()
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_folklore_hashmap,
    bench_leapfrog_leapmap,
    bench_std_hashmap,
);
criterion_main!(benches);
