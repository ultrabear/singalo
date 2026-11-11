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
//!
//!
//! # Examples
//! ```
//! # extern crate alloc;
//! # use alloc::vec::Vec;
//! use singalo::{Alignment, SingleAllocator};
//!
//! const LEN: usize = 10;
//!
//! let allocator = SingleAllocator::<
//!         { size_of::<u64>() * LEN },
//!         Alignment<{ align_of::<u64>() }>
//!     >::new();
//!
//! let ve = Vec::<u64, _>::with_capacity_in(LEN, &allocator);
//!
//! ```

#![no_std]
#![deny(clippy::pedantic)]
#![warn(missing_docs, clippy::missing_docs_in_private_items)]

#[cfg(any(test, doc))]
extern crate alloc;

use core::{
    alloc::{AllocError, Allocator, Layout},
    cell::UnsafeCell,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::{AtomicBool, Ordering},
};

pub mod align;

pub use align::Alignment;

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
    /// Callers must ensure that size is lesser than the SIZE const that defines [`SingleAllocator`]'s
    /// maximum allocation size
    ///
    /// This function does not operate with ownership of block, thus, it is safe to call before
    /// gaining ownership, and it does not generate ownership
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

    /// Attempts to take ownership of block
    fn begin_ownership(&self) -> Result<(), AllocError> {
        // try to take ownership of block (if another caller has set allocated already we know we
        // cant give out the block)
        self.allocated
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| AllocError)
            .map(|_| ())
    }

    /// Releases ownership of block
    ///
    /// # Safety
    /// The caller must have ownership of block
    unsafe fn end_ownership(&self) {
        self.allocated.store(false, Ordering::Release);
    }

    /// Returns whether the given pointer is within the bounds of this allocation for debug assertions
    #[cfg(debug_assertions)]
    fn within_allocation(&self, ptr: *const u8, layout: Layout) -> bool {
        // we special case ZST allocations to be dangling, this means we compare to our custom ZST
        // pointer (with guaranteed pointer eq) rather than our block pointer
        if layout.size() == 0 {
            return core::ptr::eq(ptr, core::ptr::without_provenance::<u8>(layout.align()));
        }

        let base = (&raw const self.block).cast::<u8>();
        // SAFETY: block is [u8; SIZE], so base.add(SIZE) is within provenance
        let end = unsafe { base.add(SIZE) };

        base <= ptr && ptr <= end
    }

    /// Returns an offset pointer allocation if it fits in layout (if it needs to be offset at all)
    ///
    /// Note that this function does not require ownership of block to compute
    /// and thus, it is not returning ownership in `Result::Ok`, only a valid memory region from block
    /// that fits Layout.
    fn block_for(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            // SAFETY: we are constructing a ZST allocation without any provenance,
            // NonNull is upheld by layout.align() being NonZero
            return Ok(unsafe {
                NonNull::slice_from_raw_parts(
                    NonNull::new_unchecked(
                        core::ptr::without_provenance::<u8>(layout.align()).cast_mut(),
                    ),
                    0,
                )
            });
        }

        // handle superaligned data
        if layout.align() > A::ALIGN {
            let ptr = unsafe {
                NonNull::new_unchecked(UnsafeCell::raw_get(&raw const self.block).cast::<u8>())
            };

            let offset = ptr.align_offset(layout.align());

            // align_offset error case
            if offset == usize::MAX {
                return Err(AllocError);
            }

            let size_avail = SIZE.checked_sub(offset).ok_or(AllocError)?;

            if layout.size() > size_avail {
                return Err(AllocError);
            }

            // SAFETY: constructed via successful align_offset that is in bounds of our SIZE
            let ptr = unsafe { ptr.add(offset) };

            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        } else {
            // cant allocate more than our fixed size buffer
            if layout.size() > SIZE {
                return Err(AllocError);
            }

            // SAFETY: we checked size and alignment requirements, and can hand it to the caller
            // we do not have ownership, so we do not provide it
            Ok(unsafe { self.unchecked_base_allocate(layout.size()) })
        }
    }
}

// UB with UB: Allocator may never unwind (panic), but it is UB to pass certain arguments to
// allocator methods (and arguments with cheap checks), in such scenarios we employ debug_assert
// to panic anyways. The rationale being that if the caller has already invoked UB we are allowed to
// do our own UB (with the goal of harm reduction)
unsafe impl<const SIZE: usize, A: AlignmentMarker> Allocator for &SingleAllocator<SIZE, A> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let block = self.block_for(layout)?;

        self.begin_ownership()?;

        Ok(block)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        debug_assert!(
            self.within_allocation(ptr.as_ptr(), layout),
            "ptr must be allocated by this SingleAllocator"
        );

        // release ownership of block
        // SAFETY: we have been passed ptr which means we can invalidate it, thus taking ownership back
        unsafe {
            self.end_ownership();
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(
            self.within_allocation(ptr.as_ptr(), old_layout),
            "ptr must be allocated by this SingleAllocator"
        );

        debug_assert!(
            new_layout.size() >= old_layout.size(),
            "`new_layout.size()` must be greater than or equal to `old_layout.size()`"
        );

        // SAFETY: as per grow contract, old_layout was previously allocated by our allocate
        // function which means it must succeed here
        let old_block = unsafe { self.block_for(old_layout).unwrap_unchecked() };

        let new_block = self.block_for(new_layout)?;

        // optimization: we do not need to copy as the base pointer is the same
        if core::ptr::eq(
            old_block.as_ptr().cast::<u8>(),
            new_block.as_ptr().cast::<u8>(),
        ) {
            return Ok(new_block);
        }

        // SAFETY: old_block likely overlaps new_block so we use overlapping copy,
        // we copy old_layout.size() and leave the grown portion uninit to match the grow contract
        // old_layout size is within bounds of both allocations as per grow contract and our checks
        // above
        unsafe {
            core::ptr::copy(
                old_block.as_ptr().cast::<u8>(),
                new_block.as_ptr().cast::<u8>(),
                old_layout.size(),
            );
        }

        Ok(new_block)
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
            self.within_allocation(ptr.as_ptr(), old_layout),
            "ptr must be allocated by this SingleAllocator"
        );

        debug_assert!(
            new_layout.size() <= old_layout.size(),
            "`new_layout.size()` must be smaller than or equal to `old_layout.size()`"
        );

        // SAFETY: old_layout was previously handed to us and successful as per caller contract
        let old_block = unsafe { self.block_for(old_layout).unwrap_unchecked() };

        let new_block = self.block_for(new_layout)?;

        // Optimization: base pointers are same so we dont need to copy data
        if core::ptr::eq(
            old_block.as_ptr().cast::<u8>(),
            new_block.as_ptr().cast::<u8>(),
        ) {
            return Ok(new_block);
        }

        // SAFETY: old_block likely overlaps new_block so we use overlapping copy,
        // we copy new_layout.size() as it is smaller than old_layout and the size of the new allocation
        unsafe {
            core::ptr::copy(
                old_block.as_ptr().cast::<u8>(),
                new_block.as_ptr().cast::<u8>(),
                new_layout.size(),
            );
        }

        Ok(new_block)
    }
}

impl<const SIZE: usize, A: AlignmentMarker> Default for SingleAllocator<SIZE, A> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        SingleAllocator,
        align::{Alignment, AlignmentMarker},
    };

    use core::alloc::{Allocator, Layout};

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

    #[test]
    fn ownership_once() {
        let allocator = &SingleAllocator::<500, Alignment<8>>::new();

        let ptr_one = allocator
            .allocate(Layout::new::<u64>())
            .expect("first allocation that fits succeeds");

        allocator
            .allocate(Layout::new::<u64>())
            .expect_err("cannot allocate a second item");

        unsafe { allocator.deallocate(ptr_one.cast::<u8>(), Layout::new::<u64>()) };

        allocator
            .allocate(Layout::new::<u64>())
            .expect("deallocate releases ownership");
    }

    #[test]
    fn grow() {
        let allocator = &SingleAllocator::<500, Alignment<2>>::new();

        let l1 = Layout::new::<[u32; 16]>();

        let ptr = allocator.allocate(l1).expect("fits");

        // increase size + align
        let l2 = Layout::new::<[u64; 16]>();

        // increasing alignment requirement works
        let grown = unsafe { allocator.grow(ptr.cast::<u8>(), l1, l2).expect("fits") };

        // increase size
        let l3 = Layout::new::<[u64; 32]>();

        unsafe {
            allocator
                .grow_zeroed(grown.cast::<u8>(), l2, l3)
                .expect("fits")
        };
    }

    #[test]
    fn shrink() {
        let allocator = &SingleAllocator::<500, Alignment<2>>::new();

        let l1 = Layout::new::<[u64; 16]>();

        let ptr = allocator.allocate(l1).expect("fits");

        let l2 = Layout::new::<[u32; 16]>();

        unsafe {
            allocator
                .shrink(ptr.cast::<u8>(), l1, l2)
                .expect("can shrink")
        };
    }

    #[test]
    fn zst() {
        let allocator = &SingleAllocator::<0, Alignment<{ 1 << 10 }>>::new();

        let l1 = Layout::new::<[<Alignment<{ 1 << 10 }> as AlignmentMarker>::Aligned; 0]>();

        let ptr = allocator.allocate(l1).expect("can allocate a ZST");

        // safe to deallocate
        unsafe {
            allocator.deallocate(ptr.cast::<u8>(), l1);
        }
    }
}
