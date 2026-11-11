# SingAlo
A singleton Allocator implementer in stable rust! SingAlo provides a basic single allocation holding Allocator implementation with a generic size and base alignment to be backed by a local array. This can be useful for any datastructures (like, say, a `Vec`) that typically will only live in a single allocation, and using such structures (and their great corpus of methods/tools!) without having to rely on a system provided allocation routine.

`SingleAllocator<SIZE, ALIGN>` will allow allocating up to SIZE bytes up to an alignment specified by ALIGN, "super aligned" allocations may be requested and will be fulfilled, but will degrade to an implementation that is not guaranteed to make the full SIZE available. For example, if you are allocating an array of u64, it is best to use `SingleAllocator<{ size_of::<u64>() * LEN }, Alignment< { align_of::<u64>() } >>`, but `Alignment<2>` would still operate at a degraded performance.

This crate is no-std and no-alloc, and thus suitable for environments where a global allocator is not present.
