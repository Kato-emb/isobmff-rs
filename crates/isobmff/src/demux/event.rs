//! What a [`Demuxer`](super::Demuxer) reports

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SegmentTypeBox};
use isobmff_core::BoxHeader;

use super::error::Error;
use crate::{SampleLocation, SampleProperties};

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
