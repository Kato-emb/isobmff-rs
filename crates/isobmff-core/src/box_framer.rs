//! [`BoxFramer`] and [`BoxEvent`], the box framing of ISO/IEC 14496-12 §4.2 taken one chunk at a time

use core::error;
use core::fmt;

use crate::box_header::{BoxHeader, BoxHeaderError};

/// Step a stream of boxes takes, as [`BoxFramer::next_event`] reports it
///
/// A box appears as [`Start`](Self::Start), then as many
/// [`Payload`](Self::Payload) events as the chunks its payload arrived in, then
/// [`End`](Self::End). A container is reported as one box like any other: its
/// payload is not descended into, and splitting it into the boxes it holds is
/// [`boxes`](crate::boxes)'s part, once those bytes are whole.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BoxEvent<'chunk> {
    /// Header of a box, whole, however many chunks it was spread over
    Start(BoxHeader),
    /// Part of the payload of the box that started, as it lies in the chunk
    Payload(&'chunk [u8]),
    /// End of the box that started, its declared total reached
    End,
}

/// Frames a stream of boxes arriving in chunks of any length
///
/// The framer is fed a chunk at a time and reports one [`BoxEvent`] per call,
/// taking the bytes of that event off the chunk. It reads no box type and holds
/// no policy: which boxes to gather, which to pass over, and what a payload
/// means are all the caller's.
///
/// # Contract
///
/// * One call reports at most one event, and takes exactly the bytes that event
///   stands for — for [`Start`](BoxEvent::Start), the part of the header the
///   chunks before this one had not already carried; for
///   [`Payload`](BoxEvent::Payload), the bytes it holds; for
///   [`End`](BoxEvent::End), none. A caller keeping its own count of where it is
///   in the stream can therefore keep it from what each call took off the chunk.
/// * `Ok(None)` says the chunk is used up and the next one is needed; the chunk
///   is empty when it is returned.
/// * A [`Payload`](BoxEvent::Payload) event is never empty. A box with no
///   payload is a [`Start`](BoxEvent::Start) followed by an
///   [`End`](BoxEvent::End).
/// * An `Err` leaves the framer failed for good: every later call reports that
///   same error. The chunk is left where the failing call had read to.
/// * [`finish`](Self::finish) is not owed. Dropping a framer part-way through a
///   box is how a caller about to read elsewhere in the stream starts over,
///   with a framer built fresh at the position it moves to.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxEvent, BoxFramer, BoxHeader, BoxSize, BoxType, CompactSize};
///
/// // One twelve-byte box, arriving in chunks that cut both its header and its payload
/// let chunks: [&[u8]; 3] = [b"\0\0\0\x0cfr", b"eeAA", b"AA"];
/// let mut framer = BoxFramer::new();
/// let mut events = Vec::new();
///
/// // A chunk is fed until it is used up, one event at a time
/// for chunk in chunks {
///     let mut input = chunk;
///     while let Some(event) = framer.next_event(&mut input).unwrap() {
///         events.push(event);
///     }
/// }
///
/// // The header spanning two chunks is gathered before the box is reported
/// let header = BoxHeader::new(
///     BoxType::compact(*b"free"),
///     BoxSize::Compact(CompactSize::new(12).unwrap()),
/// )
/// .unwrap();
/// assert_eq!(
///     events,
///     [
///         BoxEvent::Start(header),
///         BoxEvent::Payload(b"AA"),
///         BoxEvent::Payload(b"AA"),
///         BoxEvent::End,
///     ]
/// );
///
/// // The stream ended on a box boundary, so nothing was left open
/// assert_eq!(framer.finish(), Ok(None));
/// ```
// Why not derive `Copy`: `finish` takes the framer by value so that a stream is
// declared over once and no call can follow, which a copy left behind would undo.
#[derive(Debug)]
pub struct BoxFramer {
    state: State,
}

impl BoxFramer {
    /// Creates a framer waiting at the start of a box
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::Header(PartialHeader::EMPTY),
        }
    }

    /// Reports the next step of the stream, taking its bytes off `input`
    ///
    /// Returns `Ok(None)` once `input` is used up, leaving it empty; the framer
    /// keeps what it read of a header that is not yet whole. The full contract,
    /// including what an `Err` leaves behind, is on [`BoxFramer`].
    ///
    /// # Errors
    ///
    /// * [`SizeBelowHeader`](BoxFramerError::SizeBelowHeader): a header declares
    ///   a total smaller than the header itself.
    /// * The error a previous call already reported, once the framer has failed.
    pub fn next_event<'chunk>(
        &mut self,
        input: &mut &'chunk [u8],
    ) -> Result<Option<BoxEvent<'chunk>>, BoxFramerError> {
        match self.state {
            State::Failed(error) => Err(error),
            State::Header(mut partial) => match partial.take_from(input) {
                Ok(Some(header)) => {
                    self.state = State::Payload {
                        header,
                        remaining: header.payload_len(),
                    };
                    Ok(Some(BoxEvent::Start(header)))
                }
                Ok(None) => {
                    self.state = State::Header(partial);
                    Ok(None)
                }
                Err(error) => {
                    self.state = State::Failed(error);
                    Err(error)
                }
            },
            State::Payload { header, remaining } => {
                if remaining == Some(0) {
                    self.state = State::Header(PartialHeader::EMPTY);
                    return Ok(Some(BoxEvent::End));
                }
                if input.is_empty() {
                    return Ok(None);
                }

                let wanted = remaining
                    .and_then(|remaining| usize::try_from(remaining).ok())
                    .unwrap_or(usize::MAX)
                    .min(input.len());
                let (chunk, rest) = input.split_at(wanted);

                *input = rest;
                self.state = State::Payload {
                    header,
                    remaining: remaining.map(|remaining| {
                        remaining.saturating_sub(u64::try_from(chunk.len()).unwrap_or(u64::MAX))
                    }),
                };
                Ok(Some(BoxEvent::Payload(chunk)))
            }
        }
    }

    /// Reports that the stream is over, and closes the box left open
    ///
    /// A box is left open either because the stream reached its declared total
    /// without the caller asking for the [`End`](BoxEvent::End) that follows, or
    /// because it declares no total at all —
    /// [`ToEndOfFile`](crate::BoxSize::ToEndOfFile), which only the end of the
    /// stream ends. Both are closed with an [`End`](BoxEvent::End) here. A
    /// stream that stopped on a box boundary leaves nothing open and reports
    /// `Ok(None)`.
    ///
    /// The framer is consumed.
    ///
    /// # Errors
    ///
    /// * [`UnfinishedHeader`](BoxFramerError::UnfinishedHeader): the stream
    ///   stopped inside a header.
    /// * [`UnfinishedBox`](BoxFramerError::UnfinishedBox): the stream stopped
    ///   before the declared total of a box was reached.
    /// * The error a previous call already reported, once the framer has failed.
    pub fn finish(self) -> Result<Option<BoxEvent<'static>>, BoxFramerError> {
        match self.state {
            State::Failed(error) => Err(error),
            State::Header(partial) if partial.filled == 0 => Ok(None),
            State::Header(partial) => Err(BoxFramerError::UnfinishedHeader {
                needed: partial.needed,
                available: partial.filled,
            }),
            State::Payload { header, remaining } => {
                let unfinished = remaining
                    .filter(|remaining| *remaining != 0)
                    .zip(header.size().total_bytes());

                match unfinished {
                    Some((remaining, total)) => Err(BoxFramerError::UnfinishedBox {
                        needed: total,
                        available: total.saturating_sub(remaining),
                    }),
                    None => Ok(Some(BoxEvent::End)),
                }
            }
        }
    }
}

impl Default for BoxFramer {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the framer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Gathering the header of the box that starts next
    Header(PartialHeader),
    /// Passing on the payload of the box that started, with `remaining` bytes
    /// of it still to come — `None` while the box declares no total
    Payload {
        header: BoxHeader,
        remaining: Option<u64>,
    },
    /// Failed, and reporting the same error from here on
    Failed(BoxFramerError),
}

/// Header bytes gathered so far, for a header spread over several chunks
///
/// `needed` is the length the header reaches: the shortest header until the
/// bytes gathered name the true one, and never past the longest a box can carry.
#[derive(Clone, Copy, Debug)]
struct PartialHeader {
    bytes: [u8; BoxHeader::MAX_ENCODED_LEN],
    filled: usize,
    needed: usize,
}

impl PartialHeader {
    const EMPTY: Self = Self {
        bytes: [0; BoxHeader::MAX_ENCODED_LEN],
        filled: 0,
        needed: BoxHeader::MIN_ENCODED_LEN,
    };

    /// Takes header bytes off `input`, and decodes the header once it is whole
    ///
    /// Returns `Ok(None)` when `input` runs out first, having taken what it
    /// offered.
    ///
    /// # Errors
    ///
    /// * [`SizeBelowHeader`](BoxFramerError::SizeBelowHeader): the header
    ///   declares a total smaller than itself.
    fn take_from(&mut self, input: &mut &[u8]) -> Result<Option<BoxHeader>, BoxFramerError> {
        self.gather(input);
        if self.filled < BoxHeader::MIN_ENCODED_LEN {
            return Ok(None);
        }
        // Why not unreachable: the buffer is the longest header a box can carry,
        // so a chunk of the shortest always splits off, and the fallback is a
        // degenerate value in place of a panic the lints forbid.
        let Some(prefix) = self.bytes.first_chunk() else {
            return Ok(None);
        };

        self.needed = BoxHeader::encoded_len_from_prefix(prefix);
        self.gather(input);
        if self.filled < self.needed {
            return Ok(None);
        }
        // Why not unreachable: `gather` takes no more than `needed`, which is a
        // header length and so never past the buffer, for the same reason.
        let Some(gathered) = self.bytes.get(..self.filled) else {
            return Ok(None);
        };

        match BoxHeader::decode(gathered) {
            Ok((header, _nothing_beyond)) => Ok(Some(header)),
            Err(BoxHeaderError::SizeBelowHeader {
                declared,
                header_length,
            }) => Err(BoxFramerError::SizeBelowHeader {
                declared,
                header_length,
            }),
            // Why not leave this arm out: the bytes handed over are the length
            // `encoded_len_from_prefix` named, so the header cannot come up
            // short. Matching it rather than waving it past with a catch-all
            // stops the build here if a variant is added later.
            Err(BoxHeaderError::TruncatedHeader { needed, available }) => {
                Err(BoxFramerError::UnfinishedHeader { needed, available })
            }
        }
    }

    /// Takes off `input` as many bytes as the length being reached for is short
    fn gather(&mut self, input: &mut &[u8]) {
        let wanted = self.needed.saturating_sub(self.filled).min(input.len());
        let filled = self.filled.saturating_add(wanted);

        let Some(slot) = self.bytes.get_mut(self.filled..filled) else {
            return;
        };
        let (chunk, rest) = input.split_at(wanted);

        slot.copy_from_slice(chunk);
        self.filled = filled;
        *input = rest;
    }
}

/// Reason a stream of chunks does not frame into boxes
///
/// A header the chunks have not reached the end of yet is no error at all — it
/// is `Ok(None)`, an ask for the next chunk — and becomes
/// [`UnfinishedHeader`](Self::UnfinishedHeader) only once [`BoxFramer::finish`]
/// says that no chunk is coming.
// Why not carry `BoxHeaderError` the way `RawBoxError` does: only
// `SizeBelowHeader` of it can reach a framer, since a header short of its
// length is answered with more chunks rather than reported, so nesting would
// seat a variant no caller can ever meet inside a public type.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BoxFramerError {
    /// Declared total is smaller than the header it prefixes
    SizeBelowHeader {
        /// Total the `size` or `largesize` field declares
        declared: u64,
        /// Bytes the header occupies
        header_length: u64,
    },
    /// Stream ended inside a box header
    UnfinishedHeader {
        /// Bytes the header occupies, as far as the bytes read so far tell
        needed: usize,
        /// Bytes of the header the stream carried
        available: usize,
    },
    /// Stream ended before the declared total of a box was reached
    UnfinishedBox {
        /// Bytes the box occupies, as the `size` or `largesize` field declares
        needed: u64,
        /// Bytes of the box the stream carried, header included
        available: u64,
    },
}

impl fmt::Display for BoxFramerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::SizeBelowHeader {
                declared,
                header_length,
            } => write!(
                formatter,
                "box declares a total of {declared} bytes, below its {header_length}-byte header"
            ),
            Self::UnfinishedHeader { needed, available } => write!(
                formatter,
                "stream ended {available} bytes into a box header of {needed}"
            ),
            Self::UnfinishedBox { needed, available } => write!(
                formatter,
                "stream ended {available} bytes into a box of {needed}"
            ),
        }
    }
}

impl error::Error for BoxFramerError {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{BoxEvent, BoxFramer, BoxFramerError};
    use crate::box_header::BoxHeader;
    use crate::box_size::{BoxSize, CompactSize, ExtendedSize};
    use crate::box_type::BoxType;
    use crate::uuid::Uuid;

    const USER_TYPE: Uuid = Uuid::new([
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ]);

    /// Header of a box declaring `total` in the compact `size` field
    fn compact_header(box_type: [u8; 4], total: u32) -> BoxHeader {
        BoxHeader::new(
            BoxType::compact(box_type),
            BoxSize::Compact(CompactSize::new(total).unwrap()),
        )
        .unwrap()
    }

    /// Every event a framer reports for `input`, fed in chunks of `chunk_length`
    fn events_of(input: &[u8], chunk_length: usize) -> Vec<BoxEvent<'_>> {
        let mut framer = BoxFramer::new();
        let mut events = Vec::new();

        for chunk in input.chunks(chunk_length) {
            let mut remaining = chunk;
            while let Some(event) = framer.next_event(&mut remaining).unwrap() {
                events.push(event);
            }
            assert!(remaining.is_empty(), "a used-up chunk left bytes behind");
        }

        events
    }

    #[test]
    fn a_box_arriving_whole_is_reported_as_a_start_a_payload_and_an_end() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA", 12),
            vec![
                BoxEvent::Start(compact_header(*b"free", 12)),
                BoxEvent::Payload(b"AAAA"),
                BoxEvent::End,
            ]
        );
    }

    #[test]
    fn a_box_with_no_payload_is_reported_as_a_start_and_an_end() {
        assert_eq!(
            events_of(b"\0\0\0\x08skip", 8),
            vec![BoxEvent::Start(compact_header(*b"skip", 8)), BoxEvent::End]
        );
    }

    #[test]
    fn a_box_cut_at_any_byte_frames_into_the_same_boundaries() {
        let input = b"\0\0\0\x0cfreeAAAA";

        for chunk_length in 1..=input.len() {
            let boundaries = events_of(input, chunk_length)
                .into_iter()
                .filter(|event| !matches!(*event, BoxEvent::Payload(_)))
                .collect::<Vec<_>>();

            assert_eq!(
                boundaries,
                vec![BoxEvent::Start(compact_header(*b"free", 12)), BoxEvent::End],
                "chunks of {chunk_length} bytes"
            );
        }
    }

    #[test]
    fn a_header_carrying_a_large_size_and_a_user_type_is_gathered_across_chunks() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x21, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98,
            0x76, 0x54, 0x32, 0x10, b'!',
        ];
        let header = BoxHeader::new(
            BoxType::Extended(USER_TYPE),
            BoxSize::Extended(ExtendedSize::new(33).unwrap()),
        )
        .unwrap();

        assert_eq!(
            events_of(&input, 3),
            vec![
                BoxEvent::Start(header),
                BoxEvent::Payload(b"!"),
                BoxEvent::End,
            ]
        );
    }

    #[test]
    fn boxes_laid_end_to_end_are_reported_in_the_order_they_arrive() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip", 5),
            vec![
                BoxEvent::Start(compact_header(*b"free", 12)),
                BoxEvent::Payload(b"AA"),
                BoxEvent::Payload(b"AA"),
                BoxEvent::End,
                BoxEvent::Start(compact_header(*b"skip", 8)),
                BoxEvent::End,
            ]
        );
    }

    #[test]
    fn an_empty_chunk_asks_for_the_next_one_and_leaves_the_framer_where_it_was() {
        let mut framer = BoxFramer::new();
        let mut empty: &[u8] = b"";
        let mut input: &[u8] = b"\0\0\0\x08free";

        assert_eq!(framer.next_event(&mut empty), Ok(None));

        assert_eq!(
            framer.next_event(&mut input),
            Ok(Some(BoxEvent::Start(compact_header(*b"free", 8))))
        );
    }

    #[test]
    fn a_box_running_to_the_end_of_the_stream_is_closed_by_finishing() {
        let mut framer = BoxFramer::new();
        let mut input: &[u8] = b"\0\0\0\0mdatPAYLOAD";
        let mut events = Vec::new();

        while let Some(event) = framer.next_event(&mut input).unwrap() {
            events.push(event);
        }

        assert_eq!(
            events,
            vec![
                BoxEvent::Start(
                    BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap()
                ),
                BoxEvent::Payload(b"PAYLOAD"),
            ]
        );
        assert_eq!(framer.finish(), Ok(Some(BoxEvent::End)));
    }

    #[test]
    fn a_box_whose_total_the_stream_reached_is_closed_by_finishing() {
        let mut framer = BoxFramer::new();
        let mut input: &[u8] = b"\0\0\0\x0cfreeAAAA";

        assert_eq!(
            framer.next_event(&mut input),
            Ok(Some(BoxEvent::Start(compact_header(*b"free", 12))))
        );
        assert_eq!(
            framer.next_event(&mut input),
            Ok(Some(BoxEvent::Payload(b"AAAA")))
        );

        assert_eq!(framer.finish(), Ok(Some(BoxEvent::End)));
    }

    #[test]
    fn an_empty_chunk_arriving_inside_a_payload_asks_for_the_next_one() {
        let mut framer = BoxFramer::new();
        let mut started: &[u8] = b"\0\0\0\x0cfreeAA";
        let mut empty: &[u8] = b"";
        let mut rest: &[u8] = b"AA";

        while framer.next_event(&mut started).unwrap().is_some() {}

        assert_eq!(framer.next_event(&mut empty), Ok(None));
        assert_eq!(
            framer.next_event(&mut rest),
            Ok(Some(BoxEvent::Payload(b"AA")))
        );
    }

    #[test]
    fn a_stream_stopping_on_a_box_boundary_leaves_nothing_open() {
        let mut framer = BoxFramer::new();
        let mut input: &[u8] = b"\0\0\0\x08free";

        while framer.next_event(&mut input).unwrap().is_some() {}

        assert_eq!(framer.finish(), Ok(None));
    }

    #[test]
    fn a_stream_stopping_inside_a_header_is_rejected_as_unfinished() {
        let mut framer = BoxFramer::new();
        let mut input: &[u8] = &[0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00];

        while framer.next_event(&mut input).unwrap().is_some() {}

        assert_eq!(
            framer.finish(),
            Err(BoxFramerError::UnfinishedHeader {
                needed: 16,
                available: 9
            })
        );
    }

    #[test]
    fn a_stream_stopping_inside_a_box_is_rejected_as_unfinished() {
        let mut framer = BoxFramer::new();
        let mut input: &[u8] = b"\0\0\0\x10freeAAAA";

        while framer.next_event(&mut input).unwrap().is_some() {}

        assert_eq!(
            framer.finish(),
            Err(BoxFramerError::UnfinishedBox {
                needed: 16,
                available: 12
            })
        );
    }

    #[test]
    fn a_total_below_the_header_it_prefixes_fails_the_framer_for_good() {
        let mut framer = BoxFramer::new();
        let mut input: &[u8] = b"\0\0\0\x04free\0\0\0\x08skip";
        let failure = Err(BoxFramerError::SizeBelowHeader {
            declared: 4,
            header_length: 8,
        });

        assert_eq!(framer.next_event(&mut input), failure);
        assert_eq!(input, b"\0\0\0\x08skip");
        assert_eq!(framer.next_event(&mut input), failure);
        assert_eq!(framer.finish(), failure);
    }

    #[test]
    fn display_of_an_unfinished_header_names_both_lengths() {
        let error = BoxFramerError::UnfinishedHeader {
            needed: 16,
            available: 9,
        };

        assert_eq!(
            error.to_string(),
            "stream ended 9 bytes into a box header of 16"
        );
    }

    #[test]
    fn display_of_an_unfinished_box_names_both_lengths() {
        let error = BoxFramerError::UnfinishedBox {
            needed: 16,
            available: 12,
        };

        assert_eq!(error.to_string(), "stream ended 12 bytes into a box of 16");
    }

    #[test]
    fn display_of_a_size_below_its_header_names_both_totals() {
        let error = BoxFramerError::SizeBelowHeader {
            declared: 4,
            header_length: 8,
        };

        assert_eq!(
            error.to_string(),
            "box declares a total of 4 bytes, below its 8-byte header"
        );
    }
}
