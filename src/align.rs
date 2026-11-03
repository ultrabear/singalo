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
    ($(($name:ident, $size:expr)),+,) => {

        $(
        /// Alignment marker type with an alignment corresponding to its name
        #[derive(Copy, Clone)]
        #[repr(align($size))]
        pub struct $name;

        impl AlignmentMarker for Alignment<{ $size }> {
            type Aligned = $name;

            const ALIGN: usize = $size;
        }

        )+
    };
}

/// Represents an alignment parameter, implements [`AlignmentMarker`]
#[derive(Copy, Clone)]
pub struct Alignment<const A: usize>;

make_alignment!(
    (Align1, 1),
    (Align2, 2),
    (Align4, 4),
    (Align8, 8),
    (Align16, 16),
    (Align32, 32),
    (Align64, 64),
    (Align128, 128),
    (Align256, 256),
    (Align512, 512),
    (Align1024, 1024),
    (Align2048, 2048),
    (Align4096, 4096),
    (Align8192, 8192),
    (Align16384, 16384),
    (Align32768, 32768),
);

#[cfg(any(target_pointer_width = "32", target_pointer_width = "64"))]
make_alignment!(
    (Align65536, 65536),
    (Align131072, 131072),
    (Align262144, 262144),
    (Align524288, 524288),
    (Align1048576, 1048576),
    (Align2097152, 2097152),
    (Align4194304, 4194304),
    (Align8388608, 8388608),
    (Align16777216, 16777216),
    (Align33554432, 33554432),
    (Align67108864, 67108864),
    (Align134217728, 134217728),
    (Align268435456, 268435456),
    (Align536870912, 536870912),
);

