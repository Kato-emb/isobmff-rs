//! [`BoxReader`], the state machine that cuts the boxes of a file off its bytes

use core::ops::Range;

use isobmff_core::{BoxHeader, ErrorKind};

use super::{Error, Event, FramerConfig};
use crate::RequiredInput;
use crate::input_buffer::InputBuffer;

/// The smallest box header, `size` and `boxtype`, ISO/IEC 14496-12 §4.2.2
const COMPACT_HEADER_LENGTH: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// A box header starts the unconsumed bytes
    Header,
    /// The header of a box the reader gathers was consumed; its payload
    /// starts the unconsumed bytes
    // Why hold the offset, though the cursor less the payload in hand and the
    // header gives it: Skipping cannot derive its own, with nothing in hand,
    // and both states keep the box in one shape.
    Gathering { offset: u64, header: BoxHeader },
    /// The header of a box the reader does not gather, or cannot, was
    /// consumed; the bytes up to its end are dropped
    Skipping { offset: u64, header: BoxHeader },
    /// No box follows that can be found
    Ended,
}

/// Whether more input may follow the bytes a classifier is given
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Continues,
    Ends,
}

/// How much of the input handed over the state takes next
enum Take {
    /// Up to this many bytes, copied into the buffer
    Copy(u64),
    /// Up to this many bytes, walked past without a copy
    Skip(u64),
    /// No byte: no box follows that can be found
    Nothing,
}

/// What the reader saw at the front of its unconsumed bytes, in its state
enum Observation {
    Header(BoxHeader),
    MalformedHeader(isobmff_core::Error),
    /// The unconsumed bytes reach the end of the box the state reads
    BoxEndInHand,
    /// The unconsumed bytes stop before what the state waits for, and more
    /// input may follow
    NeedMoreData,
    /// The input ended before what the state waits for, with unconsumed bytes
    InputEndedShort,
    /// The input ended, and every byte of it is consumed
    InputExhausted,
}

/// Output of one transition, held until [`BoxReader::poll_event`] lends it
#[derive(Debug, Clone)]
enum Output {
    Box {
        offset: u64,
        header: BoxHeader,
    },
    Gathered {
        offset: u64,
        header: BoxHeader,
        payload: Range<usize>,
    },
    Error(Error),
}

/// What one transition leaves [`BoxReader::step`] to do
enum Step {
    /// Step again
    Continue,
    /// Hold the output for `poll_event`
    Output(Output),
    /// Wait for input
    NeedMoreData,
}

struct Transition {
    next_state: State,
    /// Bytes that leave the buffer front, already reported or cut
    consume: usize,
    step: Step,
}

/// Reader of the boxes laid end to end in a file, from bytes that arrive in pieces
///
/// The framing of boxes, ISO/IEC 14496-12 §4.2.2.
///
/// The reader names the file offset it wants next with
/// [`required_input`](Self::required_input), takes bytes from any offset up
/// to that one with [`handle_input`](Self::handle_input), and reports each
/// box at file level from [`poll_event`](Self::poll_event): a box whose type
/// [`FramerConfig::gathered_box_types`] holds as [`Event::Gathered`], with
/// its payload; any other as [`Event::Box`], whose payload it never
/// requests. Handing input over never fails: a box that cannot be read is
/// reported as [`Event::Error`]. The reader does no I/O, and does not read
/// the payload it gathers into a value. It does not report a box it skips
/// that runs past the end of the input: a caller sees that by comparing the
/// position [`required_input`](Self::required_input) names with the length of
/// its input.
///
/// One call of `handle_input` takes no more of the input than the next event
/// needs, so what the reader holds stays within a box header and one payload
/// however much is handed over. A caller that seeks hands over the bytes from
/// the position `required_input` names after each poll; a caller that reads a
/// stream in pieces hands each piece over again until that position lies past
/// the end of the piece.
///
/// ```
/// use isobmff::framing::{BoxReader, Event, FramerConfig};
///
/// let file: &[u8] = include_bytes!("../../tests/fixtures/movie_moov_last.mp4");
/// let mut reader = BoxReader::new(FramerConfig::new(1024 * 1024));
/// let mut boxes = Vec::new();
/// loop {
///     // Report what the bytes handed over hold
///     while let Some(event) = reader.poll_event() {
///         match event {
///             Event::Box { header, .. } => boxes.push(format!("{} skipped", header.box_type())),
///             Event::Gathered { box_type, payload, .. } => {
///                 boxes.push(format!("{box_type} of {} bytes", payload.len()));
///             }
///             Event::Error(error) => return Err(error.into()),
///             _ => {}
///         }
///     }
///     // Read the file where the reader asks, and end it where it ends
///     let Some(required) = reader.required_input() else { break };
///     match file.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => reader.handle_input(required.position, bytes),
///         _ => reader.finish(),
///     }
/// }
/// assert_eq!(boxes, ["ftyp of 24 bytes", "free skipped", "mdat skipped", "moov of 1792 bytes"]);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct BoxReader {
    config: FramerConfig,
    state: State,
    /// The bytes of the box header being read and of the payloads gathered;
    /// the payload of a box skipped is never copied in
    input: InputBuffer,
    /// File offset of the next byte the reader takes
    cursor: u64,
    /// The output of the last step, which `poll_event` lends
    pending: Option<Output>,
}

impl BoxReader {
    /// Creates a reader of a file from its first byte
    #[must_use]
    pub const fn new(config: FramerConfig) -> Self {
        Self {
            config,
            state: State::Header,
            input: InputBuffer::new(),
            cursor: 0,
            pending: None,
        }
    }

    /// Returns where the reader wants input next, or `None` once it wants none
    ///
    /// The position is the file offset of the first byte wanted, past the
    /// bytes already handed over and the payload of a box the reader skips;
    /// the length, when the reader knows it, is how many bytes it needs at
    /// least to report the next event. The reader wants none after
    /// [`finish`](Self::finish), and once no box follows that it can find.
    #[must_use]
    pub fn required_input(&self) -> Option<RequiredInput> {
        if self.input.is_ended() {
            return None;
        }
        let unconsumed = length_of(self.input.unconsumed());
        match self.state {
            State::Header => Some(RequiredInput {
                position: self.cursor,
                length: Some(
                    self.header_length_wanted()
                        .saturating_sub(unconsumed)
                        .max(1),
                ),
            }),
            State::Gathering { header, .. } => Some(RequiredInput {
                position: self.cursor,
                length: header
                    .payload_len()
                    .map(|payload_len| payload_len.saturating_sub(unconsumed).max(1)),
            }),
            State::Skipping { offset, header } => {
                let end = end_of_box(offset, header)?;
                Some(RequiredInput {
                    position: end.max(self.cursor),
                    length: Some(COMPACT_HEADER_LENGTH),
                })
            }
            State::Ended => None,
        }
    }

    /// Takes bytes of the file that start at the file offset `position`
    ///
    /// Bytes before the position [`required_input`](Self::required_input)
    /// names are ignored, so input may start earlier and run on past what is
    /// wanted. The reader takes bytes until it has the next event, and leaves
    /// the rest to be handed over again; it takes none while an event waits for
    /// [`poll_event`](Self::poll_event), and none once it wants no more. It
    /// copies only the box headers and the payloads it gathers, each within
    /// [`FramerConfig::payload_limit`], and walks past the payload of a box it
    /// skips.
    ///
    /// # Panics
    /// Panics if `position` is after the position `required_input` names, or
    /// if called after [`finish`](Self::finish): either is a bug of the caller.
    pub fn handle_input(&mut self, position: u64, bytes: &[u8]) {
        assert!(!self.input.is_ended(), "input handed over after finish");
        if self.pending.is_some() {
            return;
        }
        let Some(required) = self.required_input() else {
            return;
        };
        assert!(
            position <= required.position,
            "input starts at offset {position}, after the offset {} the reader requires",
            required.position
        );
        let ignored =
            usize::try_from(required.position.saturating_sub(position)).unwrap_or(usize::MAX);
        let Some(mut bytes) = bytes.get(ignored..) else {
            return;
        };
        self.cursor = required.position;
        self.input.compact();
        self.step();
        while self.pending.is_none() && !bytes.is_empty() {
            let (taken, copied) = match self.take() {
                Take::Copy(length) => (clamp(length, bytes.len()), true),
                Take::Skip(length) => (clamp(length, bytes.len()), false),
                Take::Nothing => (0, false),
            };
            // Why stop on taking nothing: every state that takes bytes wants
            // at least one more after a step, so a state taking none would
            // loop on the same bytes.
            if taken == 0 {
                break;
            }
            let (head, rest) = bytes.split_at(taken);
            if copied {
                self.input.extend(head);
            }
            bytes = rest;
            self.cursor = self
                .cursor
                .saturating_add(u64::try_from(taken).unwrap_or(u64::MAX));
            self.step();
        }
    }

    /// Returns what the bytes handed over hold, one event at a time, or `None`
    /// once the reader needs more input
    ///
    /// After [`finish`](Self::finish), each call reports the next event the
    /// end of the input leaves, until there is none.
    pub fn poll_event(&mut self) -> Option<Event<'_>> {
        if self.pending.is_none() {
            self.step();
        }
        Some(match self.pending.take()? {
            Output::Box { offset, header } => Event::Box { offset, header },
            Output::Gathered {
                offset,
                header,
                payload,
            } => Event::Gathered {
                offset,
                box_type: header.box_type(),
                payload_offset: offset.saturating_add(header.encoded_len()),
                payload: self.input.get(payload),
            },
            Output::Error(error) => Event::Error(error),
        })
    }

    /// Ends the input at the bytes handed over so far
    ///
    /// The events that follow report a box the reader gathers that the input
    /// ends inside as [`Error::TruncatedBox`], and one whose `size` is 0, which
    /// runs to the end of the file, as the bytes handed over hold it. A box
    /// the reader skips is not reported truncated: it never requests the
    /// payload of one, so it cannot tell whether the input holds it.
    pub fn finish(&mut self) {
        self.input.finish();
        self.step();
    }

    /// How much of the input the state takes next
    fn take(&self) -> Take {
        let unconsumed = length_of(self.input.unconsumed());
        match self.state {
            State::Header => Take::Copy(self.header_length_wanted().saturating_sub(unconsumed)),
            State::Gathering { header, .. } => Take::Copy(
                header
                    .payload_len()
                    // Why one byte past the limit for a box of size 0: it
                    // declares no length, so only a payload longer than the
                    // limit shows that it is.
                    .unwrap_or(self.config.payload_limit.saturating_add(1))
                    .saturating_sub(unconsumed),
            ),
            State::Skipping { offset, header } => Take::Skip(
                end_of_box(offset, header).map_or(u64::MAX, |end| end.saturating_sub(self.cursor)),
            ),
            State::Ended => Take::Nothing,
        }
    }

    /// Steps the state machine until it has an output for `poll_event`, or waits for input
    fn step(&mut self) {
        loop {
            let transition = self.transition();
            self.state = transition.next_state;
            self.input.consume(transition.consume);
            match transition.step {
                Step::Continue => {}
                Step::Output(output) => {
                    self.pending = Some(output);
                    return;
                }
                Step::NeedMoreData => return,
            }
        }
    }

    fn transition(&self) -> Transition {
        let unconsumed = self.input.unconsumed();
        let start = self.input.consumed();
        let offset = self.cursor.saturating_sub(length_of(unconsumed));
        let input = if self.input.is_ended() {
            Input::Ends
        } else {
            Input::Continues
        };
        let limit = self.config.payload_limit;
        let over_limit = |offset, header| {
            Step::Output(Output::Error(Error::PayloadOverLimit {
                offset,
                header,
                limit,
            }))
        };
        // Why compare what is in hand against the limit only for a box of
        // size 0: a box that declares its length is refused at its header
        // when that length is over the limit, and what is in hand past its
        // payload belongs to the boxes after it.
        let gathered_over_limit = length_of(unconsumed) > limit;
        match (
            self.state,
            observe(self.state, self.cursor, unconsumed, input),
        ) {
            (State::Ended, _) => Transition {
                next_state: State::Ended,
                consume: 0,
                step: Step::NeedMoreData,
            },
            (State::Header, Observation::Header(header))
                if self.gathers(header)
                    && header.payload_len().is_some_and(|length| length > limit) =>
            {
                Transition {
                    next_state: State::Skipping { offset, header },
                    consume: header_length(header),
                    step: over_limit(offset, header),
                }
            }
            (State::Header, Observation::Header(header)) if self.gathers(header) => Transition {
                next_state: State::Gathering { offset, header },
                consume: header_length(header),
                step: Step::Continue,
            },
            (State::Header, Observation::Header(header)) => Transition {
                next_state: State::Skipping { offset, header },
                consume: header_length(header),
                step: Step::Output(Output::Box { offset, header }),
            },
            (State::Header, Observation::MalformedHeader(source)) => Transition {
                next_state: State::Ended,
                consume: unconsumed.len(),
                step: Step::Output(Output::Error(Error::MalformedHeader { offset, source })),
            },
            (State::Header, Observation::InputEndedShort) => Transition {
                next_state: State::Ended,
                consume: unconsumed.len(),
                step: Step::Output(Output::Error(Error::TruncatedHeader { offset })),
            },
            (
                State::Gathering {
                    offset: box_offset,
                    header,
                },
                Observation::BoxEndInHand | Observation::NeedMoreData,
            ) if header.payload_len().is_none() && gathered_over_limit => Transition {
                next_state: State::Skipping {
                    offset: box_offset,
                    header,
                },
                consume: unconsumed.len(),
                step: over_limit(box_offset, header),
            },
            (
                State::Gathering {
                    offset: box_offset,
                    header,
                },
                Observation::BoxEndInHand,
            ) => {
                let (payload_len, next_state) = match header
                    .payload_len()
                    .and_then(|payload_len| usize::try_from(payload_len).ok())
                {
                    Some(payload_len) => (payload_len, State::Header),
                    None => (unconsumed.len(), State::Ended),
                };
                Transition {
                    next_state,
                    consume: payload_len,
                    step: Step::Output(Output::Gathered {
                        offset: box_offset,
                        header,
                        payload: start..start.saturating_add(payload_len),
                    }),
                }
            }
            (
                State::Gathering {
                    offset: box_offset,
                    header,
                },
                Observation::InputEndedShort | Observation::InputExhausted,
            ) => Transition {
                next_state: State::Ended,
                consume: unconsumed.len(),
                step: Step::Output(Output::Error(Error::TruncatedBox {
                    offset: box_offset,
                    header,
                })),
            },
            (State::Header | State::Gathering { .. }, Observation::NeedMoreData) => Transition {
                next_state: self.state,
                consume: 0,
                step: Step::NeedMoreData,
            },
            (State::Skipping { .. }, Observation::BoxEndInHand) => Transition {
                next_state: State::Header,
                consume: 0,
                step: Step::Continue,
            },
            (State::Skipping { .. }, Observation::NeedMoreData) => Transition {
                next_state: self.state,
                consume: 0,
                step: Step::NeedMoreData,
            },
            (
                State::Skipping { .. },
                Observation::InputEndedShort
                | Observation::InputExhausted
                | Observation::Header(_)
                | Observation::MalformedHeader(_),
            )
            | (State::Gathering { .. }, Observation::Header(_) | Observation::MalformedHeader(_))
            | (State::Header, Observation::BoxEndInHand | Observation::InputExhausted) => {
                Transition {
                    next_state: State::Ended,
                    consume: unconsumed.len(),
                    step: Step::NeedMoreData,
                }
            }
        }
    }

    fn gathers(&self, header: BoxHeader) -> bool {
        self.config.gathered_box_types.contains(&header.box_type())
    }

    /// Length of the header that starts the unconsumed bytes, as far as they
    /// show it
    fn header_length_wanted(&self) -> u64 {
        match BoxHeader::decode(self.input.unconsumed()).map_err(isobmff_core::Error::kind) {
            Err(ErrorKind::TruncatedHeader { needed_bytes, .. }) => needed_bytes,
            Ok(_) | Err(_) => COMPACT_HEADER_LENGTH,
        }
    }
}

/// Classifies the unconsumed bytes in `state`, `cursor` being the file offset past them
fn observe(state: State, cursor: u64, bytes: &[u8], input: Input) -> Observation {
    let in_hand = match state {
        State::Header => match BoxHeader::decode(bytes) {
            Ok((header, _)) => return Observation::Header(header),
            Err(error) if !matches!(error.kind(), ErrorKind::TruncatedHeader { .. }) => {
                return Observation::MalformedHeader(error);
            }
            Err(_) => false,
        },
        State::Gathering { header, .. } => match header.payload_len() {
            Some(payload_len) => length_of(bytes) >= payload_len,
            None => input == Input::Ends,
        },
        State::Skipping { offset, header } => {
            end_of_box(offset, header).is_some_and(|end| cursor >= end)
        }
        State::Ended => false,
    };
    match (in_hand, input) {
        (true, _) => Observation::BoxEndInHand,
        (false, Input::Continues) => Observation::NeedMoreData,
        (false, Input::Ends) if bytes.is_empty() => Observation::InputExhausted,
        (false, Input::Ends) => Observation::InputEndedShort,
    }
}

/// File offset just past the box at `offset`, or `None` for a box that runs to the end of the file
fn end_of_box(offset: u64, header: BoxHeader) -> Option<u64> {
    // Why saturating: a declared size can reach past u64 from a late offset,
    // and such a box ends past every byte a file can hold.
    header
        .size()
        .total_bytes()
        .map(|total| offset.saturating_add(total))
}

fn header_length(header: BoxHeader) -> usize {
    usize::try_from(header.encoded_len()).unwrap_or(usize::MAX)
}

fn length_of(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// `length`, or `available` where that is less
fn clamp(length: u64, available: usize) -> usize {
    usize::try_from(length).map_or(available, |length| length.min(available))
}
