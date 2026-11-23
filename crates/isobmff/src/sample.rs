//! The samples of a track as the structure-data describes them

use isobmff_boxes::SampleFlags;

/// What the structure-data declares of a `sample`, apart from where its bytes lie
///
/// The fields of a sample a sample table (ISO/IEC 14496-12 §8.5) or a track
/// fragment (§8.8.6) gives it.
#[expect(
    clippy::exhaustive_structs,
    reason = "a caller of a writer builds one, and the fields are those a sample table or a track fragment declares of a sample"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleProperties {
    /// `track_ID` of the track the sample belongs to
    pub track_id: u32,
    /// Decoding time of the sample, in the timescale of the media, as given by
    /// the sample table (§8.6.1.2) or track fragment (§8.8.12)
    pub decoding_time: u64,
    /// `sample_duration` of the sample, the `sample_delta` of the sample table
    /// (§8.6.1.2) or the `sample_duration` of the track fragment (§8.8.7, §8.8.8)
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
