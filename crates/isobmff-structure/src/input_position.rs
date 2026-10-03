//! [`InputPosition`], where the input a demux FSM takes in order stands in the file, and [`WantedInput`], the one read it wants next

use core::ops::Range;

/// The one read a demux FSM wants next: where in the file, and how much if it knows
///
/// [`offset`](Self::offset) is a file offset, counted from the first byte of
/// the file as a chunk offset or a base data offset is (ISO/IEC 14496-12
/// §8.7.5, §8.8.7). [`length`](Self::length) is `Some` for bytes the samples
/// lack that the input taken in order has already passed by — the whole of
/// what they lack from there — and `None` for the continuation of that
/// input, which runs on as far as the caller reads.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WantedInput {
    offset: u64,
    length: Option<u64>,
}

impl WantedInput {
    /// Creates the read of `length` bytes from `offset`, or of the continuation of the input from `offset` where `length` is `None`
    #[must_use]
    pub const fn new(offset: u64, length: Option<u64>) -> Self {
        Self { offset, length }
    }

    /// Returns the file offset the read starts at
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Returns how many bytes the read wants, or `None` where it runs on as far as the caller reads
    #[must_use]
    pub const fn length(self) -> Option<u64> {
        self.length
    }
}

/// Where input handed over at a file offset goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputRoute {
    /// To the framing, as the continuation of the input taken in order
    InOrder,
    /// To the samples, as the bytes they lack that the input taken in order has passed
    Lacking,
    /// Nowhere: the offset is neither
    Unwanted,
}

/// Where the input a demux FSM takes in order stands: the offset the reading started at, and what was handed over since
// Why not checked_add anywhere here: the offset the reading resumes at is one
// the caller vouches for, and a change of coordinates carries no failure kind,
// so an offset past any real file saturates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InputPosition {
    base: u64,
    handed: u64,
}

impl InputPosition {
    /// Creates a position at the first byte of the file
    pub(crate) const fn new() -> Self {
        Self { base: 0, handed: 0 }
    }

    /// Restarts the input at `offset`, nothing handed over from there yet
    pub(crate) const fn resume(&mut self, offset: u64) {
        self.base = offset;
        self.handed = 0;
    }

    /// Counts `length` more bytes handed over in order
    pub(crate) const fn advance(&mut self, length: usize) {
        self.handed = self.handed.saturating_add(length as u64);
    }

    /// Returns the file offset the next byte handed over in order lies at
    pub(crate) const fn offset(self) -> u64 {
        self.base.saturating_add(self.handed)
    }

    /// Returns the file offset of `framed`, an offset the framing counts from where the input started
    pub(crate) const fn file_offset(self, framed: u64) -> u64 {
        framed.saturating_add(self.base)
    }

    /// Returns the read wanted next: `lacking` where the input has passed its start, the continuation of the input otherwise
    pub(crate) fn wanted_input(self, lacking: Option<Range<u64>>) -> WantedInput {
        match lacking {
            Some(lacking) if lacking.start < self.offset() => WantedInput::new(
                lacking.start,
                Some(lacking.end.saturating_sub(lacking.start)),
            ),
            _ => WantedInput::new(self.offset(), None),
        }
    }

    /// Returns where input handed over at `offset` goes, given the bytes `lacking` the samples still lack
    pub(crate) fn route(self, offset: u64, lacking: Option<Range<u64>>) -> InputRoute {
        if offset == self.offset() {
            InputRoute::InOrder
        } else if lacking.is_some_and(|lacking| lacking.start == offset && offset < self.offset()) {
            InputRoute::Lacking
        } else {
            InputRoute::Unwanted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InputPosition, InputRoute, WantedInput};

    #[test]
    fn bytes_lacking_are_wanted_only_where_they_start_before_the_in_order_offset() {
        let mut position = InputPosition::new();
        position.advance(10);

        assert_eq!(
            [
                position.wanted_input(Some(9..20)),
                position.wanted_input(Some(10..20)),
                position.wanted_input(None)
            ],
            [
                WantedInput::new(9, Some(11)),
                WantedInput::new(10, None),
                WantedInput::new(10, None)
            ]
        );
    }

    #[test]
    fn input_goes_to_the_framing_in_order_to_the_samples_at_bytes_passed_and_nowhere_else() {
        let mut position = InputPosition::new();
        position.advance(10);

        assert_eq!(
            [
                position.route(10, Some(9..20)),
                position.route(9, Some(9..20)),
                position.route(12, Some(12..20)),
                position.route(8, Some(9..20)),
                position.route(9, None)
            ],
            [
                InputRoute::InOrder,
                InputRoute::Lacking,
                InputRoute::Unwanted,
                InputRoute::Unwanted,
                InputRoute::Unwanted
            ]
        );
    }
}
