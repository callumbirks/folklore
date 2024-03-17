# folklore
A lock-less concurrent hash map, based on the 'folklore' implementation described in ["Concurrent Hash Tables: Fast and General(?)!" by Maier et al.](https://arxiv.org/pdf/1601.04017.pdf)

## What?
This has some major limitations compared to a more general hash-map implementation. Namely;
- It cannot be grown past its initial capacity.
- The capacity is limited to `i16::MAX`_*_.
- It can only store values which are exactly 2 bytes.
- Removals are not supported (because of the immense slowdown caused by tombstones filling up the map).

The only benefits are:
- Blazingly fast 🔥 for concurrent access / modification.
- Can be shared safely across threads without requiring `Mutex`, `RwLock` etc.

This is kind of just a fun project exploring the implementation of something I read about in an academic paper. I wouldn't really recommend using it.

## How?
Map entries are a 16-bit key offset, and a 16-bit value. This means that any operation on a map entry can be completed with a single 32-bit CAS instruction. This could be modified / extended to use a 32-bit key offset and 32-bit value, which would improve the max capacity to `i32::MAX`, allow values of 4 bytes, and still all operations would be done in single CAS instructions (of 64 bits).

The actual map entries store a "key offset" rather than a key, because the keys are allocated in a separate store. The key store is a "ConcurrentArray" which is lock-less and safe for concurrent access, but entries are immutable, and can only be removed if they were the most recently added.

## Performance
Some basic benchmarks are included in this repo which compare against `std::collections::HashMap` and `leapfrog::LeapMap`. There are a set of benchmarks for single-thread, and a set for multi-thread.
### Single-threaded
| Map                  | Time     | Throughput     |
| -------------------- | -------- | -------------- |
| std HashMap          | 12.011ms | 32.736 Melem/s |
| leapfrog LeapMap     | 11.250ms | 34.952 Melem/s |
| folklore HashMap     | 7.7111ms | 50.992 Melem/s |
### Multi-threaded (8 threads)
| Map                  | Time     | Throughput     |
| -------------------- | -------- | -------------- |
| std HashMap          | 126.47ms | 24.873 Melem/s |
| leapfrog LeapMap     | 23.950ms | 131.34 Melem/s |
| folklore HashMap     | 17.704ms | 177.68 Melem/s |

The numbers of each benchmark are pretty useless on their own, but comparing them we can see that for single-threaded scenarios there isn't much benefit in choosing this library over the std (hashbrown) HashMap. But it really shines in multi-threaded scenarios. Again these benchmarks are very basic, only testing insertion and updating.

_\* Actually, the capacity will be slightly more than `i16::MAX`, to account for the load factor. But the `::new()` function won't accept anything higher than `i16::MAX`_.

Inspired by the `ConcurrentMap` implementation in [couchbase/fleece](https://github.com/couchbase/fleece/blob/master/Fleece/Support/ConcurrentMap.cc).

[robclu/leapfrog](https://github.com/robclu/leapfrog) was instrumental in my understanding how this kinda thing should be written in Rust. The leapfrog map also doesn't suffer from many of the limitations of this map, and is a much better choice in most cases.
