//! singalo; the Singleton [Allocator], now in stable rust!
//!
//! The use of singleton here refers to the amount of allocations an instance of a [`SingleAllocator`]
//! can support; one.
//!
//! This is useful as the backing for general structures ([`Vec`][alloc::vec::Vec],
//! [`Box`][alloc::boxed::Box]) that always use a
//! single allocation in an otherwise `no_alloc` context, due to the use of a local array that backs
//! the allocation. It is also suitable for structures that *may* exist with one allocation in
//! certain usage scenarios.
//!
//! As this is a generic tool, and at the time of writing not many structures are implemented with
//! [Allocator], there are not many datastructures that can make use of singalo yet. In the future
//! it is expected that structures like `VecDeque`, `HashMap`, `Box` etc. have effective [Allocator]
//! taking constructors (note `Box` has `new_in` but not zeroed/uninit constructors) and this crate
//! becomes more generally useful.

#![no_std]
#![deny(clippy::pedantic)]
#![warn(missing_docs, clippy::missing_docs_in_private_items)]

#[cfg(doc)]
extern crate alloc;

use core::{
    alloc::{AllocError, Allocator, Layout},
    cell::UnsafeCell,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::{AtomicBool, Ordering},
};

pub mod align;

use align::AlignmentMarker;

/// A block of u8 with an alignment defined by an [`Alignment`][align::Alignment]
#[repr(C)]
struct AlignedBlock<const SIZE: usize, A: AlignmentMarker>(
    MaybeUninit<[u8; SIZE]>,
    [A::Aligned; 0],
);

/// A local array based singleton Allocator implementer
///
/// `SingleAllocator` stores a `[u8; SIZE]` locally and a state flag that informs whether the single
/// allocation to the backing `[u8; SIZE]` has been given out
///
/// The current maximum supported alignment of `SingleAllocator` is 64 via the internal block
/// requiring that alignment, this simplifies implementation and covers the vast majority of
/// users, consequently, the current smallest `SingleAllocator` instance is 128 bytes
/// (and must grow by 64 byte increments, so a SIZE of eg. 65 is wasteful).
pub struct SingleAllocator<const SIZE: usize, A: AlignmentMarker> {
    /// flag for whether or not the single allocation has been given out already
    /// defines ownership: allocated=false then we own block, allocated=true then a user with ptr
    /// owns block, and them passing ptr to us is considered ownership in that scenario (so we only
    /// need to RMU during full dealloc/alloc calls)
    allocated: AtomicBool,
    /// the block of memory, stored behind `UnsafeCell` to prevent accidental noalias violation,
    /// though this is probably not strictly necessary for the implementation as is
    block: UnsafeCell<AlignedBlock<SIZE, A>>,
}

impl<const SIZE: usize, A: AlignmentMarker> SingleAllocator<SIZE, A> {
    /// Creates a new `SingleAllocator` with the given backing array size
    #[must_use]
    pub const fn new() -> Self {
        Self {
            allocated: AtomicBool::new(false),
            block: UnsafeCell::new(AlignedBlock(MaybeUninit::uninit(), [])),
        }
    }

    /// Generates a [`NonNull<[u8]>`][core::ptr::NonNull] with the requested size from the base of the singleton allocation block
    ///
    /// # Safety
    /// Callers must ensure that they have ownership of the block (via the allocated [`AtomicBool`] for
    /// alloc calls or pointer ownership via the grow/shrink methods)
    /// Callers must ensure that size is lesser than the SIZE const that defines [`SingleAllocator`]'s
    /// maximum allocation size
    unsafe fn unchecked_base_allocate(&self, size: usize) -> NonNull<[u8]> {
        NonNull::slice_from_raw_parts(
            // SAFETY: we have ensured that the layouts size is smaller than the block, so casting
            // is safe, we have ensured that the pointer is non null as it comes from a &raw ref, we
            // can safely create a NonNull<[u8]> with Uninit storage bytes as the Allocator::allocate
            // contract states it may be uninit
            unsafe {
                NonNull::new_unchecked(UnsafeCell::raw_get(&raw const self.block).cast::<u8>())
            },
            size,
        )
    }
}

// UB with UB: Allocator may never unwind (panic), but it is UB to pass certain arguments to
// allocator methods (and arguments with cheap checks), in such scenarios we employ debug_assert
// to panic anyways. The rationale being that if the caller has already envoked UB we are allowed to
// do our own UB (with the goal of harm reduction)
unsafe impl<const SIZE: usize, A: AlignmentMarker> Allocator for &SingleAllocator<SIZE, A> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // handle superaligned data
        if layout.align() > A::ALIGN {
            let ptr = unsafe {
                NonNull::new_unchecked(UnsafeCell::raw_get(&raw const self.block).cast::<u8>())
            };


            ptr.align_offset(layout.align());

            return Err(AllocError);
        } else {
            // cant allocate more than our fixed size buffer
            if layout.size() > SIZE {
                return Err(AllocError);
            }

            // compute last to not deadlock:
            // try to take ownership of block (if another caller has set allocated already we know we
            // cant give out the block)
            self.allocated
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .map_err(|_| AllocError)?;

            // SAFETY: we took ownership of the block, checked size and alignment requirements, and can
            // hand it to the caller
            Ok(unsafe { self.unchecked_base_allocate(layout.size()) })
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        debug_assert!(
            core::ptr::eq(ptr.as_ptr(), (&raw const self.block).cast::<u8>()),
            "ptr must be allocated by this SingleAllocator"
        );

        // release ownership of block
        self.allocated.store(false, Ordering::Release);
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(
            core::ptr::eq(ptr.as_ptr(), (&raw const self.block).cast::<u8>()),
            "ptr must be allocated by this SingleAllocator"
        );

        debug_assert!(
            new_layout.size() >= old_layout.size(),
            "`new_layout.size()` must be greater than or equal to `old_layout.size()`"
        );

        // assume block is owned by us as per Allocator contract

        // reject superaligned data for now
        if new_layout.align() > A::ALIGN {
            return Err(AllocError);
        }

        // cant allocate more than our fixed size buffer
        if new_layout.size() > SIZE {
            return Err(AllocError);
        }

        // rewriting is unnecessary as its the same allocation

        // SAFETY: we check the size and alignment requirements above, a caller of grow must assert
        // that _ptr+old_layout was previously handed out by this allocator, so we skip the
        // ownership check (_ptr holds the ownership)
        Ok(unsafe { self.unchecked_base_allocate(new_layout.size()) })
    }

    unsafe fn grow_zeroed(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        // SAFETY: inherits safety of caller of grow_zeroed
        let grown = unsafe { self.grow(ptr, old_layout, new_layout)? };

        // SAFETY: new_layout.size() must be greater than old_layout.size() so we will not underflow
        // additionally, the contract of grow_zeroed states the full `new_size` must be zeroed,
        // which is distinct from new_layout (the length of the NonNull<[u8]>) returned
        // an implementation detail of SingleAllocator is we always return the exact layout
        // requested, so new_layout.size() will always be equal to `new_size`
        unsafe {
            grown
                .cast::<u8>()
                .add(old_layout.size())
                .write_bytes(0, new_layout.size() - old_layout.size());
        }

        Ok(grown)
    }

    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(
            core::ptr::eq(ptr.as_ptr(), (&raw const self.block).cast::<u8>()),
            "ptr must be allocated by this SingleAllocator"
        );

        debug_assert!(
            new_layout.size() <= old_layout.size(),
            "`new_layout.size()` must be smaller than or equal to `old_layout.size()`"
        );

        // reject superaligned data for now
        if new_layout.align() > A::ALIGN {
            return Err(AllocError);
        }

        // SAFETY: new_layout.size() must be smaller than old_layout.size(), and a caller must have
        // upheld that _ptr + old_layout was previously successfully allocated by this allocator
        // (this, new_layout is guaranteed to fit aside from alignment differences which we check for)
        Ok(unsafe { self.unchecked_base_allocate(new_layout.size()) })
    }
}

impl<const SIZE: usize, A: AlignmentMarker> Default for SingleAllocator<SIZE, A> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::{SingleAllocator, align::Alignment};

    extern crate alloc;

    #[test]
    fn allocates_vec() {
        let mut block = SingleAllocator::<500, Alignment<64>>::new();

        let mut ve = alloc::vec::Vec::<u8, _>::new_in(&block);

        ve.push(4);

        assert_eq!(ve[0], 4);

        for _ in 0..250 {
            ve.push(5);
        }

        assert!(ve.capacity() <= 500);

        assert!(ve.try_reserve(300).is_err());

        assert_eq!(
            block.allocated.load(core::sync::atomic::Ordering::Relaxed),
            true
        );

        // should release block
        drop(ve);

        assert_eq!(*block.allocated.get_mut(), false);

        let mut ve = alloc::vec::Vec::with_capacity_in(500, &block);

        for _ in 0..500 {
            ve.push(14u8);
        }

        assert!(ve.try_reserve_exact(1).is_err());

        drop(ve);

        let mut align = alloc::vec::Vec::<u128, _>::new_in(&block);

        align.push(50);

        drop(align);

        let canbox = alloc::boxed::Box::new_in([0u8; 500], &block);
        drop(canbox);
    }
}
