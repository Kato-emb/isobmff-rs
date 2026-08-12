//! [`PayloadAccumulator`], the payload of one box gathered under a limit

use alloc::vec::Vec;
use core::error;
use core::fmt;

/// Gathers the payload of one box out of the chunks it arrives in, under a limit
///
/// A payload arrives cut into as many chunks as the stream carried it in, and
/// is whole only once the last of them is in. The limit is what keeps a
/// declared length from being taken at its word: no more than that many bytes
/// are ever gathered, however long the stream declares the box to be.
///
/// # Contract
///
/// * A chunk is gathered whole or not at all: a [`push`](Self::push) that would
///   carry the payload past the limit gathers none of it.
/// * Such a push leaves the accumulator failed for good — every later
///   [`push`](Self::push), and [`finish`](Self::finish), reports that same
///   error, and what was gathered before it is never handed over.
/// * An accumulator gathers one payload. [`finish`](Self::finish) consumes it,
///   and the box that follows is gathered by one built fresh.
///
/// # Examples
///
/// ```
/// use isobmff::{PayloadAccumulator, PayloadAccumulatorError};
///
/// // A payload arriving in two chunks, gathered under a limit of eight bytes
/// let mut accumulator = PayloadAccumulator::new(8);
/// accumulator.push(b"AA").unwrap();
/// accumulator.push(b"BB").unwrap();
///
/// // The chunks are handed over as one payload, in the order they arrived
/// assert_eq!(accumulator.finish(), Ok(b"AABB".to_vec()));
///
/// // A chunk reaching past the limit is rejected whole, the limit itself named
/// let mut accumulator = PayloadAccumulator::new(3);
///
/// assert_eq!(
///     accumulator.push(b"AAAA"),
///     Err(PayloadAccumulatorError::LimitExceeded {
///         limit: 3,
///         needed: 4
///     })
/// );
/// ```
#[derive(Debug)]
pub struct PayloadAccumulator {
    limit: usize,
    /// Payload as far as the chunks pushed so far carried it, or the error that
    /// failed the accumulator for good
    gathered: Result<Vec<u8>, PayloadAccumulatorError>,
}

impl PayloadAccumulator {
    /// Creates an accumulator gathering at most `limit` bytes
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            // Why not Vec::with_capacity(limit): the limit is a DoS bound, not
            // an expected length — reserving it would cost every accumulator
            // its worst case before a single payload byte arrives.
            gathered: Ok(Vec::new()),
        }
    }

    /// Gathers `chunk` onto what the payload holds so far
    ///
    /// # Errors
    ///
    /// * [`LimitExceeded`](PayloadAccumulatorError::LimitExceeded): the payload
    ///   would reach past the limit with `chunk` gathered.
    /// * The error a previous call already reported, once the accumulator has
    ///   failed.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), PayloadAccumulatorError> {
        let gathered = self.gathered.as_mut().map_err(|error| *error)?;
        let needed = gathered.len().saturating_add(chunk.len());

        if needed > self.limit {
            let error = PayloadAccumulatorError::LimitExceeded {
                limit: self.limit,
                needed,
            };

            self.gathered = Err(error);
            return Err(error);
        }

        gathered.extend_from_slice(chunk);
        Ok(())
    }

    /// Reports the payload as whole, and hands over the bytes gathered
    ///
    /// The accumulator is consumed.
    ///
    /// # Errors
    ///
    /// * The error a previous call already reported, once the accumulator has
    ///   failed.
    pub fn finish(self) -> Result<Vec<u8>, PayloadAccumulatorError> {
        self.gathered
    }
}

/// Reason a payload is not gathered
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PayloadAccumulatorError {
    /// Payload reaches past the limit the accumulator was given
    LimitExceeded {
        /// Bytes the accumulator gathers at most
        limit: usize,
        /// Bytes the payload reaches, the rejected chunk counted in
        needed: usize,
    },
}

impl fmt::Display for PayloadAccumulatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::LimitExceeded { limit, needed } => write!(
                formatter,
                "payload reaches {needed} bytes, past the limit of {limit}"
            ),
        }
    }
}

impl error::Error for PayloadAccumulatorError {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;
    use alloc::vec::Vec;

    use super::{PayloadAccumulator, PayloadAccumulatorError};

    #[test]
    fn chunks_are_gathered_in_the_order_they_arrive() {
        let mut accumulator = PayloadAccumulator::new(8);

        assert_eq!(accumulator.push(b"AA"), Ok(()));
        assert_eq!(accumulator.push(b"BB"), Ok(()));

        assert_eq!(accumulator.finish(), Ok(b"AABB".to_vec()));
    }

    #[test]
    fn an_accumulator_given_no_chunks_finishes_on_an_empty_payload() {
        assert_eq!(PayloadAccumulator::new(8).finish(), Ok(Vec::new()));
    }

    #[test]
    fn a_payload_reaching_exactly_the_limit_is_gathered() {
        let mut accumulator = PayloadAccumulator::new(4);

        assert_eq!(accumulator.push(b"AAAA"), Ok(()));

        assert_eq!(accumulator.finish(), Ok(b"AAAA".to_vec()));
    }

    #[test]
    fn a_chunk_reaching_past_the_limit_is_counted_in_whole() {
        let mut accumulator = PayloadAccumulator::new(3);

        assert_eq!(accumulator.push(b"AA"), Ok(()));

        assert_eq!(
            accumulator.push(b"BBBB"),
            Err(PayloadAccumulatorError::LimitExceeded {
                limit: 3,
                needed: 6
            })
        );
    }

    #[test]
    fn a_chunk_past_the_limit_fails_the_accumulator_for_good() {
        let mut accumulator = PayloadAccumulator::new(3);
        let failure = PayloadAccumulatorError::LimitExceeded {
            limit: 3,
            needed: 4,
        };

        assert_eq!(accumulator.push(b"AAAA"), Err(failure));
        assert_eq!(accumulator.push(b"A"), Err(failure));
        assert_eq!(accumulator.finish(), Err(failure));
    }

    #[test]
    fn display_of_an_exceeded_limit_names_both_lengths() {
        let error = PayloadAccumulatorError::LimitExceeded {
            limit: 3,
            needed: 4,
        };

        assert_eq!(
            error.to_string(),
            "payload reaches 4 bytes, past the limit of 3"
        );
    }
}
