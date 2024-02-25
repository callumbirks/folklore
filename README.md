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

## Todo
- [ ] Benchmarks, would be interested in comparing against [robclu/leapfrog](https://github.com/robclu/leapfrog).
- [ ] Consider using a "value store", like the key store, which would allow for values of any size.

_\* Actually, the capacity will be slightly more than `i16::MAX`, to account for the load factor. But the `::new()` function won't accept anything higher than `i16::MAX`_.

Inspired by the `ConcurrentMap` implementation in [couchbase/fleece](https://github.com/couchbase/fleece/blob/master/Fleece/Support/ConcurrentMap.cc).

[robclu/leapfrog](https://github.com/robclu/leapfrog) was instrumental in my understanding how this kinda thing should be written in Rust. The leapfrog map also doesn't suffer from many of the limitations of this map, and is a much better choice in most cases.
