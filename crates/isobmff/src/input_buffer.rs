//! The input a reader holds, lent to the events it returns
//!
//! A reader copies the input handed over into one buffer, cuts boxes off its
//! front, and lends an event a range of it until the next poll. This is that
//! buffer, with the rule for when the cut bytes are dropped.

use alloc::vec::Vec;
use core::ops::Range;

/// The input handed to a reader and not yet dropped
#[derive(Debug, Clone)]
pub(crate) struct InputBuffer {
    bytes: Vec<u8>,
    /// Bytes at the front of `bytes` that were reported or cut; a range an
    /// event lends can still point at them
    consumed: usize,
    /// The input was ended
    ended: bool,
}

impl InputBuffer {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            consumed: 0,
            ended: false,
        }
    }

    /// Appends input; the ranges already lent keep pointing at the same bytes
    ///
    /// # Panics
    /// Panics if called after [`InputBuffer::finish`].
    pub(crate) fn extend(&mut self, bytes: &[u8]) {
        assert!(!self.ended, "input handed over after finish");
        // Why copy every chunk, not only the tail that crosses into the next:
        // an event borrows the reader rather than the caller's chunk, so
        // poll_event can lend a box after handle_input has returned.
        self.bytes.extend_from_slice(bytes);
    }

    /// Ends the input at the bytes handed over so far
    pub(crate) fn finish(&mut self) {
        self.ended = true;
    }

    pub(crate) const fn is_ended(&self) -> bool {
        self.ended
    }

    /// Drops the bytes reported or cut, if they are no fewer than the bytes
    /// left, and returns how many it dropped
    pub(crate) fn compact(&mut self) -> usize {
        // Why not a VecDeque, which drops the front without moving the rest:
        // a payload straddling its wrap has no single slice to lend. Why not
        // compact whenever bytes were consumed: that moves the unread rest of
        // a large chunk on every box. Compacting only once the consumed bytes
        // are no fewer than the rest moves each byte a bounded number of
        // times in all.
        if self.consumed < self.bytes.len().saturating_sub(self.consumed) {
            return 0;
        }
        let dropped = self.consumed;
        self.bytes.drain(..dropped);
        self.consumed = 0;
        dropped
    }

    /// Returns the bytes of a range an event lends
    pub(crate) fn get(&self, range: Range<usize>) -> &[u8] {
        self.bytes.get(range).unwrap_or_default()
    }

    /// Returns the bytes not yet reported or cut
    pub(crate) fn unconsumed(&self) -> &[u8] {
        self.bytes.get(self.consumed..).unwrap_or_default()
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) const fn consumed(&self) -> usize {
        self.consumed
    }

    /// Marks `length` more bytes at the front as reported or cut
    pub(crate) fn consume(&mut self, length: usize) {
        self.consumed = self.consumed.saturating_add(length).min(self.bytes.len());
    }
}
