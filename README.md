# folklore
A lock-less concurrent hash map, based on the 'folklore' implementation described in ["Concurrent Hash Tables: Fast and General(?)!" by Maier et al.](https://arxiv.org/pdf/1601.04017.pdf)

## What?
This has some major limitations compared to a more general hash-map implementation. Namely;
- It cannot be grown past its initial capacity.
- The capacity is limited to `i16::MAX`_*_.
- (Currently) It can only store values which are exactly 2 bytes.
- Lookup performance is degraded when the map contains many "tombstones" (deleted entries).

The only benefits are:
- Blazingly fast 🔥 for concurrent access / modification.
- Can be shared safely across threads without requiring `Arc`, `Mutex`, etc.

There are very few use cases which are not impacted by any of the above limitations, but the benefits can be pretty good for use cases that don't care about those limitations.

## How?
Map entries are a 16-bit key offset, and a 16-bit value. This means that any operation on a map entry can be completed with a single 32-bit CAS instruction. This could be modified / extended to use a 32-bit key offset and 32-bit value, which would improve the max capacity to `i32::MAX`, allow values of 4 bytes, and still all operations would be done in single CAS instructions (of 64 bits). I'd like to offer a version of the map that uses 32-bit key offset and 32-bit values, but unsure how to go about it, and I don't need it right now.

The actual map entries store a "key offset" rather than a key, because the keys are allocated in a separate store. The key store is a "ConcurrentArray" which is lock-less and safe for concurrent access, but entries are immutable, and can only be removed if they were the most recently added.

## Performance
Some basic benchmarks are included in this repo which compare against `std::collections::HashMap` and `leapfrog::LeapMap`. There are a set of benchmarks for single-thread, and a set for multi-thread.
### Single-threaded
| Map              | Time     | Throughput     |
| ---------------- | -------- | -------------- |
| std HashMap      | 12.011ms | 32.736 Melem/s |
| leapfrog LeapMap | 11.250ms | 34.952 Melem/s |
| folklore Map     | 7.7111ms | 50.992 Melem/s |
### Multi-threaded (8 threads)
| Map              | Time     | Throughput     |
| ---------------- | -------- | -------------- |
| std HashMap      | 126.47ms | 24.873 Melem/s |
| leapfrog LeapMap | 23.950ms | 131.34 Melem/s |
| folklore Map     | 17.704ms | 177.68 Melem/s |

The numbers of each benchmark are pretty useless on their own, but comparing them we can see that for single-threaded scenarios there isn't much benefit in choosing this library over the std (hashbrown) HashMap. But it really shines in multi-threaded scenarios. Again these benchmarks are very basic, only testing insertion and removal.

## Todo
- [ ] Consider using a "value store", like the key store, which would allow for values of any size.

_\* Actually, the capacity will be slightly more than `i16::MAX`, to account for the load factor. But the `::new()` function won't accept anything higher than `i16::MAX`_.

Inspired by the `ConcurrentMap` implementation in [couchbase/fleece](https://github.com/couchbase/fleece/blob/master/Fleece/Support/ConcurrentMap.cc).

[robclu/leapfrog](https://github.com/robclu/leapfrog) was instrumental in my understanding how this kinda thing should be written in Rust. The leapfrog map also doesn't suffer from many of the limitations of this map, and is a much better choice in most cases.
