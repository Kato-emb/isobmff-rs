//! [`BoxEvent`], one step of the sequence of boxes of ISO/IEC 14496-12 §4.2

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SegmentTypeBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader, BoxType, DecodeError};

/// Step of the sequence of boxes, owning the box or the bytes it carries
///
/// A box the reader reads into a value is reported as that value, once the box
/// is whole: `ftyp`, `styp`, `moov`, and `moof`. Every other box appears as
/// [`RawStart`](Self::RawStart), then as many [`RawPayload`](Self::RawPayload)
/// events as the input cut its payload into, then [`RawEnd`](Self::RawEnd). A
/// container among them is reported as one box like any other: its payload is
/// passed on whole rather than descended into.
///
/// Every event carries the offset the box it reports begins at, which is what a
/// sample layer resolves the offsets a box declares against.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum BoxEvent {
    /// Brands a file declares itself readable as, and where in the file the box begins
    FileType {
        /// Box the payload read into
        ftyp: FileTypeBox,
        /// Bytes the file carried before the header of this box
        file_offset: u64,
    },
    /// Brands a segment declares itself readable as, and where in the file the box begins
    SegmentType {
        /// Box the payload read into
        styp: SegmentTypeBox,
        /// Bytes the file carried before the header of this box
        file_offset: u64,
    },
    /// Metadata of the presentation a file holds, and where in the file the box begins
    Movie {
        /// Box the payload read into
        moov: MovieBox,
        /// Bytes the file carried before the header of this box
        file_offset: u64,
    },
    /// Metadata of one fragment of a presentation, and where in the file the box begins
    MovieFragment {
        /// Box the payload read into
        moof: MovieFragmentBox,
        /// Bytes the file carried before the header of this box
        file_offset: u64,
    },
    /// Header of a box, whole, and where in the file that box begins
    RawStart {
        /// Header of the box, however the input cut across it
        header: BoxHeader,
        /// Bytes the file carried before the header of this box
        file_offset: u64,
    },
    /// Part of the payload of the box that started, as it lay in the input
    RawPayload(Vec<u8>),
    /// End of the box that started, its declared total reached
    RawEnd,
}

/// Box the reader reads into a value rather than passing on as it lies
#[derive(Clone, Copy, Debug)]
pub(crate) enum ValueBox {
    /// [`FileTypeBox`] (`ftyp`)
    FileType,
    /// [`SegmentTypeBox`] (`styp`)
    SegmentType,
    /// [`MovieBox`] (`moov`)
    Movie,
    /// [`MovieFragmentBox`] (`moof`)
    MovieFragment,
}

impl ValueBox {
    /// Returns the box `box_type` names, when it is one that reads into a value
    pub(crate) fn of(box_type: BoxType) -> Option<Self> {
        if box_type == FileTypeBox::BOX_TYPE {
            Some(Self::FileType)
        } else if box_type == SegmentTypeBox::BOX_TYPE {
            Some(Self::SegmentType)
        } else if box_type == MovieBox::BOX_TYPE {
            Some(Self::Movie)
        } else if box_type == MovieFragmentBox::BOX_TYPE {
            Some(Self::MovieFragment)
        } else {
            None
        }
    }

    /// Reads the payload of the box, whole, into the event that carries it
    ///
    /// # Errors
    ///
    /// The failure [`BoxDecode::decode_payload`] reports for the box `self`
    /// names.
    pub(crate) fn read(self, payload: &[u8], file_offset: u64) -> Result<BoxEvent, DecodeError> {
        match self {
            Self::FileType => Ok(BoxEvent::FileType {
                ftyp: FileTypeBox::decode_payload(payload)?,
                file_offset,
            }),
            Self::SegmentType => Ok(BoxEvent::SegmentType {
                styp: SegmentTypeBox::decode_payload(payload)?,
                file_offset,
            }),
            Self::Movie => Ok(BoxEvent::Movie {
                moov: MovieBox::decode_payload(payload)?,
                file_offset,
            }),
            Self::MovieFragment => Ok(BoxEvent::MovieFragment {
                moof: MovieFragmentBox::decode_payload(payload)?,
                file_offset,
            }),
        }
    }
}
