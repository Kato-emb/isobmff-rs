//! Cutting an input into the parts it arrives in
//!
//! Shared across the fuzz targets with `#[path = "helpers/cut.rs"] mod cut;`. A
//! file under `fuzz_targets/` is a target only where the `[[bin]]` table names
//! it, so this module is not one.

/// Cuts `bytes` at the cycled `lengths`, each one byte longer than it reads
pub fn cut_into(bytes: &[u8], lengths: [u8; 4]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;

    lengths.into_iter().cycle().map_while(move |length| {
        if rest.is_empty() {
            return None;
        }
        let (taken, remainder) = rest.split_at((usize::from(length) + 1).min(rest.len()));
        rest = remainder;
        Some(taken)
    })
}
