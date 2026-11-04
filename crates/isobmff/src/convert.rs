//! The conversions of numbers in memory to the numbers of a file
//!
//! A conversion never panics: a value that would not fit is capped at the
//! largest the wider type holds, past any size or limit the crate counts. The
//! targets the crate builds for are checked as it builds, so that a value
//! that always fits does on every one of them.

/// Holds that every `usize` fits in a `u64`, so that [`widen`] always fits, on any target the crate builds for
const _: () = assert!(size_of::<usize>() <= size_of::<u64>());

/// Widens the length, or the count, `length` of something in memory to the `u64` the sizes of boxes (ISO/IEC 14496-12 §4.2.2) and the limits of the crate count in
///
/// It always fits: the crate builds only for targets whose `usize` is no
/// wider than a `u64`, as the module holds.
pub(crate) fn widen(length: usize) -> u64 {
    // Why not `as` or `From`: no `From<usize>` for `u64` holds on every target, and the type system cannot prove the fit across them; a value that did not fit would count past any size or limit.
    u64::try_from(length).unwrap_or(u64::MAX)
}
