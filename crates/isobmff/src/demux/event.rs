//! What a [`Demuxer`](super::Demuxer) reports

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_core::BoxHeader;

use super::error::Error;
use crate::SampleLocation;

/// What a [`Demuxer`](super::Demuxer) found in its input
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The `ftyp` of the file, read into its value
    FileType(FileTypeBox),
    /// The `moov` of the file, read into its value; the location of each of
    /// its samples follows as [`Event::SampleLocation`]
    Movie(alloc::boxed::Box<MovieBox>),
    /// Where the bytes of a sample of the `moov` lie, for every sample of
    /// every track, in the order of their offsets
    ///
    /// The `data_origin` is [`Container`](crate::DataOrigin::Container): the
    /// offset counts from the start of the `container`, which is
    /// [`SameFile`](crate::MediaDataContainer::SameFile), where it is a file
    /// offset, or [`Resource`](crate::MediaDataContainer::Resource), where it
    /// is an offset in the resource the data reference entry names. The
    /// demuxer requests the bytes of no sample.
    SampleLocation(SampleLocation),
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
