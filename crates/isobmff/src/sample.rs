//! The samples of a track as the structure-data describes them

use isobmff_boxes::SampleFlags;

/// What the structure-data declares of a `sample`, apart from where its bytes lie
///
/// The fields of a sample a sample table (ISO/IEC 14496-12 §8.5) or a track
/// fragment run (§8.8.8) gives it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleProperties {
    /// `track_ID` of the track the sample belongs to
    pub track_id: u32,
    /// Decoding time of the sample, in the timescale of the media: the sum of
    /// the `sample_duration`s of the samples before it in the track
    pub decoding_time: u64,
    /// `sample_duration`, the difference between the decoding time of the
    /// sample and that of the next
    pub sample_duration: u32,
    /// `sample_composition_time_offset`, the difference between the
    /// composition time of the sample and its decoding time
    pub sample_composition_time_offset: i64,
    /// `sample_flags`, as §8.8.3.1 packs them; for a sample of a sample
    /// table, the sync sample table and the sample dependency types give them
    pub sample_flags: SampleFlags,
    /// `sample_description_index`, from 1, of the sample entry that describes
    /// the sample
    pub sample_description_index: u32,
}

/// Where the bytes of a `sample` lie, with what the structure-data declares of it
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleLocation {
    /// What the structure-data declares of the sample
    pub properties: SampleProperties,
    /// File offset of the first byte of the sample
    pub offset: u64,
    /// Number of bytes of the sample
    pub size: u32,
}
