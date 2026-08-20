//! [`BoxEvent`], one step of the sequence of boxes of ISO/IEC 14496-12 §4.2

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SegmentTypeBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader, BoxType, Error};

/// Step of the sequence of boxes, owning the box or the bytes it carries
///
/// A box read into a value is one step, once the box is whole: `ftyp`, `styp`,
/// `moov`, and `moof`. Every other box is [`RawStart`](Self::RawStart), then as
/// many [`RawPayload`](Self::RawPayload) steps as its payload was cut into, then
/// [`RawEnd`](Self::RawEnd). A container among them is one box like any other:
/// its payload is carried whole rather than descended into.
///
/// The bytes one step is made of: the whole box for a value, the header alone
/// for a [`RawStart`](Self::RawStart), that part of the payload for a
/// [`RawPayload`](Self::RawPayload), and none at all for a
/// [`RawEnd`](Self::RawEnd), which stands where the box ended. The steps of a
/// file lie end to end, so together they measure it.
///
/// A step says what the file holds and not where it holds it, so the same step
/// is what [`BoxReader`](crate::BoxReader) reports and what
/// [`BoxWriter`](crate::BoxWriter) takes. Where it lies is the extent each of
/// them names for the step it last handled —
/// [`BoxReader::event_extent`](crate::BoxReader::event_extent) and
/// [`BoxWriter::event_extent`](crate::BoxWriter::event_extent).
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum BoxEvent {
    /// Brands a file declares itself readable as
    FileType(FileTypeBox),
    /// Brands a segment declares itself readable as
    SegmentType(SegmentTypeBox),
    /// Metadata of the presentation a file holds
    Movie(MovieBox),
    /// Metadata of one fragment of a presentation
    MovieFragment(MovieFragmentBox),
    /// Header of a box carried as it lies, whole
    RawStart(BoxHeader),
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
    pub(crate) fn read(self, payload: &[u8]) -> Result<BoxEvent, Error> {
        match self {
            Self::FileType => Ok(BoxEvent::FileType(FileTypeBox::decode_payload(payload)?)),
            Self::SegmentType => Ok(BoxEvent::SegmentType(SegmentTypeBox::decode_payload(
                payload,
            )?)),
            Self::Movie => Ok(BoxEvent::Movie(MovieBox::decode_payload(payload)?)),
            Self::MovieFragment => Ok(BoxEvent::MovieFragment(MovieFragmentBox::decode_payload(
                payload,
            )?)),
        }
    }
}
