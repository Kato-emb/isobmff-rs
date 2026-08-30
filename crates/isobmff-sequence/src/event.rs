//! [`BoxEvent`], one step of the sequence of boxes of ISO/IEC 14496-12 §4.2

use alloc::vec::Vec;

use isobmff_core::BoxHeader;

/// Step of the sequence of boxes, owning the bytes it carries
///
/// A box is one [`Header`](Self::Header), then as many
/// [`Payload`](Self::Payload) steps as its payload was cut into, then
/// [`End`](Self::End). A container is one box like any other: its payload is
/// carried as it lies rather than descended into, and so is the payload of a
/// box a specification reads into a value.
///
/// The bytes one step is made of: the header alone for a
/// [`Header`](Self::Header), that part of the payload for a
/// [`Payload`](Self::Payload), and none at all for an [`End`](Self::End), which
/// stands where the box ended. The steps of a file lie end to end, so together
/// they measure it.
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
    /// Header of the box that started
    Header(BoxHeader),
    /// Part of the payload of the box that started, as it lay in the input
    Payload(Vec<u8>),
    /// End of the box that started, its declared total reached
    End,
}
