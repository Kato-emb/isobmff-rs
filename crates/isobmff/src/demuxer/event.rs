//! What a [`Demuxer`](super::Demuxer) reports

use core::fmt;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SegmentTypeBox};
use isobmff_core::{BoxHeader, BoxType};

use crate::{SampleLocation, SampleProperties, framing, structure_data};

/// What a [`Demuxer`](super::Demuxer) found in its input
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum Event<'a> {
    /// The `ftyp` of the file, read into its value
    FileType(FileTypeBox),
    /// An `styp` of the file, read into its value: the brands of the segment
    /// it opens, ISO/IEC 14496-12 §8.14.2
    SegmentType(SegmentTypeBox),
    /// The `moov` of the file, read into its value; the location of each of
    /// its samples follows as [`Event::SampleLocation`]
    Movie(alloc::boxed::Box<MovieBox>),
    /// A `moof` of the file, read into its value; the location of each of
    /// the samples it adds follows as [`Event::SampleLocation`]
    MovieFragment(alloc::boxed::Box<MovieFragmentBox>),
    /// Where the bytes of a sample of the `moov` or of a `moof` lie, for
    /// every sample of every track, in the order of their file offsets
    /// within the box
    SampleLocation(SampleLocation),
    /// A sample of a subscribed track, its bytes lent from the demuxer's
    /// buffer
    Sample(Sample<'a>),
    /// A box at file level the demuxer does not read into a value, such as an
    /// `mdat`; its payload is not requested
    Box {
        /// File offset of the box
        offset: u64,
        /// Header of the box
        header: BoxHeader,
    },
    /// What cannot be read; demuxing goes on where the error says
    Error(Error),
}

/// A `sample` of a subscribed track, with its bytes
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sample<'a> {
    /// What the structure-data declares of the sample
    pub properties: SampleProperties,
    /// File offset of the first byte of the sample
    pub offset: u64,
    /// The bytes of the sample
    pub bytes: &'a [u8],
}

impl Sample<'_> {
    /// Returns where the bytes of the sample lie
    #[must_use]
    pub fn location(&self) -> SampleLocation {
        SampleLocation {
            properties: self.properties,
            offset: self.offset,
            size: u32::try_from(self.bytes.len()).unwrap_or(u32::MAX),
        }
    }
}

/// What a [`Demuxer`](super::Demuxer) cannot read
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// A box at file level cannot be framed
    Framing(framing::Error),
    /// A box the demuxer reads into a value does not read
    Decode {
        /// File offset of the box
        offset: u64,
        /// Type of the box
        box_type: BoxType,
        /// Why the box does not read
        source: isobmff_core::Error,
    },
    /// The samples a `moov` or a `moof` declares cannot be resolved as the
    /// specification reads them; the box and the location of each of its
    /// samples follow where they resolve when read lossily, and none
    /// otherwise
    StructureData {
        /// File offset of the box
        offset: u64,
        /// Type of the box, `moov` or `moof`
        box_type: BoxType,
        /// Why its samples cannot be resolved
        source: structure_data::Error,
    },
    /// A `moov` follows the one already read, which a file holds one of,
    /// ISO/IEC 14496-12 §8.2.1; it is not read, and the first is kept
    DuplicateMovie {
        /// File offset of the `moov` that follows
        offset: u64,
    },
    /// A sample of a subscribed track is larger than
    /// [`DemuxerConfig::sample_size_limit`](super::DemuxerConfig::sample_size_limit);
    /// its bytes are not requested, and demuxing goes on at the next sample
    SampleOverLimit {
        /// Where the sample lies
        location: SampleLocation,
        /// The limit the sample is larger than
        limit: u64,
    },
    /// The input ends inside a sample of a subscribed track
    TruncatedSample {
        /// Where the sample lies
        location: SampleLocation,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(_) => write!(formatter, "a box at file level cannot be framed"),
            Self::Decode {
                offset, box_type, ..
            } => write!(
                formatter,
                "the box `{box_type}` at offset {offset} does not read"
            ),
            Self::StructureData {
                offset, box_type, ..
            } => write!(
                formatter,
                "the samples of the `{box_type}` at offset {offset} cannot be resolved as the specification reads them"
            ),
            Self::DuplicateMovie { offset } => write!(
                formatter,
                "the `moov` at offset {offset} follows the one already read"
            ),
            Self::SampleOverLimit { location, limit } => write!(
                formatter,
                "the sample of track {} at offset {} is {} bytes, over the limit of {limit}",
                location.properties.track_id, location.offset, location.size
            ),
            Self::TruncatedSample { location } => write!(
                formatter,
                "the input ends inside the sample of track {} at offset {}",
                location.properties.track_id, location.offset
            ),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Framing(source) => Some(source),
            Self::Decode { source, .. } => Some(source),
            Self::StructureData { source, .. } => Some(source),
            Self::DuplicateMovie { .. }
            | Self::SampleOverLimit { .. }
            | Self::TruncatedSample { .. } => None,
        }
    }
}
