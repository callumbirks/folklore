# folklore
A lock-free concurrent hash map, based on the 'folklore' implementation described in ["Concurrent Hash Tables: Fast and General(?)!" by Maier et al.](https://arxiv.org/pdf/1601.04017.pdf)

## What?
This has some major limitations compared to a more general hash-map implementation. Namely;
- It cannot be grown past its initial capacity.
- The capacity is limited to `i16::MAX`.
- It can only store values which are exactly 2 bytes.
- Removals are not (currently) supported (because of the immense slowdown caused by tombstones filling up the map).

The only benefits are:
- Blazingly fast 🔥 for concurrent access / modification.
- Can be shared safely across threads without requiring `Mutex`, `RwLock` etc.

This is kind of just a fun project exploring the implementation of something I read about in an academic paper. I wouldn't really recommend using it.

## How?
Map entries are a 16-bit key offset, a 16-bit value, and a 32-bit key hash. This means that any operation on a map entry can be completed with a single 64-bit (1 word) CAS instruction.

The actual map entries store a "key offset" rather than a key, because the keys are allocated in a separate store. The key store is a "ConcurrentArray" which is lock-free and safe for concurrent access, but entries are immutable, and can only be removed if they were the most recently added.

## Consistency
Loads and Stores generally use `Ordering::Acquire` and `Ordering::Release` respectively. Initial lookup for an entry uses `Ordering::Relaxed` for performance reasons, so sometimes a newly inserted key might be missed by another thread.
However, that thread will never overwrite the key, because a stronger ordering is used for the actual insertion.

## Performance
Some basic benchmarks are included in this repo which compare against `std::collections::HashMap` and `leapfrog::LeapMap`. There are a set of benchmarks for single-thread, and a set for multi-thread. Here are the numbers I got on an M1 Pro MacBook:
### Single-threaded
| Map                  | Time     | Throughput     |
| -------------------- | -------- | -------------- |
| std HashMap          | 7.9036ms | 49.750 Melem/s |
| leapfrog LeapMap     | 8.8983ms | 44.189 Melem/s |
| folklore HashMap     | 4.9738ms | 79.055 Melem/s |
### Multi-threaded (8 threads)
| Map                  | Time     | Throughput     |
| -------------------- | -------- | -------------- |
| std HashMap (RWLock) | 58.689ms | 53.599 Melem/s |
| leapfrog LeapMap     | 18.841ms | 166.96 Melem/s |
| folklore HashMap     | 16.571ms | 189.83 Melem/s |

The numbers of each benchmark are pretty useless on their own, but comparing them we can see that folklore manages to just about beat out leapfrog. Again these benchmarks are very basic, only testing insertion and updating.

Inspired by the `ConcurrentMap` implementation in [couchbase/fleece](https://github.com/couchbase/fleece/blob/master/Fleece/Support/ConcurrentMap.cc).

[robclu/leapfrog](https://github.com/robclu/leapfrog) was instrumental in my understanding how this kinda thing should be written in Rust. The leapfrog map also doesn't suffer from many of the limitations of this map, and is a much better choice in most cases.
