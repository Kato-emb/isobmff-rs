//! [`DemuxLimits`], how much of what a file declares reading it allocates for

use crate::media_data::MediaDataLimits;
use crate::structure_data::StructureDataLimits;

/// Limits a [`Demuxer`](crate::Demuxer) reads a file up to, and a [`MediaDataReader`](crate::media_data::MediaDataReader) holds its samples up to
///
/// Each bounds what reading allocates for a value the file declares, and is
/// checked before the allocation. [`new`](Self::new) sets every limit to a
/// default that a movie of some hours fits, and each `with_` method replaces
/// one of them.
///
/// | Limit | Default |
/// |---|---|
/// | Bytes of the payload of one box gathered into its value (`moov`, `moof`, ...) | 16 MiB |
/// | Samples one `moov` or `moof` declares | 1,048,576 |
/// | Segment indexes kept | 65,536 |
/// | Subsegments the kept segment indexes reference, together | 1,048,576 |
/// | Sample locations a `MediaDataReader` holds | 1,048,576 |
/// | Bytes of media data the held locations name, together | 16 MiB |
///
/// # Examples
///
/// ```
/// use isobmff::media_data::MediaDataReader;
/// use isobmff::{DemuxLimits, Demuxer};
///
/// const LIMITS: DemuxLimits = DemuxLimits::new()
///     .with_payload_bytes(64 * 1024 * 1024)
///     .with_held_bytes(64 * 1024 * 1024);
///
/// let demuxer = Demuxer::with_limits(LIMITS);
/// let media_data = MediaDataReader::new(LIMITS.media_data());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DemuxLimits {
    payload_bytes: u64,
    structure_data: StructureDataLimits,
    media_data: MediaDataLimits,
}

impl DemuxLimits {
    /// Creates the limits of the defaults
    #[must_use]
    pub const fn new() -> Self {
        Self {
            payload_bytes: 16 * 1024 * 1024,
            structure_data: StructureDataLimits {
                sample_count: 1_048_576,
                segment_index_count: 65_536,
                subsegment_count: 1_048_576,
            },
            media_data: MediaDataLimits {
                held_location_count: 1_048_576,
                held_bytes: 16 * 1024 * 1024,
            },
        }
    }

    /// Replaces the bytes of the payload of one box the demuxer gathers into its value
    #[must_use]
    pub const fn with_payload_bytes(mut self, payload_bytes: u64) -> Self {
        self.payload_bytes = payload_bytes;
        self
    }

    /// Replaces the number of samples one `moov`, or one `moof`, is resolved up to
    #[must_use]
    pub const fn with_sample_count(mut self, sample_count: u64) -> Self {
        self.structure_data.sample_count = sample_count;
        self
    }

    /// Replaces the number of segment indexes the demuxer keeps
    #[must_use]
    pub const fn with_segment_index_count(mut self, segment_index_count: u64) -> Self {
        self.structure_data.segment_index_count = segment_index_count;
        self
    }

    /// Replaces the number of subsegments the segment indexes the demuxer keeps reference, together
    #[must_use]
    pub const fn with_subsegment_count(mut self, subsegment_count: u64) -> Self {
        self.structure_data.subsegment_count = subsegment_count;
        self
    }

    /// Replaces the number of sample locations a `MediaDataReader` holds
    #[must_use]
    pub const fn with_held_location_count(mut self, held_location_count: u64) -> Self {
        self.media_data.held_location_count = held_location_count;
        self
    }

    /// Replaces the bytes of media data the locations a `MediaDataReader` holds name, together
    #[must_use]
    pub const fn with_held_bytes(mut self, held_bytes: u64) -> Self {
        self.media_data.held_bytes = held_bytes;
        self
    }

    /// Returns the limits to create a [`MediaDataReader`](crate::media_data::MediaDataReader) with
    #[must_use]
    pub const fn media_data(self) -> MediaDataLimits {
        self.media_data
    }

    /// Returns the bytes of the payload of one box the demuxer gathers into its value
    pub(crate) const fn payload_bytes(self) -> u64 {
        self.payload_bytes
    }

    /// Returns the limits of the structure-data the demuxer resolves and keeps
    pub(crate) const fn structure_data(self) -> StructureDataLimits {
        self.structure_data
    }
}

impl Default for DemuxLimits {
    fn default() -> Self {
        Self::new()
    }
}
