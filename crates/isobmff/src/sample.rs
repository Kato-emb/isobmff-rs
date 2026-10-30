//! [`Sample`], [`SampleProperties`] and [`SampleLocation`], what structure-data declares about a sample and where its media data lies

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::SampleFlags;

/// Values structure-data declares about one sample, ISO/IEC 14496-12 §8.8.7 and §8.8.8
///
/// A sample table (§8.5) and a track run (§8.8.8) declare the same values,
/// which a movie fragment lays out as one row: the names are those of the
/// fields of a track run, and of `tfhd` for `sample_description_index`. Times
/// and durations are in the timescale of the media of the track (§8.4.2).
///
/// # Examples
///
/// ```
/// use isobmff::SampleProperties;
/// use isobmff::boxes::SampleFlags;
///
/// // The first sample of track 1, a sync sample of 1001 ticks
/// let properties = SampleProperties {
///     track_id: 1,
///     decoding_time: 0,
///     sample_duration: 1001,
///     sample_composition_time_offset: 2002,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// assert!(!properties.sample_flags.sample_is_non_sync_sample);
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "the values are those a track run declares for a sample, and a caller writing samples names every one of them"
)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SampleProperties {
    /// `track_ID` of the track the sample is of (§8.3.2)
    pub track_id: u32,
    /// Decoding time of the sample, on the decoding timeline of the track (§8.6.1.2)
    pub decoding_time: u64,
    /// Duration of the sample, the difference between its decoding time and that of the next sample (§8.6.1.2)
    pub sample_duration: u32,
    /// Offset of the composition time of the sample from its decoding time (§8.6.1.3)
    pub sample_composition_time_offset: i64,
    /// Dependencies, padding, sync and degradation priority of the sample (§8.8.3.1)
    pub sample_flags: SampleFlags,
    /// Index, from 1, of the sample entry of the `stsd` that describes the sample (§8.5.2)
    pub sample_description_index: u32,
}

/// Sample: the values structure-data declares about it, and its media data, ISO/IEC 14496-12 A.4
///
/// # Examples
///
/// ```
/// use isobmff::boxes::SampleFlags;
/// use isobmff::{Sample, SampleProperties};
///
/// let properties = SampleProperties {
///     track_id: 1,
///     decoding_time: 0,
///     sample_duration: 1024,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// let sample = Sample::new(properties, vec![0x21, 0x10]);
/// assert_eq!(sample.properties().sample_duration, 1024);
/// assert_eq!(sample.data(), [0x21, 0x10]);
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Sample {
    properties: SampleProperties,
    data: Vec<u8>,
}

impl Sample {
    /// Creates the sample of `properties` whose media data is `data`
    #[must_use]
    pub const fn new(properties: SampleProperties, data: Vec<u8>) -> Self {
        Self { properties, data }
    }

    /// Returns the values structure-data declares about the sample
    #[must_use]
    pub const fn properties(&self) -> &SampleProperties {
        &self.properties
    }

    /// Returns the media data of the sample
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns the media data of the sample, giving up the rest
    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

/// Location of a sample: the values structure-data declares about it, and the data reference and byte range its media data lies at, ISO/IEC 14496-12 §6.1
///
/// The byte range counts in file offsets of the file the data reference
/// names: the data offsets structure-data declares are resolved from the
/// origin Table 1 of §6.1 gives them.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::SampleFlags;
/// use isobmff::{SampleLocation, SampleProperties};
///
/// let properties = SampleProperties {
///     track_id: 1,
///     decoding_time: 0,
///     sample_duration: 1024,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// // 371 bytes of media data at file offset 48, in the file itself
/// let location = SampleLocation::new(properties, 1, 48..419);
/// assert_eq!(location.data_reference_index(), 1);
/// assert_eq!(location.byte_range(), 48..419);
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleLocation {
    properties: SampleProperties,
    data_reference_index: u16,
    byte_range: Range<u64>,
}

impl SampleLocation {
    /// Creates the location of the sample of `properties` whose media data is `byte_range` of the file entry `data_reference_index` of the `dref` names
    #[must_use]
    pub const fn new(
        properties: SampleProperties,
        data_reference_index: u16,
        byte_range: Range<u64>,
    ) -> Self {
        Self {
            properties,
            data_reference_index,
            byte_range,
        }
    }

    /// Returns the values structure-data declares about the sample
    #[must_use]
    pub const fn properties(&self) -> &SampleProperties {
        &self.properties
    }

    /// Returns the index, from 1, of the entry of the `dref` that names the file the media data is in (§8.7.2)
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// Returns the file offsets of the media data of the sample
    #[must_use]
    pub fn byte_range(&self) -> Range<u64> {
        self.byte_range.clone()
    }
}
