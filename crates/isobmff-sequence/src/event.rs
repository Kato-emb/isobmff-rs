//! [`BoxEvent`] and [`BoxEventAt`], one step of the sequence of boxes of ISO/IEC 14496-12 §4.2

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentBox, SegmentTypeBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader, BoxType, DecodeError};

/// Step of the sequence of boxes, owning the box or the bytes it carries
///
/// A box read into a value is one step, once the box is whole: `ftyp`, `styp`,
/// `moov`, and `moof`. Every other box is [`RawStart`](Self::RawStart), then as
/// many [`RawPayload`](Self::RawPayload) steps as its payload was cut into, then
/// [`RawEnd`](Self::RawEnd). A container among them is one box like any other:
/// its payload is carried whole rather than descended into.
///
/// A step says what the file holds and not where it holds it:
/// [`BoxReader`](crate::BoxReader) reports it wrapped in the [`BoxEventAt`] that
/// names where it begins, and [`BoxWriter`](crate::BoxWriter) takes it on its
/// own.
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

/// Step of the sequence of boxes, and where in the file it begins
///
/// The offset counts from the first byte handed to the reader, and points at the
/// first byte of what the step carries: the header of the box for a value and
/// for [`RawStart`](BoxEvent::RawStart), the first byte of that part for
/// [`RawPayload`](BoxEvent::RawPayload), and the byte just past the box for
/// [`RawEnd`](BoxEvent::RawEnd).
///
/// It is what a sample layer resolves the offsets a box declares against, and it
/// is the reader's report alone: nothing hands it to
/// [`BoxWriter`](crate::BoxWriter), which counts the bytes it lays down itself.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct BoxEventAt {
    file_offset: u64,
    event: BoxEvent,
}

impl BoxEventAt {
    /// Creates the step `event` beginning `file_offset` bytes into the file
    #[must_use]
    pub const fn new(file_offset: u64, event: BoxEvent) -> Self {
        Self { file_offset, event }
    }

    /// Returns the bytes the file carried before this step
    #[must_use]
    pub const fn file_offset(&self) -> u64 {
        self.file_offset
    }

    /// Returns the step itself
    #[must_use]
    pub const fn event(&self) -> &BoxEvent {
        &self.event
    }

    /// Takes the step out, leaving the offset behind
    #[must_use]
    pub fn into_event(self) -> BoxEvent {
        self.event
    }
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
    pub(crate) fn read(self, payload: &[u8]) -> Result<BoxEvent, DecodeError> {
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
