#![allow(clippy::pedantic)]

use criterion::Criterion;
use criterion::Throughput;
use criterion::{criterion_group, criterion_main};
use rand::{thread_rng, Rng};
use std::hash::BuildHasherDefault;

const CAPACITY: usize = i16::MAX as usize;
const NUM_OPS: u64 = i16::MAX as u64;

type HashFn = std::collections::hash_map::DefaultHasher;

fn bench_folklore_hashmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("folklore_hashmap");
    group.throughput(Throughput::Elements(NUM_OPS * 6 * 2_u64));
    group.sample_size(10);
    group.bench_function("insert_and_remove", |b| {
        let map = folklore::HashMap::with_capacity(CAPACITY);
        let mut rng = thread_rng();
        let mut bits: u64 = rng.gen();
        let mut mask = 0u64;

        b.iter(|| {
            for _ in 0..6 {
                mask <<= 4;
                mask |= bits & 0b00001111;
                bits >>= 4;

                for i in 0..NUM_OPS {
                    let key = rng.gen::<u64>() & mask;
                    map.insert(key, i as u16);
                    let key = rng.gen::<u64>() & mask;
                    map.update(&key, i as u16);
                }
            }
        })
    });
    group.finish();
}

fn bench_leapfrog_leapmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("leapfrog_leapmap");
    group.throughput(Throughput::Elements(NUM_OPS * 6 * 2_u64));
    group.sample_size(10);
    group.bench_function("insert_and_remove", |b| {
        let map = leapfrog::LeapMap::with_capacity_and_hasher(
            CAPACITY,
            BuildHasherDefault::<HashFn>::default(),
        );

        let mut rng = thread_rng();
        let mut bits: u64 = rng.gen();
        let mut mask = 0u64;

        b.iter(|| {
            for _ in 0..6 {
                // Add 4 random bits
                mask <<= 4;
                mask |= bits & 0b00001111;
                bits >>= 4;

                for i in 0..NUM_OPS {
                    let key = rng.gen::<u64>() & mask;
                    map.insert(key, i as u16);
                    let key = rng.gen::<u64>() & mask;
                    map.update(&key, i as u16);
                }
            }
        })
    });
    group.finish();
}

fn bench_std_hashmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("std_hashmap");
    group.throughput(Throughput::Elements(NUM_OPS * 6 * 2_u64));
    group.sample_size(10);
    group.bench_function("insert_and_remove", |b| {
        let mut map = std::collections::HashMap::with_capacity_and_hasher(
            CAPACITY,
            BuildHasherDefault::<HashFn>::default(),
        );

        let mut rng = thread_rng();
        let mut bits: u64 = rng.gen();
        let mut mask = 0u64;

        b.iter(|| {
            for _ in 0..6 {
                // Add 4 random bits
                mask <<= 4;
                mask |= bits & 0b00001111;
                bits >>= 4;

                for i in 0..NUM_OPS {
                    let key = rng.gen::<u64>() & mask;
                    map.insert(key, i as u16);
                    let key = rng.gen::<u64>() & mask;
                    map.insert(key, i as u16);
                }
            }
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_folklore_hashmap,
    bench_leapfrog_leapmap,
    bench_std_hashmap
);
criterion_main!(benches);
