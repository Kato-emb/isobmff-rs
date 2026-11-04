//! The conversions of numbers in memory to the numbers of a file
//!
//! A conversion never panics: a value that would not fit is capped at the
//! largest the wider type holds, past any size or limit the crate counts. The
//! targets the crate builds for are checked as it builds, so that a value
//! that always fits does on every one of them.

/// Holds that every `usize` fits in a `u64`, so that [`to_u64`] always fits, on any target the crate builds for
const _: () = assert!(size_of::<usize>() <= size_of::<u64>());

/// Converts a length or a count in memory to `u64`, as the sizes of boxes (ISO/IEC 14496-12 §4.2.2) and the limits of the crate count it
///
/// It always fits on the targets this crate builds for, as the module checks
/// at build time; a value that would not is counted as `u64::MAX`. Unlike a
/// conversion that may fail, it returns the `u64` itself.
pub(crate) fn to_u64(length: usize) -> u64 {
    // Why not `as` or `From`: no `From<usize>` for `u64` holds on every target, and the type system cannot prove the fit across them; a value that did not fit would count past any size or limit.
    u64::try_from(length).unwrap_or(u64::MAX)
}
