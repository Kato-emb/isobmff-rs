//! [`to_u64`], which counts a length in memory as the lengths of a file count

/// Converts the length, or the count, `length` of something in memory to the `u64` the sizes of boxes (ISO/IEC 14496-12 §4.2.2) and the limits of the crate count in
///
/// It always fits on the targets the crate builds for, whose `usize` is no
/// wider than 64 bits; were it not to, it would count as `u64::MAX`, past any
/// size or limit.
pub(crate) fn to_u64(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
