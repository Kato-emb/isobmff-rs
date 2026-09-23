//! Reading the `sample_flags` word a fuzz input states
//!
//! Shared across the fuzz targets with `#[path = "helpers/sample_flags.rs"] mod sample_flags;`.
//! A file under `fuzz_targets/` is a target only where the `[[bin]]` table names
//! it, so this module is not one.

use isobmff::boxes::SampleFlags;
use libfuzzer_sys::arbitrary::{self, Unstructured};

/// Reads a `sample_flags` word, passing over one that sets a reserved bit
pub fn sample_flags(unstructured: &mut Unstructured<'_>) -> arbitrary::Result<SampleFlags> {
    SampleFlags::from_bits(unstructured.arbitrary()?).ok_or(arbitrary::Error::IncorrectFormat)
}
