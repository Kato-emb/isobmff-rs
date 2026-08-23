//! [`Sample`], the unit a presentation carries its media in

use alloc::vec::Vec;

/// One sample of one track, with the bytes it is carried as
///
/// A sample is what a presentation is made of — ISO/IEC 14496-12 §3.1.14 has it
/// as all the data associated with a single timestamp. What the fragments state
/// about it is resolved before it is handed over: the properties a `trun` row
/// leaves unstated are taken from the `tfhd` of its fragment, and what that
/// leaves unstated from the `trex` of its track (§8.8.7, §8.8.8), so every
/// field here is settled.
///
/// The times are measured in the time scale of the track, the one its `mdhd`
/// declares (§8.4.2). They are not converted: a caller placing samples of two
/// tracks on one timeline reads that time scale off the `moov` itself.
///
/// The `sample_flags` are carried as the wire holds them, the bit layout of
/// §8.8.3.1.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Sample {
    track_id: u32,
    decode_time: u64,
    sample_duration: u32,
    sample_composition_time_offset: Option<i64>,
    sample_flags: u32,
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
        sample_composition_time_offset: Option<i64>,
        sample_flags: u32,
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
    pub const fn sample_composition_time_offset(&self) -> Option<i64> {
        self.sample_composition_time_offset
    }

    /// Returns the flags of this sample, which state how it may be decoded
    #[must_use]
    pub const fn sample_flags(&self) -> u32 {
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
}
