//! [`InOrderPosition`], where the input a demux FSM takes in order stands in the file

use core::ops::Range;

/// Where the input a demux FSM takes in order stands: the offset the reading started at, and what was handed over since
// Why not checked_add anywhere here: the offset the reading resumes at is one
// the caller vouches for, and a change of coordinates carries no failure kind,
// so an offset past any real file saturates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InOrderPosition {
    base: u64,
    handed: u64,
}

impl InOrderPosition {
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

    /// Returns `wanted` where the input has passed its start, `None` where the input is still to bring it
    pub(crate) fn passed(self, wanted: Option<Range<u64>>) -> Option<Range<u64>> {
        wanted.filter(|wanted| wanted.start < self.offset())
    }
}

#[cfg(test)]
mod tests {
    use super::InOrderPosition;

    #[test]
    fn a_want_is_named_only_where_it_starts_before_the_input_offset() {
        let mut position = InOrderPosition::new();
        position.advance(10);

        assert_eq!(
            [
                position.passed(Some(9..20)),
                position.passed(Some(10..20)),
                position.passed(None)
            ],
            [Some(9..20), None, None]
        );
    }
}
