//! [`BoxWriter`], the sequence of boxes of ISO/IEC 14496-12 §4.2 written as the events come

use alloc::vec::Vec;
use core::error;
use core::fmt;

use isobmff_core::{BoxHeader, BoxType, BoxWrite, EncodeError};

use crate::event::BoxEvent;

/// Writes the sequence of boxes a file is formed as, taking the events as they come
///
/// The writer is handed the steps of the sequence and lays down the bytes they
/// make. It reaches for no destination of its own: when to write and to where
/// stay with the caller. What [`BoxReader`](crate::BoxReader) reports, this
/// writes, so an event stream read from one file writes back as that file.
///
/// A box read into a value — [`FileType`](BoxEvent::FileType),
/// [`SegmentType`](BoxEvent::SegmentType), [`Movie`](BoxEvent::Movie),
/// [`MovieFragment`](BoxEvent::MovieFragment) — writes as a whole box, its header
/// settled by the payload the value forms. A box carried as it lies writes the
/// header of its [`RawStart`](BoxEvent::RawStart) as it stands, so the framing
/// stays the caller's: the writer neither shortens nor widens the `size` field it
/// was handed.
///
/// # Contract
///
/// * [`handle_event`](Self::handle_event) takes the event whole and owns the
///   bytes it made of it. Those bytes are taken from
///   [`poll_output`](Self::poll_output), which fills the buffer the caller offers
///   and reports how much of it was filled — `0` once the events handed over so
///   far are written out.
/// * The caller drains before handing over more events. Bytes are held until
///   they are taken, so writing on without polling has the writer hold the whole
///   file.
/// * How the bytes of one box are split across
///   [`poll_output`](Self::poll_output) calls follows the buffers the caller
///   offers, and nothing else. The bytes themselves, end to end, do not follow
///   them.
/// * A payload is carried by as many [`RawPayload`](BoxEvent::RawPayload) events
///   as the caller cares to send, and must measure what the box declares:
///   offering more is [`PayloadPastDeclared`](BoxWriterError::PayloadPastDeclared)
///   and closing early is [`UnfinishedBox`](BoxWriterError::UnfinishedBox). The
///   writer does not correct the header it was handed to match what arrived.
/// * A box declaring no total —
///   [`ToEndOfFile`](isobmff_core::BoxSize::ToEndOfFile) — takes payload of any
///   length and is closed by [`RawEnd`](BoxEvent::RawEnd) like any other. Nothing
///   may follow it: it runs to the end of the file by definition, so an event
///   after it is [`PastEndOfFile`](BoxWriterError::PastEndOfFile).
/// * The offsets [`BoxReader`](crate::BoxReader) reports are not this writer's
///   input — a [`BoxEvent`] carries none, see [`BoxEventAt`](crate::BoxEventAt) —
///   so boxes may be dropped from an event stream or added to it.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](BoxWriterError::AlreadyFinished) aside: every later
///   [`handle_event`](Self::handle_event) and [`finish`](Self::finish) reports
///   [`AlreadyFailed`](BoxWriterError::AlreadyFailed). The bytes made before it
///   are still there to take, and no further byte is ever made.
/// * [`finish`](Self::finish) declares the file over. Bytes are still taken after
///   it, but an event handed over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](BoxWriterError::AlreadyFinished). A file being over is
///   not a failure, so that is what every later call reports as well.
/// * [`finish`](Self::finish) reports
///   [`UnfinishedBox`](BoxWriterError::UnfinishedBox) for a box whose declared
///   total was not reached. A box that declares no total, and one whose payload
///   is all there but was not closed, end the file where it stands — the bytes
///   written already form the whole box.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::FileTypeBox;
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize, FourCC};
/// use isobmff_sequence::{BoxEvent, BoxWriter};
///
/// // A file opening with an `ftyp` box and carrying one `mdat`
/// let mdat = BoxHeader::new(
///     BoxType::compact(*b"mdat"),
///     BoxSize::Compact(CompactSize::new(12).unwrap()),
/// )
/// .unwrap();
/// let events = [
///     BoxEvent::FileType(FileTypeBox::new(
///         FourCC::new(*b"iso6"),
///         512,
///         vec![FourCC::new(*b"iso6")],
///     )),
///     BoxEvent::RawStart(mdat),
///     BoxEvent::RawPayload(b"SAMP".to_vec()),
///     BoxEvent::RawEnd,
/// ];
/// let mut writer = BoxWriter::new();
/// let mut file = Vec::new();
/// let mut buffer = [0; 8];
///
/// // Events are handed over one at a time, and what they made is drained
/// for event in events {
///     writer.handle_event(event).unwrap();
///     loop {
///         let written = writer.poll_output(&mut buffer);
///         if written == 0 {
///             break;
///         }
///         file.extend_from_slice(&buffer[..written]);
///     }
/// }
///
/// // Every box was closed, so the file ends here
/// writer.finish().unwrap();
///
/// // The `ftyp` wrote itself as a whole box, and the `mdat` came back out as it
/// // went in
/// assert_eq!(file, *b"\0\0\0\x14ftypiso6\0\0\x02\0iso6\0\0\0\x0cmdatSAMP");
/// ```
#[derive(Debug)]
pub struct BoxWriter {
    state: State,
    output: Vec<u8>,
    taken: usize,
}

impl BoxWriter {
    /// Creates a writer waiting at the start of a file
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::Between,
            output: Vec::new(),
            taken: 0,
        }
    }

    /// Takes the next step of the sequence, and makes the bytes it lays down
    ///
    /// The event is taken whole. What it made is then taken from
    /// [`poll_output`](Self::poll_output).
    ///
    /// # Errors
    ///
    /// * [`NoBoxOpen`](BoxWriterError::NoBoxOpen): a payload or an end came
    ///   while no box was open.
    /// * [`BoxStillOpen`](BoxWriterError::BoxStillOpen): a box started while the
    ///   box before it was still open.
    /// * [`PayloadPastDeclared`](BoxWriterError::PayloadPastDeclared): more
    ///   payload was offered for a box than it declares.
    /// * [`UnfinishedBox`](BoxWriterError::UnfinishedBox): a box was closed
    ///   before its declared total was reached.
    /// * [`PastEndOfFile`](BoxWriterError::PastEndOfFile): an event came after
    ///   the box running to the end of the file was closed.
    /// * [`Encode`](BoxWriterError::Encode): a value does not write as the box
    ///   it forms.
    /// * [`AlreadyFinished`](BoxWriterError::AlreadyFinished): the file was
    ///   declared over by [`finish`](Self::finish).
    /// * [`AlreadyFailed`](BoxWriterError::AlreadyFailed): a previous call
    ///   failed, and the writer takes no more events.
    pub fn handle_event(&mut self, event: BoxEvent) -> Result<(), BoxWriterError> {
        match self.state {
            State::Failed => return Err(BoxWriterError::AlreadyFailed),
            State::Finished => return Err(BoxWriterError::AlreadyFinished),
            State::EndOfFile => return Err(self.fail(BoxWriterError::PastEndOfFile)),
            State::Payload { header, .. }
                if !matches!(event, BoxEvent::RawPayload(_) | BoxEvent::RawEnd) =>
            {
                return Err(self.fail(BoxWriterError::BoxStillOpen {
                    box_type: header.box_type(),
                }));
            }
            State::Between | State::Payload { .. } => {}
        }

        match event {
            BoxEvent::FileType(ftyp) => self.write_whole(&ftyp),
            BoxEvent::SegmentType(styp) => self.write_whole(&styp),
            BoxEvent::Movie(moov) => self.write_whole(&moov),
            BoxEvent::MovieFragment(moof) => self.write_whole(&moof),
            BoxEvent::RawStart(header) => {
                let mut scratch = [0; BoxHeader::MAX_ENCODED_LEN];

                self.output.extend_from_slice(header.encode(&mut scratch));
                self.state = State::Payload { header, written: 0 };

                Ok(())
            }
            BoxEvent::RawPayload(payload) => {
                let State::Payload { header, written } = self.state else {
                    return Err(self.fail(BoxWriterError::NoBoxOpen));
                };
                let offered =
                    written.saturating_add(u64::try_from(payload.len()).unwrap_or(u64::MAX));

                if let Some(declared) = header.payload_len() {
                    if offered > declared {
                        return Err(self.fail(BoxWriterError::PayloadPastDeclared {
                            box_type: header.box_type(),
                            declared,
                            offered,
                        }));
                    }
                }

                if self.output.is_empty() {
                    self.output = payload;
                    self.taken = 0;
                } else {
                    self.output.extend_from_slice(&payload);
                }
                self.state = State::Payload {
                    header,
                    written: offered,
                };

                Ok(())
            }
            BoxEvent::RawEnd => {
                let State::Payload { header, written } = self.state else {
                    return Err(self.fail(BoxWriterError::NoBoxOpen));
                };

                match header.payload_len() {
                    Some(declared) if written < declared => {
                        Err(self.fail(unfinished(header, written)))
                    }
                    Some(_reached) => {
                        self.state = State::Between;

                        Ok(())
                    }
                    None => {
                        self.state = State::EndOfFile;

                        Ok(())
                    }
                }
            }
        }
    }

    /// Fills `buffer` with the bytes the events handed over so far made
    ///
    /// Reports how many bytes of `buffer` were filled, `0` once they are used
    /// up: more events are needed, or the file is over. Failure is reported by
    /// [`handle_event`](Self::handle_event) and [`finish`](Self::finish) alone,
    /// so this call never fails — a failed writer hands over the bytes it had
    /// already made, then nothing from there on.
    pub fn poll_output(&mut self, buffer: &mut [u8]) -> usize {
        let pending = self.output.get(self.taken..).unwrap_or_default();
        let wanted = pending.len().min(buffer.len());
        let (Some(taking), Some(slot)) = (pending.get(..wanted), buffer.get_mut(..wanted)) else {
            return 0;
        };

        slot.copy_from_slice(taking);
        self.taken = self.taken.saturating_add(wanted);
        if self.taken >= self.output.len() {
            self.output.clear();
            self.taken = 0;
        } else if self.taken >= self.output.len().saturating_sub(self.taken) {
            self.output.drain(..self.taken);
            self.taken = 0;
        }

        wanted
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`UnfinishedBox`](BoxWriterError::UnfinishedBox): a box whose declared
    ///   total was not reached is still open.
    /// * [`AlreadyFinished`](BoxWriterError::AlreadyFinished): the file was
    ///   already declared over.
    /// * [`AlreadyFailed`](BoxWriterError::AlreadyFailed): a previous call
    ///   failed, and the writer takes no more events.
    pub fn finish(&mut self) -> Result<(), BoxWriterError> {
        match self.state {
            State::Failed => Err(BoxWriterError::AlreadyFailed),
            State::Finished => Err(BoxWriterError::AlreadyFinished),
            State::Between | State::EndOfFile => {
                self.state = State::Finished;

                Ok(())
            }
            State::Payload { header, written } => match header.payload_len() {
                Some(declared) if written < declared => Err(self.fail(unfinished(header, written))),
                Some(_) | None => {
                    self.state = State::Finished;

                    Ok(())
                }
            },
        }
    }

    /// Lays down the whole box `value` forms
    fn write_whole<Value: BoxWrite>(&mut self, value: &Value) -> Result<(), BoxWriterError> {
        let needed = value.encoded_len();
        let Ok(length) = usize::try_from(needed) else {
            // Why not an error of its own for a box beyond `usize`: such a total
            // exceeds every buffer this target can hold, which is what
            // `BoxWrite::encode` reports as a short buffer for the same reason.
            return Err(self.encode_failure::<Value>(EncodeError::BufferTooShort {
                needed,
                available: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
            }));
        };
        let written = self.output.len();

        self.output.resize(written.saturating_add(length), 0);

        let encoded = value
            .encode(self.output.get_mut(written..).unwrap_or_default())
            .map(|_nothing_beyond| ());

        match encoded {
            Ok(()) => Ok(()),
            Err(error) => Err(self.encode_failure::<Value>(error)),
        }
    }

    /// Fails the writer on a value that does not write, and names the box it forms
    fn encode_failure<Value: BoxWrite>(&mut self, source: EncodeError) -> BoxWriterError {
        self.fail(BoxWriterError::Encode {
            box_type: Value::BOX_TYPE,
            source,
        })
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: BoxWriterError) -> BoxWriterError {
        self.state = State::Failed;

        failure
    }
}

impl Default for BoxWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the failure of a box the events carried `written` payload bytes of
///
/// The box declares a total, which is what makes it unfinished; the fallback is
/// a degenerate value in place of a panic the lints forbid.
fn unfinished(header: BoxHeader, written: u64) -> BoxWriterError {
    let header_len = u64::try_from(header.encoded_len()).unwrap_or(u64::MAX);

    BoxWriterError::UnfinishedBox {
        needed: header.size().total_bytes().unwrap_or(header_len),
        available: header_len.saturating_add(written),
    }
}

/// Where the writer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Between boxes, ready for the one that starts next
    Between,
    /// Laying down the payload of the box that started, `written` bytes of it so far
    Payload { header: BoxHeader, written: u64 },
    /// Closed the box running to the end of the file, so nothing may follow it
    EndOfFile,
    /// Told the file is over, and taking no more events
    Finished,
    /// Failed, and taking no more events
    Failed,
}

/// Reason a sequence of events does not write as a sequence of boxes
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BoxWriterError {
    /// Payload, or the end of a box, came while no box was open
    NoBoxOpen,
    /// Box started while the box before it was still open
    BoxStillOpen {
        /// Box type of the box left open
        box_type: BoxType,
    },
    /// More payload was offered for a box than it declares
    PayloadPastDeclared {
        /// Box type of the box that declared it
        box_type: BoxType,
        /// Bytes of payload the box declares, its header not counted
        declared: u64,
        /// Bytes of payload offered for it, the part that overran counted
        offered: u64,
    },
    /// Events ended before the declared total of a box was reached
    UnfinishedBox {
        /// Bytes the box occupies, as the `size` or `largesize` field declares
        needed: u64,
        /// Bytes of the box the events carried, header included
        available: u64,
    },
    /// Event came after the box running to the end of the file was closed
    PastEndOfFile,
    /// Value does not write as the box it forms
    Encode {
        /// Box type of the box that failed
        box_type: BoxType,
        /// Failure the box reported
        source: EncodeError,
    },
    /// File was declared over, and takes no more events
    AlreadyFinished,
    /// Writer failed, and takes no more events
    AlreadyFailed,
}

impl fmt::Display for BoxWriterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NoBoxOpen => formatter.write_str("no box is open to carry a payload or an end"),
            Self::BoxStillOpen { box_type } => {
                write!(formatter, "{box_type} box is still open")
            }
            Self::PayloadPastDeclared {
                box_type,
                declared,
                offered,
            } => write!(
                formatter,
                "{box_type} box declares a payload of {declared} bytes, and {offered} were offered"
            ),
            Self::UnfinishedBox { needed, available } => write!(
                formatter,
                "events ended {available} bytes into a box of {needed}"
            ),
            Self::PastEndOfFile => {
                formatter.write_str("box running to the end of the file was closed already")
            }
            Self::Encode { box_type, .. } => write!(formatter, "{box_type} box does not write"),
            Self::AlreadyFinished => {
                formatter.write_str("file was declared over and takes no more events")
            }
            Self::AlreadyFailed => formatter.write_str("writer failed and takes no more events"),
        }
    }
}

impl error::Error for BoxWriterError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            Self::Encode { ref source, .. } => Some(source),
            Self::NoBoxOpen
            | Self::BoxStillOpen { .. }
            | Self::PayloadPastDeclared { .. }
            | Self::UnfinishedBox { .. }
            | Self::PastEndOfFile
            | Self::AlreadyFinished
            | Self::AlreadyFailed => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use alloc::vec;
    use alloc::vec::Vec;
    use core::error::Error as _;

    use isobmff_boxes::FileTypeBox;
    use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize, EncodeError, FourCC};

    use super::{BoxEvent, BoxWriter, BoxWriterError};

    /// Header of a box declaring `total` in the compact `size` field
    fn compact_header(box_type: [u8; 4], total: u32) -> BoxHeader {
        BoxHeader::new(
            BoxType::compact(box_type),
            BoxSize::Compact(CompactSize::new(total).unwrap()),
        )
        .unwrap()
    }

    /// Header of a box running to the end of the file
    fn unbounded_header(box_type: [u8; 4]) -> BoxHeader {
        BoxHeader::new(BoxType::compact(box_type), BoxSize::ToEndOfFile).unwrap()
    }

    /// Brands a file declares itself readable as
    fn file_type() -> FileTypeBox {
        FileTypeBox::new(FourCC::new(*b"iso6"), 512, vec![FourCC::new(*b"iso6")])
    }

    /// Everything the writer has laid down, drained `buffer_length` bytes at a time
    fn drained(writer: &mut BoxWriter, buffer_length: usize) -> Vec<u8> {
        let mut buffer = vec![0; buffer_length];
        let mut bytes = Vec::new();

        loop {
            let written = writer.poll_output(&mut buffer);

            match buffer.get(..written) {
                Some([]) | None => return bytes,
                Some(taken) => bytes.extend_from_slice(taken),
            }
        }
    }

    #[test]
    fn a_value_writes_as_the_whole_box_it_forms() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::FileType(file_type()))
            .unwrap();

        assert_eq!(
            drained(&mut writer, 64),
            *b"\0\0\0\x14ftypiso6\0\0\x02\0iso6"
        );
    }

    #[test]
    fn a_box_passed_on_writes_the_header_and_the_payload_it_came_with() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 16)))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"PAYL")))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"OAD!")))
            .unwrap();
        writer.handle_event(BoxEvent::RawEnd).unwrap();
        writer.finish().unwrap();

        assert_eq!(drained(&mut writer, 64), *b"\0\0\0\x10mdatPAYLOAD!");
    }

    #[test]
    fn a_buffer_with_no_room_takes_nothing_and_leaves_the_bytes_where_they_are() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::FileType(file_type()))
            .unwrap();

        assert_eq!(writer.poll_output(&mut []), 0);
        assert_eq!(
            drained(&mut writer, 64),
            *b"\0\0\0\x14ftypiso6\0\0\x02\0iso6"
        );
    }

    #[test]
    fn more_payload_than_the_box_declares_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::RawPayload(vec![0x11; 5])),
            Err(BoxWriterError::PayloadPastDeclared {
                box_type: BoxType::compact(*b"mdat"),
                declared: 4,
                offered: 5
            })
        );
    }

    #[test]
    fn a_box_closed_before_its_declared_total_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"PA")))
            .unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::RawEnd),
            Err(BoxWriterError::UnfinishedBox {
                needed: 12,
                available: 10
            })
        );
    }

    #[test]
    fn a_payload_with_no_box_open_is_rejected() {
        let mut writer = BoxWriter::new();

        assert_eq!(
            writer.handle_event(BoxEvent::RawPayload(Vec::from(*b"PAYL"))),
            Err(BoxWriterError::NoBoxOpen)
        );
    }

    #[test]
    fn an_end_with_no_box_open_is_rejected() {
        let mut writer = BoxWriter::new();

        assert_eq!(
            writer.handle_event(BoxEvent::RawEnd),
            Err(BoxWriterError::NoBoxOpen)
        );
    }

    #[test]
    fn a_box_starting_while_the_one_before_it_is_open_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::RawStart(compact_header(*b"free", 8))),
            Err(BoxWriterError::BoxStillOpen {
                box_type: BoxType::compact(*b"mdat")
            })
        );
    }

    #[test]
    fn a_value_arriving_while_a_box_is_open_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::FileType(file_type())),
            Err(BoxWriterError::BoxStillOpen {
                box_type: BoxType::compact(*b"mdat")
            })
        );
    }

    #[test]
    fn an_event_after_the_box_running_to_the_end_of_the_file_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(unbounded_header(*b"mdat")))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"PAYL")))
            .unwrap();
        writer.handle_event(BoxEvent::RawEnd).unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::FileType(file_type())),
            Err(BoxWriterError::PastEndOfFile)
        );
    }

    #[test]
    fn a_box_running_to_the_end_of_the_file_ends_it_wherever_it_stands() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(unbounded_header(*b"mdat")))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"PAYL")))
            .unwrap();

        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(drained(&mut writer, 64), *b"\0\0\0\0mdatPAYL");
    }

    #[test]
    fn a_box_whose_declared_payload_is_all_there_ends_the_file_though_it_was_not_closed() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"PAYL")))
            .unwrap();

        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(drained(&mut writer, 64), *b"\0\0\0\x0cmdatPAYL");
    }

    #[test]
    fn a_file_ending_inside_a_box_is_rejected_as_unfinished() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"PA")))
            .unwrap();

        assert_eq!(
            writer.finish(),
            Err(BoxWriterError::UnfinishedBox {
                needed: 12,
                available: 10
            })
        );
    }

    #[test]
    fn a_failed_writer_takes_no_more_events() {
        let mut writer = BoxWriter::new();

        assert_eq!(
            writer.handle_event(BoxEvent::RawEnd),
            Err(BoxWriterError::NoBoxOpen)
        );
        assert_eq!(
            writer.handle_event(BoxEvent::FileType(file_type())),
            Err(BoxWriterError::AlreadyFailed)
        );
        assert_eq!(writer.finish(), Err(BoxWriterError::AlreadyFailed));
    }

    #[test]
    fn a_writer_that_failed_while_finishing_takes_no_more_events() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();

        assert_eq!(
            writer.finish(),
            Err(BoxWriterError::UnfinishedBox {
                needed: 12,
                available: 8
            })
        );
        assert_eq!(
            writer.handle_event(BoxEvent::RawPayload(Vec::from(*b"PAYL"))),
            Err(BoxWriterError::AlreadyFailed)
        );
        assert_eq!(writer.finish(), Err(BoxWriterError::AlreadyFailed));
    }

    #[test]
    fn a_failed_writer_hands_over_the_bytes_it_had_already_made() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::RawStart(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::RawPayload(Vec::from(*b"PA")))
            .unwrap();

        assert!(writer.handle_event(BoxEvent::RawEnd).is_err());

        assert_eq!(drained(&mut writer, 64), *b"\0\0\0\x0cmdatPA");
    }

    #[test]
    fn the_bytes_a_finished_file_had_made_are_still_taken_after_it() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::FileType(file_type()))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(
            drained(&mut writer, 64),
            *b"\0\0\0\x14ftypiso6\0\0\x02\0iso6"
        );
    }

    #[test]
    fn an_event_handed_over_after_finishing_is_rejected() {
        let mut writer = BoxWriter::new();

        writer.finish().unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::FileType(file_type())),
            Err(BoxWriterError::AlreadyFinished)
        );
    }

    #[test]
    fn finishing_a_file_that_is_already_over_is_rejected() {
        let mut writer = BoxWriter::new();

        writer.finish().unwrap();

        assert_eq!(writer.finish(), Err(BoxWriterError::AlreadyFinished));
    }

    #[test]
    fn display_of_a_payload_past_what_the_box_declares_names_both_lengths() {
        let error = BoxWriterError::PayloadPastDeclared {
            box_type: BoxType::compact(*b"mdat"),
            declared: 64,
            offered: 68,
        };

        assert_eq!(
            error.to_string(),
            "mdat box declares a payload of 64 bytes, and 68 were offered"
        );
    }

    #[test]
    fn display_of_an_unfinished_box_names_both_lengths() {
        let error = BoxWriterError::UnfinishedBox {
            needed: 16,
            available: 12,
        };

        assert_eq!(error.to_string(), "events ended 12 bytes into a box of 16");
    }

    #[test]
    fn display_of_a_value_that_does_not_write_leaves_the_reason_to_its_source() {
        let error = BoxWriterError::Encode {
            box_type: BoxType::compact(*b"moov"),
            source: EncodeError::BufferTooShort {
                needed: 24,
                available: 16,
            },
        };

        assert_eq!(error.to_string(), "moov box does not write");
        assert_eq!(
            error.source().map(ToString::to_string),
            Some(String::from(
                "value of 24 bytes needs a buffer at least that long, not 16"
            ))
        );
    }
}
