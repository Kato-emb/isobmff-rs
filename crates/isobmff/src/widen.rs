//! [`widen`], which widens a length in memory to the width the crate counts file offsets and limits in

/// Widens the length, or the count, `length` of something in memory to the `u64` the sizes of boxes (ISO/IEC 14496-12 §4.2.2) and the limits of the crate count in
///
/// It always fits on the targets the crate builds for, 64-bit ones and
/// `thumbv7em`.
pub(crate) fn widen(length: usize) -> u64 {
    // Why not `as` or `From`: no `From<usize>` for `u64` holds on every target, and the type system cannot prove the fit across them; a value that did not fit would count past any size or limit.
    u64::try_from(length).unwrap_or(u64::MAX)
}
