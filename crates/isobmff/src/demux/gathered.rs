//! [`Gathered`], a box the box reader gathered whole, read into its value

use alloc::boxed::Box;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SegmentIndexBox, SegmentTypeBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxType, Error};

/// A box the demuxer gathers, read into its value
pub(super) enum Gathered {
    FileType(FileTypeBox),
    SegmentType(SegmentTypeBox),
    Movie(Box<MovieBox>),
    MovieFragment(Box<MovieFragmentBox>),
    SegmentIndex(SegmentIndexBox),
}

impl Gathered {
    /// The box types the demuxer gathers
    pub(super) const BOX_TYPES: [BoxType; 5] = [
        FileTypeBox::BOX_TYPE,
        SegmentTypeBox::BOX_TYPE,
        MovieBox::BOX_TYPE,
        MovieFragmentBox::BOX_TYPE,
        SegmentIndexBox::BOX_TYPE,
    ];

    /// Reads `payload`, that of a box of `box_type`, one of [`Gathered::BOX_TYPES`], into its value
    ///
    /// # Errors
    /// * The error the box does not read with; a `box_type` the demuxer
    ///   does not gather reads as the `moov`.
    pub(super) fn decode(box_type: BoxType, payload: &[u8]) -> Result<Self, Error> {
        Ok(if box_type == FileTypeBox::BOX_TYPE {
            Self::FileType(FileTypeBox::decode_payload(payload)?)
        } else if box_type == SegmentTypeBox::BOX_TYPE {
            Self::SegmentType(SegmentTypeBox::decode_payload(payload)?)
        } else if box_type == MovieFragmentBox::BOX_TYPE {
            Self::MovieFragment(Box::new(MovieFragmentBox::decode_payload(payload)?))
        } else if box_type == SegmentIndexBox::BOX_TYPE {
            Self::SegmentIndex(SegmentIndexBox::decode_payload(payload)?)
        } else {
            Self::Movie(Box::new(MovieBox::decode_payload(payload)?))
        })
    }
}
