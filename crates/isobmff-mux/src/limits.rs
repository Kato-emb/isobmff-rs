//! [`DemuxLimits`], how much of what a file declares reading it allocates for

use isobmff::media_data::MediaDataLimits;
use isobmff::structure_data::StructureDataLimits;

/// Limits a [`Demuxer`](crate::Demuxer) reads a file up to, and a [`MediaDataReader`](isobmff::media_data::MediaDataReader) holds its samples up to
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
/// | Leniencies kept, over the whole file | 65,536 |
/// | Sample locations a `MediaDataReader` holds | 1,048,576 |
/// | Bytes of media data of one sample | 16 MiB |
/// | Bytes of media data allocated for the samples not yet taken, together | 16 MiB |
/// | Compressed boxes kept the places of, over the whole file (§8.16) | 65,536 |
///
/// The locations of the samples of a `moov` or a `moof` are resolved into one
/// list before the first of them is returned, so the samples it declares
/// allocate up to `sample_count × size_of::<SampleLocation>()` bytes for it,
/// besides the limits above.
///
/// # Examples
///
/// ```
/// use isobmff::media_data::MediaDataReader;
/// use isobmff::structure_data::StructureDataReader;
/// use isobmff_mux::{DemuxLimits, Demuxer};
///
/// const LIMITS: DemuxLimits = DemuxLimits::new()
///     .with_payload_bytes(64 * 1024 * 1024)
///     .with_held_bytes(64 * 1024 * 1024);
///
/// // The demuxer, or a reader of its own made of the layers, up to the same limits
/// let demuxer = Demuxer::with_limits(LIMITS);
/// let structure_data = StructureDataReader::new(LIMITS.structure_data());
/// let media_data = MediaDataReader::new(LIMITS.media_data());
/// assert_eq!(LIMITS.payload_bytes(), 64 * 1024 * 1024);
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
                leniency_count: 65_536,
                identified_media_data_count: 65_536,
                unresolved_sample_count: 1_048_576,
                compressed_box_count: 65_536,
            },
            media_data: MediaDataLimits {
                held_location_count: 1_048_576,
                sample_bytes: 16 * 1024 * 1024,
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

    /// Replaces the number of compressed boxes the demuxer keeps the places of, over the whole file (§8.16)
    #[must_use]
    pub const fn with_compressed_box_count(mut self, compressed_box_count: u64) -> Self {
        self.structure_data.compressed_box_count = compressed_box_count;
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

    /// Replaces the number of leniencies the demuxer keeps, over the whole file
    #[must_use]
    pub const fn with_leniency_count(mut self, leniency_count: u64) -> Self {
        self.structure_data.leniency_count = leniency_count;
        self
    }

    /// Replaces the number of `IdentifiedMediaDataBox`es the demuxer keeps the places of, over the whole file
    #[must_use]
    pub const fn with_identified_media_data_count(
        mut self,
        identified_media_data_count: u64,
    ) -> Self {
        self.structure_data.identified_media_data_count = identified_media_data_count;
        self
    }

    /// Replaces the number of samples whose `IdentifiedMediaDataBox` is not read yet the demuxer holds
    #[must_use]
    pub const fn with_unresolved_sample_count(mut self, unresolved_sample_count: u64) -> Self {
        self.structure_data.unresolved_sample_count = unresolved_sample_count;
        self
    }

    /// Replaces the number of sample locations a `MediaDataReader` holds
    #[must_use]
    pub const fn with_held_location_count(mut self, held_location_count: u64) -> Self {
        self.media_data.held_location_count = held_location_count;
        self
    }

    /// Replaces the bytes of media data of one sample a `MediaDataReader` holds
    #[must_use]
    pub const fn with_sample_bytes(mut self, sample_bytes: u64) -> Self {
        self.media_data.sample_bytes = sample_bytes;
        self
    }

    /// Replaces the bytes of media data a `MediaDataReader` holds allocated for the samples not yet taken, together
    #[must_use]
    pub const fn with_held_bytes(mut self, held_bytes: u64) -> Self {
        self.media_data.held_bytes = held_bytes;
        self
    }

    /// Returns the limits to create a [`MediaDataReader`](isobmff::media_data::MediaDataReader) with
    #[must_use]
    pub const fn media_data(self) -> MediaDataLimits {
        self.media_data
    }

    /// Returns the bytes of the payload of one box the demuxer gathers into its value, the limit to gather a box with [`BoxReader::gather`](isobmff::framing::BoxReader::gather)
    #[must_use]
    pub const fn payload_bytes(self) -> u64 {
        self.payload_bytes
    }

    /// Returns the limits to create a [`StructureDataReader`](isobmff::structure_data::StructureDataReader) with
    #[must_use]
    pub const fn structure_data(self) -> StructureDataLimits {
        self.structure_data
    }
}

impl Default for DemuxLimits {
    fn default() -> Self {
        Self::new()
    }
}

/// Limits a [`Muxer`](crate::Muxer) writes a file up to
///
/// [`new`](Self::new) sets every limit to a default, and each `with_` method
/// replaces one of them.
///
/// | Limit | Default |
/// |---|---|
/// | Samples whose locations a recoverable file keeps, to declare them as it finishes | 1,048,576 |
///
/// # Examples
///
/// ```
/// use isobmff_mux::{Muxer, MuxLimits, NonFragmentedOptions};
///
/// let mut options = NonFragmentedOptions::default();
/// options.recoverable = true;
/// let muxer = Muxer::non_fragmented(options).with_limits(MuxLimits::new().with_held_location_count(1 << 24));
/// assert_eq!(MuxLimits::new().held_location_count(), 1_048_576);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MuxLimits {
    held_location_count: u64,
}

impl MuxLimits {
    /// Creates the limits of the defaults
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held_location_count: 1_048_576,
        }
    }

    /// Replaces the number of samples whose locations a recoverable file keeps
    #[must_use]
    pub const fn with_held_location_count(mut self, held_location_count: u64) -> Self {
        self.held_location_count = held_location_count;
        self
    }

    /// Returns the number of samples whose locations a recoverable file keeps, to declare them in the sample tables as it finishes
    ///
    /// The locations are kept as the rows of the sample tables, a few tens
    /// of bytes each. A file past the limit is not laid out again: the muxer
    /// keeps no more of them, writes the rest of the samples as fragments,
    /// and [`finish`](crate::Muxer::finish) fails with
    /// [`NotFinalised`](crate::MuxError::NotFinalised).
    #[must_use]
    pub const fn held_location_count(self) -> u64 {
        self.held_location_count
    }
}

impl Default for MuxLimits {
    fn default() -> Self {
        Self::new()
    }
}
