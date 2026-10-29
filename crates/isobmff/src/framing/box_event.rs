//! [`BoxEvent`], a step through the boxes of a file that reading reports and writing takes

use isobmff_core::BoxHeader;

/// Step through the boxes of a file, ISO/IEC 14496-12 §4.2.2
///
/// [`BoxReader`](super::BoxReader) reports a file as these events, and
/// [`BoxWriter`](super::BoxWriter) writes a file from them. A box is its
/// [`Header`](Self::Header), the pieces of its payload as
/// [`Payload`](Self::Payload)s, then its [`End`](Self::End). Where a box lies
/// in the file is asked of the reader or the writer, not carried here.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoxEvent<'a> {
    /// Header that opens a box
    Header(BoxHeader),
    /// Piece of the payload of the box opened last
    Payload(&'a [u8]),
    /// Last byte of the box opened last
    End,
}

impl BoxEvent<'_> {
    /// Returns which of the events this is, without what it carries
    #[must_use]
    pub const fn kind(&self) -> BoxEventKind {
        match self {
            Self::Header(_) => BoxEventKind::Header,
            Self::Payload(_) => BoxEventKind::Payload,
            Self::End => BoxEventKind::End,
        }
    }
}

/// Which of the [`BoxEvent`]s an event is, without what it carries
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BoxEventKind {
    /// [`BoxEvent::Header`]
    Header,
    /// [`BoxEvent::Payload`]
    Payload,
    /// [`BoxEvent::End`]
    End,
}
