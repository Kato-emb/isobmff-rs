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

/// Output of one transition
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

/// Whether [`BoxReader::step`] has an output, or waits for input
enum Progress {
    Output(Output),
    NeedMoreData,
}

struct Transition {
    next_state: State,
    /// Bytes that leave the buffer front, already reported or cut
    consume: usize,
    /// `None` keeps [`BoxReader::step`] stepping
    progress: Option<Progress>,
}

/// Reader of the boxes laid end to end in a file, from bytes that arrive in pieces, ISO/IEC 14496-12 §4.2.2
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
///             Event::Gathered { header, payload, .. } => {
///                 boxes.push(format!("{} of {} bytes", header.box_type(), payload.len()));
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
    input: InputBuffer,
    /// File offset of the first byte `input` holds
    position: u64,
}

impl BoxReader {
    /// Creates a reader of a file from its first byte
    #[must_use]
    pub const fn new(config: FramerConfig) -> Self {
        Self {
            config,
            state: State::Header,
            input: InputBuffer::new(),
            position: 0,
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
        let end_of_input = self.offset_of(self.input.len());
        let unconsumed = u64::try_from(self.input.unconsumed().len()).unwrap_or(u64::MAX);
        match self.state {
            State::Header => Some(RequiredInput {
                position: end_of_input,
                length: Some(
                    self.header_length_wanted()
                        .saturating_sub(unconsumed)
                        .max(1),
                ),
            }),
            State::Gathering { header, .. } => Some(RequiredInput {
                position: end_of_input,
                length: header
                    .payload_len()
                    .map(|payload_len| payload_len.saturating_sub(unconsumed).max(1)),
            }),
            State::Skipping { offset, header } => {
                let end = end_of_box(offset, header)?;
                Some(RequiredInput {
                    position: end.max(end_of_input),
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
    /// wanted. Input once the reader wants none is ignored.
    ///
    /// # Panics
    /// Panics if `position` is after the position `required_input` names, or
    /// if called after [`finish`](Self::finish): either is a bug of the caller.
    pub fn handle_input(&mut self, position: u64, bytes: &[u8]) {
        assert!(!self.input.is_ended(), "input handed over after finish");
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
        let Some(bytes) = bytes.get(ignored..) else {
            return;
        };
        if required.position > self.offset_of(self.input.len()) {
            // Why drop the buffer rather than append after it: the bytes
            // between its end and the required position are payload of a
            // box being skipped, which were never handed over.
            self.input.consume(self.input.len());
            self.input.compact();
            self.position = required.position;
        }
        self.input.extend(bytes);
    }

    /// Returns what the bytes handed over hold, one event at a time, or `None`
    /// once the reader needs more input
    pub fn poll_event(&mut self) -> Option<Event<'_>> {
        let dropped = self.input.compact();
        self.position = self.offset_of(dropped);
        let output = match self.step() {
            Progress::Output(output) => output,
            Progress::NeedMoreData => return None,
        };
        Some(match output {
            Output::Box { offset, header } => Event::Box { offset, header },
            Output::Gathered {
                offset,
                header,
                payload,
            } => Event::Gathered {
                offset,
                header,
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
    }

    /// Steps the state machine until it has an output, or waits for input
    fn step(&mut self) -> Progress {
        loop {
            let transition = self.transition();
            self.state = transition.next_state;
            self.input.consume(transition.consume);
            if let Some(progress) = transition.progress {
                return progress;
            }
        }
    }

    fn transition(&self) -> Transition {
        let unconsumed = self.input.unconsumed();
        let offset = self.offset_of(self.input.consumed());
        let input = if self.input.is_ended() {
            Input::Ends
        } else {
            Input::Continues
        };
        match (self.state, observe(self.state, offset, unconsumed, input)) {
            (State::Header, Observation::Header(header)) if self.gathers(header) => {
                match header.payload_len() {
                    Some(payload_len) if payload_len > self.config.payload_limit => Transition {
                        next_state: State::Skipping { offset, header },
                        consume: header_length(header),
                        progress: Some(Progress::Output(Output::Error(Error::PayloadOverLimit {
                            offset,
                            header,
                            limit: self.config.payload_limit,
                        }))),
                    },
                    Some(_) | None => Transition {
                        next_state: State::Gathering { offset, header },
                        consume: header_length(header),
                        progress: None,
                    },
                }
            }
            (State::Header, Observation::Header(header)) => Transition {
                next_state: State::Skipping { offset, header },
                consume: header_length(header),
                progress: Some(Progress::Output(Output::Box { offset, header })),
            },
            (State::Header, Observation::MalformedHeader(source)) => Transition {
                next_state: State::Ended,
                consume: unconsumed.len(),
                progress: Some(Progress::Output(Output::Error(Error::MalformedHeader {
                    offset,
                    source,
                }))),
            },
            (State::Header, Observation::InputEndedShort) => Transition {
                next_state: State::Ended,
                consume: unconsumed.len(),
                progress: Some(Progress::Output(Output::Error(Error::TruncatedHeader {
                    offset,
                }))),
            },
            (State::Gathering { offset, header }, Observation::BoxEndInHand) => {
                let payload_len = header
                    .payload_len()
                    .and_then(|payload_len| usize::try_from(payload_len).ok())
                    .unwrap_or(unconsumed.len());
                let start = self.input.consumed();
                Transition {
                    next_state: State::Header,
                    consume: payload_len,
                    progress: Some(Progress::Output(Output::Gathered {
                        offset,
                        header,
                        payload: start..start.saturating_add(payload_len),
                    })),
                }
            }
            (State::Gathering { offset, header }, Observation::NeedMoreData)
                if u64::try_from(unconsumed.len()).unwrap_or(u64::MAX)
                    > self.config.payload_limit =>
            {
                Transition {
                    next_state: State::Skipping { offset, header },
                    consume: 0,
                    progress: Some(Progress::Output(Output::Error(Error::PayloadOverLimit {
                        offset,
                        header,
                        limit: self.config.payload_limit,
                    }))),
                }
            }
            (State::Ended, _)
            | (State::Header | State::Gathering { .. }, Observation::NeedMoreData) => Transition {
                next_state: self.state,
                consume: 0,
                progress: Some(Progress::NeedMoreData),
            },
            (
                State::Gathering { offset, header },
                Observation::InputEndedShort | Observation::InputExhausted,
            ) if header.payload_len().is_none()
                && u64::try_from(unconsumed.len()).unwrap_or(u64::MAX)
                    > self.config.payload_limit =>
            {
                Transition {
                    next_state: State::Ended,
                    consume: unconsumed.len(),
                    progress: Some(Progress::Output(Output::Error(Error::PayloadOverLimit {
                        offset,
                        header,
                        limit: self.config.payload_limit,
                    }))),
                }
            }
            (
                State::Gathering { offset, header },
                Observation::InputEndedShort | Observation::InputExhausted,
            ) if header.payload_len().is_none() => {
                let start = self.input.consumed();
                Transition {
                    next_state: State::Ended,
                    consume: unconsumed.len(),
                    progress: Some(Progress::Output(Output::Gathered {
                        offset,
                        header,
                        payload: start..self.input.len(),
                    })),
                }
            }
            (
                State::Gathering { offset, header },
                Observation::InputEndedShort | Observation::InputExhausted,
            ) => Transition {
                next_state: State::Ended,
                consume: unconsumed.len(),
                progress: Some(Progress::Output(Output::Error(Error::TruncatedBox {
                    offset,
                    header,
                }))),
            },
            (State::Skipping { offset, header }, Observation::BoxEndInHand) => Transition {
                next_state: State::Header,
                consume: end_of_box(offset, header)
                    .and_then(|end| {
                        usize::try_from(end.saturating_sub(self.offset_of(self.input.consumed())))
                            .ok()
                    })
                    .unwrap_or(unconsumed.len()),
                progress: None,
            },
            // Why consume what the box being skipped holds before more input
            // arrives: compacting can then drop it, so skipping a large
            // `mdat` handed over in pieces holds no more than one piece.
            (State::Skipping { .. }, Observation::NeedMoreData) => Transition {
                next_state: self.state,
                consume: unconsumed.len(),
                progress: Some(Progress::NeedMoreData),
            },
            (State::Skipping { .. } | State::Gathering { .. }, _)
            | (State::Header, Observation::BoxEndInHand | Observation::InputExhausted) => {
                Transition {
                    next_state: State::Ended,
                    consume: unconsumed.len(),
                    progress: Some(Progress::NeedMoreData),
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

    /// File offset of the buffer byte at `index`
    fn offset_of(&self, index: usize) -> u64 {
        // Why saturating rather than wrapping: u64 spans 16 EiB of input, so
        // the ceiling is unreachable, and stopping at it beats wrapping an
        // offset into one that names the wrong byte.
        self.position
            .saturating_add(u64::try_from(index).unwrap_or(u64::MAX))
    }
}

/// Classifies the unconsumed bytes, starting at the file offset `offset`, in `state`
fn observe(state: State, offset: u64, bytes: &[u8], input: Input) -> Observation {
    let available = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let wanted = match state {
        State::Header => {
            return match (BoxHeader::decode(bytes), input) {
                (Ok((header, _)), _) => Observation::Header(header),
                (Err(error), _) if !matches!(error.kind(), ErrorKind::TruncatedHeader { .. }) => {
                    Observation::MalformedHeader(error)
                }
                (Err(_), Input::Continues) => Observation::NeedMoreData,
                (Err(_), Input::Ends) if bytes.is_empty() => Observation::InputExhausted,
                (Err(_), Input::Ends) => Observation::InputEndedShort,
            };
        }
        State::Gathering { header, .. } => header.payload_len(),
        State::Skipping {
            offset: box_offset,
            header,
        } => end_of_box(box_offset, header).map(|end| end.saturating_sub(offset)),
        State::Ended => Some(u64::MAX),
    };
    match (wanted, input) {
        (Some(wanted), _) if available >= wanted => Observation::BoxEndInHand,
        (_, Input::Continues) => Observation::NeedMoreData,
        (_, Input::Ends) if bytes.is_empty() => Observation::InputExhausted,
        (_, Input::Ends) => Observation::InputEndedShort,
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
