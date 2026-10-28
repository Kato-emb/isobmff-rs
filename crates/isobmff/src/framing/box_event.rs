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

/// Events the bytes given to a [`BoxReader`](super::BoxReader) hold, in file order
///
/// Returned by [`BoxReader::handle_input`](super::BoxReader::handle_input) and
/// [`BoxReader::finish`](super::BoxReader::finish). They borrow the input,
/// never the reader.
#[derive(Clone, Debug, Default)]
#[must_use = "the events open and close boxes, and a header waits for an answer"]
pub struct BoxEvents<'a> {
    // Why not a `Vec`: the events of one call stop at the first `Header`, after
    // one `End`, and at most one `Payload` before it, so three slots always
    // hold them and a call allocates nothing.
    events: [Option<BoxEvent<'a>>; 3],
}

impl<'a> BoxEvents<'a> {
    pub(super) fn push(&mut self, event: BoxEvent<'a>) {
        let slot = self.events.iter_mut().find(|slot| slot.is_none());
        debug_assert!(slot.is_some(), "more events than a call can stop at");
        if let Some(slot) = slot {
            *slot = Some(event);
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.events.iter().all(Option::is_none)
    }
}

impl<'a> Iterator for BoxEvents<'a> {
    type Item = BoxEvent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.events.iter_mut().find_map(Option::take)
    }
}
