//! Pseudo generic alignment

mod sealed {
    pub trait Sealed {}
    impl<const A: usize> Sealed for super::Alignment<A> {}
}

/// A marker for alignment, this trait carries the Aligned associated type that is meant to be used
/// in a [T; 0] ZST to enforce minimum alignment of a struct
///
/// For example:
/// ```
/// # use singalo::align::{Alignment, AlignmentMarker};
/// assert_eq!(align_of::< <Alignment<2> as  AlignmentMarker>::Aligned>(), 2);
/// ```
pub trait AlignmentMarker: sealed::Sealed + Copy + Clone {
    /// The aligned type of this marker
    type Aligned: Copy + Clone;

    /// The alignment size
    const ALIGN: usize;
}

macro_rules! make_alignment {
    ($name:ident, $size:expr) => {
        /// Alignment marker type with an alignment corresponding to its name
        #[derive(Copy, Clone)]
        #[repr(align($size))]
        pub struct $name;

        impl AlignmentMarker for Alignment<{ $size }> {
            type Aligned = $name;

            const ALIGN: usize = $size;
        }
    };
}

/// Represents an alignment parameter, implements [`AlignmentMarker`]
#[derive(Copy, Clone)]
pub struct Alignment<const A: usize>;

make_alignment!(Align1, 1);
make_alignment!(Align2, 2);
make_alignment!(Align4, 4);
make_alignment!(Align8, 8);
make_alignment!(Align16, 16);
make_alignment!(Align32, 32);
make_alignment!(Align64, 64);
make_alignment!(Align128, 128);
make_alignment!(Align256, 256);
make_alignment!(Align512, 512);
make_alignment!(Align1024, 1024);
make_alignment!(Align2048, 2048);
make_alignment!(Align4096, 4096);
make_alignment!(Align8192, 8192);
