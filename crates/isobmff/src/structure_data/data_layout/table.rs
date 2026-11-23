//! How many entries a table of a box declares, in the 32 bits a box counts them in
//!
//! A box of ISO/IEC 14496-12 counts the entries of its table in a 32-bit
//! integer: the `entry_count` of the sample description table (§8.5.2), the
//! data reference table (§8.7.2), the sample-to-chunk table (§8.7.4) and the
//! chunk offset table (§8.7.5), and the `sample_count` of the sample size
//! table (§8.7.3.2).

/// Returns the number of the entries of a table of `length` entries, or `u32::MAX` past what a `u32` counts, the width of an `entry_count`
pub(super) fn count(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}
