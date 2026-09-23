//! [`Sample`], the unit a presentation carries its media in, and [`SampleExtent`], where one lies

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::SampleFlags;

/// One sample of one track, with the bytes it is carried as
///
/// A sample is what a presentation is made of — ISO/IEC 14496-12 §3.1.14 has it
/// as all the data associated with a single timestamp. What the file declares
/// about it is resolved before it is handed over: what a sample table spreads
/// over its tables (§8.7), or what a `trun` row leaves to the `tfhd` of its
/// fragment and the `trex` of its track (§8.8.7, §8.8.8), so every field here
/// is settled.
///
/// The times are measured in the time scale of the track, the one its `mdhd`
/// declares (§8.4.2). They are not converted: a caller placing samples of two
/// tracks on one timeline reads that time scale off the `moov` itself. A sample
/// the file states no composition time offset for — a track without a `ctts`
/// (§8.6.1.3), a `trun` row without the field (§8.8.8) — is composed when it is
/// decoded, and carries an offset of zero.
///
/// The `sample_flags` are a [`SampleFlags`], the fields §8.8.3.1 lays out,
/// which cannot state a reserved bit. A sample table states those fields in
/// tables of their own — `sdtp`, `padb`, `stss` and `stdp` — and a field the
/// track carries no table for is zero, but for the sync samples: every sample
/// of a track without an `stss` (§8.6.2) is one, and leaves
/// `sample_is_non_sync_sample` clear.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Sample {
    track_id: u32,
    decode_time: u64,
    sample_duration: u32,
    sample_composition_time_offset: i64,
    sample_flags: SampleFlags,
    sample_description_index: u32,
    data: Vec<u8>,
}

impl Sample {
    /// Creates the sample from the properties settled for it and the bytes it carries
    #[must_use]
    pub const fn new(
        track_id: u32,
        decode_time: u64,
        sample_duration: u32,
        sample_composition_time_offset: i64,
        sample_flags: SampleFlags,
        sample_description_index: u32,
        data: Vec<u8>,
    ) -> Self {
        Self {
            track_id,
            decode_time,
            sample_duration,
            sample_composition_time_offset,
            sample_flags,
            sample_description_index,
            data,
        }
    }

    /// Returns the track this sample belongs to
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns when this sample is decoded, in the time scale of its track
    #[must_use]
    pub const fn decode_time(&self) -> u64 {
        self.decode_time
    }

    /// Returns how long this sample lasts, in the time scale of its track
    #[must_use]
    pub const fn sample_duration(&self) -> u32 {
        self.sample_duration
    }

    /// Returns the offset from the decode time of this sample to its composition time
    #[must_use]
    pub const fn sample_composition_time_offset(&self) -> i64 {
        self.sample_composition_time_offset
    }

    /// Returns the flags of this sample, which state how it may be decoded
    #[must_use]
    pub const fn sample_flags(&self) -> SampleFlags {
        self.sample_flags
    }

    /// Returns the `stsd` entry this sample is described by
    #[must_use]
    pub const fn sample_description_index(&self) -> u32 {
        self.sample_description_index
    }

    /// Returns the bytes this sample is carried as
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Takes the bytes this sample is carried as, leaving the properties behind
    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

/// One sample of one track as the file declares it, and where its bytes lie
///
/// A file declares its samples in one of two forms, the sample table of a
/// movie (ISO/IEC 14496-12 §8.7) or the track runs of a movie fragment (§8.8),
/// and either resolves to this: the properties a [`Sample`] carries, the data
/// reference that names the resource its bytes lie in (§8.7.2), and the extent
/// of that resource they occupy — a contiguous subset of its bytes, §8.11.3,
/// counted from its first byte. A `Sample` is made of one of these and the
/// bytes of the extent.
///
/// The `data_reference_index` is the one the sample entry of the sample states
/// (§8.5.2.3), counted from one over the entries of `dref` (§8.7.2). A movie
/// carrying its media in the file itself states one entry, which names that
/// file.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleExtent {
    track_id: u32,
    decode_time: u64,
    sample_duration: u32,
    sample_composition_time_offset: i64,
    sample_flags: SampleFlags,
    sample_description_index: u32,
    data_reference_index: u16,
    extent: Range<u64>,
}

impl SampleExtent {
    /// Creates the extent from the properties settled for the sample and where its bytes lie
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "every field is a settled fact of the sample, and a constructor that took fewer would leave one unsettled"
    )]
    pub const fn new(
        track_id: u32,
        decode_time: u64,
        sample_duration: u32,
        sample_composition_time_offset: i64,
        sample_flags: SampleFlags,
        sample_description_index: u32,
        data_reference_index: u16,
        extent: Range<u64>,
    ) -> Self {
        Self {
            track_id,
            decode_time,
            sample_duration,
            sample_composition_time_offset,
            sample_flags,
            sample_description_index,
            data_reference_index,
            extent,
        }
    }

    /// Returns the track the sample belongs to
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns when the sample is decoded, in the time scale of its track
    #[must_use]
    pub const fn decode_time(&self) -> u64 {
        self.decode_time
    }

    /// Returns how long the sample lasts, in the time scale of its track
    #[must_use]
    pub const fn sample_duration(&self) -> u32 {
        self.sample_duration
    }

    /// Returns the offset from the decode time of the sample to its composition time
    #[must_use]
    pub const fn sample_composition_time_offset(&self) -> i64 {
        self.sample_composition_time_offset
    }

    /// Returns the flags of the sample, which state how it may be decoded
    #[must_use]
    pub const fn sample_flags(&self) -> SampleFlags {
        self.sample_flags
    }

    /// Returns the `stsd` entry the sample is described by
    #[must_use]
    pub const fn sample_description_index(&self) -> u32 {
        self.sample_description_index
    }

    /// Returns the `dref` entry naming the resource the bytes of the sample lie in
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// Returns the bytes of the resource the sample occupies
    #[inline]
    #[must_use]
    pub fn extent(&self) -> Range<u64> {
        self.extent.clone()
    }
}
