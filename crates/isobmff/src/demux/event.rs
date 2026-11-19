//! What a [`Demuxer`](super::Demuxer) reports

use isobmff_boxes::{
    FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox,
    SegmentTypeBox,
};
use isobmff_core::BoxHeader;

use super::error::Error;
use crate::SampleLocation;
use crate::media_data::Sample;

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
    /// A `sidx` of the file, read into its value, ISO/IEC 14496-12 §8.14.3;
    /// [`subsegments`](crate::structure_data::subsegments) resolves it to
    /// where its subsegments lie and when they are presented
    SegmentIndex {
        /// File offset of the first byte after the box, the anchor point its
        /// offsets count from
        anchor: u64,
        /// The box
        sidx: SegmentIndexBox,
    },
    /// The `mfra` of the file, read into its value, ISO/IEC 14496-12 §8.8.9:
    /// where sync samples lie, which is a hint only, the movie fragments being
    /// definitive; the demuxer does not seek by it
    MovieFragmentRandomAccess(MovieFragmentRandomAccessBox),
    /// Where the bytes of a sample of the `moov` or of a `moof` lie, for
    /// every sample of every track, in the order of their file offsets
    /// within the box
    ///
    /// A sample that lies in an `imda` not yet read follows that `imda`
    /// instead, reported as [`Event::Box`], in the order of the file offsets
    /// of the samples that lie in it. The offset of a sample whose data
    /// reference entry names another file is one of that file, ISO/IEC
    /// 14496-12 §8.7.5.1: the entry is the one of the `dref` of its track that
    /// the `data_reference_index` of its sample entry, the
    /// `sample_description_index`-th of the `stsd`, names.
    SampleLocation(SampleLocation),
    /// A sample of a subscribed track, its bytes lent from the demuxer's
    /// buffer
    Sample(Sample<'a>),
    /// A box at file level the demuxer does not read into a value, such as an
    /// `mdat`; its payload is not requested, but for the identifier of an
    /// `imda`
    Box {
        /// File offset of the box
        offset: u64,
        /// Header of the box
        header: BoxHeader,
    },
    /// What cannot be read; demuxing goes on where the error says
    Error(Error),
}
