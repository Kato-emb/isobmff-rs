//! [`BoxWriter`], the sequence of boxes of ISO/IEC 14496-12 §4.2 written as the events come

use alloc::collections::VecDeque;
use core::ops::Range;

use isobmff_core::BoxHeader;

use crate::error::Error;
use crate::event::{BoxEvent, EventBytes};

/// Writes the sequence of boxes a file is formed as, taking the events as they come
///
/// The writer is handed the steps of the sequence and lays down the bytes they
/// make. It reaches for no destination of its own: when to write and to where
/// stay with the caller. What [`BoxReader`](crate::BoxReader) reports, this
/// writes, so an event stream read from one file writes back as that file.
///
/// A box writes the header of its [`Header`](BoxEvent::Header) as it stands, so
/// the framing stays the caller's: the writer neither shortens nor widens the
/// `size` field it was handed.
///
/// # Contract
///
/// * [`handle_event`](Self::handle_event) takes the event whole and makes the
///   bytes it lays down. Those bytes are handed over by
///   [`poll_output`](Self::poll_output) as one [`EventBytes`] an event, an
///   [`End`](BoxEvent::End) making none — `None` once the events handed over so
///   far are written out.
/// * The caller drains before handing over more events. Bytes are held until
///   they are taken, so writing on without polling has the writer hold the whole
///   file.
/// * The payload of an event is handed on in the allocation it arrived in:
///   [`EventBytes::into_vec`] gives back that same `Vec`, neither copied nor
///   grown. The bytes of a header are held inline until they are asked for as a
///   `Vec`.
/// * A payload is carried by as many [`Payload`](BoxEvent::Payload) events as
///   the caller cares to send, and must measure what the box declares:
///   offering more is [`PayloadPastDeclared`](crate::ErrorKind::PayloadPastDeclared)
///   and closing early is [`UnfinishedBox`](crate::ErrorKind::UnfinishedBox). The
///   writer does not correct the header it was handed to match what arrived.
/// * A box declaring no total —
///   [`ToEndOfFile`](isobmff_core::BoxSize::ToEndOfFile) — takes payload of any
///   length and is closed by [`End`](BoxEvent::End) like any other. Nothing
///   may follow it: it runs to the end of the file by definition, so an event
///   after it is [`PastEndOfFile`](crate::ErrorKind::PastEndOfFile).
/// * A [`BoxEvent`] carries no position, so the extents
///   [`BoxReader`](crate::BoxReader) reported are not this writer's input: boxes
///   may be dropped from an event stream or added to it, and where the events
///   land is the writer's own count. [`event_extent`](Self::event_extent) names
///   it for the event last handed over.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside: every later
///   [`handle_event`](Self::handle_event) and [`finish`](Self::finish) reports
///   that same failure again. The bytes made before it are still there to take,
///   and no further byte is ever made.
/// * [`finish`](Self::finish) declares the file over. Bytes are still taken after
///   it, but an event handed over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished). A file being over is
///   not a failure, so that is what every later call reports as well.
/// * [`finish`](Self::finish) reports
///   [`UnfinishedBox`](crate::ErrorKind::UnfinishedBox) for a box whose declared
///   total was not reached. A box that declares no total, and one whose payload
///   is all there but was not closed, end the file where it stands — the bytes
///   written already form the whole box.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize};
/// use isobmff_sequence::{BoxEvent, BoxWriter};
///
/// // A file opening with an `ftyp` box and carrying one `mdat`
/// let whole_box = |box_type, total| {
///     BoxHeader::new(box_type, BoxSize::Compact(CompactSize::new(total).unwrap())).unwrap()
/// };
/// let events = [
///     BoxEvent::Header(whole_box(BoxType::compact(*b"ftyp"), 20)),
///     BoxEvent::Payload(b"iso6\0\0\x02\0iso6".to_vec()),
///     BoxEvent::End,
///     BoxEvent::Header(whole_box(BoxType::compact(*b"mdat"), 12)),
///     BoxEvent::Payload(b"SAMP".to_vec()),
///     BoxEvent::End,
/// ];
/// let mut writer = BoxWriter::new();
/// let mut file = Vec::new();
/// let mut extents = Vec::new();
///
/// // Events are handed over one at a time, each with the bytes of the file it
/// // was written to, and what they made is drained
/// for event in events {
///     writer.handle_event(event).unwrap();
///     extents.push(writer.event_extent().unwrap());
///     while let Some(written) = writer.poll_output() {
///         file.extend_from_slice(&written);
///     }
/// }
///
/// // Every box was closed, so the file ends here
/// writer.finish().unwrap();
///
/// // Both boxes came back out as they went in
/// assert_eq!(file, *b"\0\0\0\x14ftypiso6\0\0\x02\0iso6\0\0\0\x0cmdatSAMP");
///
/// // Each event landed where the one before it ended, as the reader reports the
/// // same file
/// assert_eq!(extents, [0..8, 8..20, 20..20, 20..28, 28..32, 32..32]);
/// ```
#[derive(Debug)]
pub struct BoxWriter {
    state: State,
    /// The bytes still to be drained, as the events made them
    output: VecDeque<EventBytes>,
    position: u64,
    event_extent: Option<Range<u64>>,
}

impl BoxWriter {
    /// Creates a writer waiting at the start of a file
    ///
    /// The extents the writer reports count from the first byte it lays down,
    /// which it takes as offset zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::Between,
            output: VecDeque::new(),
            position: 0,
            event_extent: None,
        }
    }

    /// Takes the next step of the sequence, and makes the bytes it lays down
    ///
    /// The event is taken whole. What it made is then taken from
    /// [`poll_output`](Self::poll_output).
    ///
    /// # Errors
    ///
    /// * [`NoBoxOpen`](crate::ErrorKind::NoBoxOpen): a payload or an end came
    ///   while no box was open.
    /// * [`BoxStillOpen`](crate::ErrorKind::BoxStillOpen): a box started while the
    ///   box before it was still open.
    /// * [`PayloadPastDeclared`](crate::ErrorKind::PayloadPastDeclared): more
    ///   payload was offered for a box than it declares.
    /// * [`UnfinishedBox`](crate::ErrorKind::UnfinishedBox): a box was closed
    ///   before its declared total was reached.
    /// * [`PastEndOfFile`](crate::ErrorKind::PastEndOfFile): an event came after
    ///   the box running to the end of the file was closed.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the file was
    ///   declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_event(&mut self, event: BoxEvent) -> Result<(), Error> {
        let began_at = self.position;
        let length = match (self.state, event) {
            (State::Failed(failure), _) => return Err(failure),
            (State::Finished, _) => return Err(Error::already_finished()),
            (State::EndOfFile, _) => return Err(self.fail(Error::past_end_of_file())),
            (
                State::Payload { header, .. } | State::PayloadToEndOfFile { header },
                BoxEvent::Header(_),
            ) => return Err(self.fail(Error::box_still_open(header.box_type()))),
            (State::Between, BoxEvent::Payload(_) | BoxEvent::End) => {
                return Err(self.fail(Error::no_box_open()));
            }
            (State::Between, BoxEvent::Header(header)) => {
                let header_len = header.encoded_len() as u64;

                self.output.push_back(EventBytes::header(header));
                self.state = match header.payload_len() {
                    Some(declared) => State::Payload {
                        header,
                        declared,
                        written: 0,
                    },
                    None => State::PayloadToEndOfFile { header },
                };

                header_len
            }
            (
                State::Payload {
                    header,
                    declared,
                    written,
                },
                BoxEvent::Payload(payload),
            ) => {
                let length = payload.len() as u64;
                let offered = written.saturating_add(length);

                if offered > declared {
                    return Err(self.fail(Error::payload_past_declared(
                        header.box_type(),
                        declared,
                        offered,
                    )));
                }
                self.output.push_back(EventBytes::payload(payload));
                self.state = State::Payload {
                    header,
                    declared,
                    written: offered,
                };

                length
            }
            (State::PayloadToEndOfFile { .. }, BoxEvent::Payload(payload)) => {
                let length = payload.len() as u64;

                self.output.push_back(EventBytes::payload(payload));

                length
            }
            (
                State::Payload {
                    header,
                    declared,
                    written,
                },
                BoxEvent::End,
            ) => {
                if written < declared {
                    return Err(self.fail(unfinished(header, declared, written)));
                }
                self.state = State::Between;

                0
            }
            (State::PayloadToEndOfFile { .. }, BoxEvent::End) => {
                self.state = State::EndOfFile;

                0
            }
        };

        self.position = self.position.saturating_add(length);
        self.event_extent = Some(began_at..self.position);

        Ok(())
    }

    /// Returns the bytes of the file the event last handed over was written to
    ///
    /// The extent counts from the first byte the writer laid down, and covers
    /// the bytes that event is made of — see [`BoxEvent`]. They are the extent
    /// of the event whether they were drained by
    /// [`poll_output`](Self::poll_output) or are still held.
    ///
    /// It is the event [`handle_event`](Self::handle_event) took last that it
    /// names, and `None` until the first is taken. The events partition the
    /// output, so the end of one is where the next begins.
    #[must_use]
    pub fn event_extent(&self) -> Option<Range<u64>> {
        self.event_extent.clone()
    }

    /// Hands over the bytes the next event made
    ///
    /// Reports `None` once the bytes made so far are taken: more events are
    /// needed, or the file is over. A call hands over what one event made,
    /// whole. Failure is reported by [`handle_event`](Self::handle_event) and
    /// [`finish`](Self::finish) alone, so this call never fails — a failed
    /// writer hands over the bytes it had already made, then nothing from there
    /// on.
    pub fn poll_output(&mut self) -> Option<EventBytes> {
        self.output.pop_front()
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`UnfinishedBox`](crate::ErrorKind::UnfinishedBox): a box whose declared
    ///   total was not reached is still open.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the file was
    ///   already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            State::Failed(failure) => Err(failure),
            State::Finished => Err(Error::already_finished()),
            State::Payload {
                header,
                declared,
                written,
            } if written < declared => Err(self.fail(unfinished(header, declared, written))),
            State::Between
            | State::Payload { .. }
            | State::PayloadToEndOfFile { .. }
            | State::EndOfFile => {
                self.state = State::Finished;

                Ok(())
            }
        }
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for BoxWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the failure of a box declaring `declared` payload bytes that carried `written`
fn unfinished(header: BoxHeader, declared: u64, written: u64) -> Error {
    let header_len = header.encoded_len() as u64;

    Error::unfinished_box(
        header_len.saturating_add(declared),
        header_len.saturating_add(written),
    )
}

/// Where the writer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Between boxes, ready for the one that starts next
    Between,
    /// Laying down the `declared` payload bytes of the box that started, `written` so far
    Payload {
        header: BoxHeader,
        declared: u64,
        written: u64,
    },
    /// Laying down the payload of the box running to the end of the file
    PayloadToEndOfFile { header: BoxHeader },
    /// Closed the box running to the end of the file, so nothing may follow it
    EndOfFile,
    /// Told the file is over, and taking no more events
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize};

    use super::{BoxEvent, BoxWriter, Error, EventBytes};

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

    /// Everything the writer has laid down
    fn drained(writer: &mut BoxWriter) -> Vec<u8> {
        let mut bytes = Vec::new();

        while let Some(written) = writer.poll_output() {
            bytes.extend_from_slice(&written);
        }

        bytes
    }

    #[test]
    fn a_box_writes_the_header_and_the_payload_it_came_with() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 16)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PAYL")))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"OAD!")))
            .unwrap();
        writer.handle_event(BoxEvent::End).unwrap();
        writer.finish().unwrap();

        assert_eq!(drained(&mut writer), *b"\0\0\0\x10mdatPAYLOAD!");
    }

    #[test]
    fn the_bytes_of_an_event_come_out_one_event_at_a_time() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PAYL")))
            .unwrap();

        assert_eq!(
            writer.poll_output().map(EventBytes::into_vec),
            Some(Vec::from(*b"\0\0\0\x0cmdat"))
        );
        assert_eq!(
            writer.poll_output().map(EventBytes::into_vec),
            Some(Vec::from(*b"PAYL"))
        );
        assert_eq!(writer.poll_output(), None);
    }

    #[test]
    fn the_payload_of_an_event_is_handed_on_in_the_allocation_it_arrived_in() {
        let mut writer = BoxWriter::new();
        let payload = Vec::from(*b"PAYL");
        let arrived_in = payload.as_ptr();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer.handle_event(BoxEvent::Payload(payload)).unwrap();
        writer.poll_output().unwrap();
        let handed_over = writer.poll_output().unwrap().into_vec();

        assert_eq!(handed_over.as_ptr(), arrived_in);
    }

    #[test]
    fn more_payload_than_the_box_declares_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::Payload(vec![0x11; 5])),
            Err(Error::payload_past_declared(
                BoxType::compact(*b"mdat"),
                4,
                5
            ))
        );
    }

    #[test]
    fn a_box_closed_before_its_declared_total_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PA")))
            .unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::End),
            Err(Error::unfinished_box(12, 10))
        );
    }

    #[test]
    fn a_payload_with_no_box_open_is_rejected() {
        let mut writer = BoxWriter::new();

        assert_eq!(
            writer.handle_event(BoxEvent::Payload(Vec::from(*b"PAYL"))),
            Err(Error::no_box_open())
        );
    }

    #[test]
    fn an_end_with_no_box_open_is_rejected() {
        let mut writer = BoxWriter::new();

        assert_eq!(
            writer.handle_event(BoxEvent::End),
            Err(Error::no_box_open())
        );
    }

    #[test]
    fn a_box_starting_while_the_one_before_it_is_open_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::Header(compact_header(*b"free", 8))),
            Err(Error::box_still_open(BoxType::compact(*b"mdat")))
        );
    }

    #[test]
    fn an_event_after_the_box_running_to_the_end_of_the_file_is_rejected() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(unbounded_header(*b"mdat")))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PAYL")))
            .unwrap();
        writer.handle_event(BoxEvent::End).unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::Header(compact_header(*b"free", 8))),
            Err(Error::past_end_of_file())
        );
    }

    #[test]
    fn a_box_running_to_the_end_of_the_file_ends_it_wherever_it_stands() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(unbounded_header(*b"mdat")))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PAYL")))
            .unwrap();

        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(drained(&mut writer), *b"\0\0\0\0mdatPAYL");
    }

    #[test]
    fn a_box_whose_declared_payload_is_all_there_ends_the_file_though_it_was_not_closed() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PAYL")))
            .unwrap();

        assert_eq!(writer.finish(), Ok(()));
        assert_eq!(drained(&mut writer), *b"\0\0\0\x0cmdatPAYL");
    }

    #[test]
    fn a_file_ending_inside_a_box_is_rejected_as_unfinished() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PA")))
            .unwrap();

        assert_eq!(writer.finish(), Err(Error::unfinished_box(12, 10)));
    }

    #[test]
    fn a_failed_writer_reports_the_same_failure_for_every_call_after_it() {
        let mut writer = BoxWriter::new();
        let failure = Error::no_box_open();

        assert_eq!(writer.handle_event(BoxEvent::End), Err(failure));
        assert_eq!(
            writer.handle_event(BoxEvent::Header(compact_header(*b"free", 8))),
            Err(failure)
        );
        assert_eq!(writer.finish(), Err(failure));
    }

    #[test]
    fn a_writer_that_failed_while_finishing_reports_that_failure_again() {
        let mut writer = BoxWriter::new();
        let failure = Error::unfinished_box(12, 8);

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();

        assert_eq!(writer.finish(), Err(failure));
        assert_eq!(
            writer.handle_event(BoxEvent::Payload(Vec::from(*b"PAYL"))),
            Err(failure)
        );
        assert_eq!(writer.finish(), Err(failure));
    }

    #[test]
    fn a_failed_writer_hands_over_the_bytes_it_had_already_made() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PA")))
            .unwrap();

        assert!(writer.handle_event(BoxEvent::End).is_err());

        assert_eq!(drained(&mut writer), *b"\0\0\0\x0cmdatPA");
    }

    #[test]
    fn the_bytes_a_finished_file_had_made_are_still_taken_after_it() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PAYL")))
            .unwrap();
        writer.finish().unwrap();

        assert_eq!(drained(&mut writer), *b"\0\0\0\x0cmdatPAYL");
    }

    #[test]
    fn the_extent_reported_is_the_one_of_the_event_handed_over_last() {
        let mut writer = BoxWriter::new();

        assert_eq!(writer.event_extent(), None);

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"free", 12)))
            .unwrap();

        assert_eq!(writer.event_extent(), Some(0..8));

        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"AAAA")))
            .unwrap();

        assert_eq!(writer.event_extent(), Some(8..12));

        writer.handle_event(BoxEvent::End).unwrap();

        assert_eq!(writer.event_extent(), Some(12..12));
    }

    #[test]
    fn the_extent_of_an_event_covers_the_bytes_it_made_though_the_ones_before_it_were_drained() {
        let mut writer = BoxWriter::new();

        writer
            .handle_event(BoxEvent::Header(compact_header(*b"mdat", 12)))
            .unwrap();
        writer
            .handle_event(BoxEvent::Payload(Vec::from(*b"PAYL")))
            .unwrap();
        writer.handle_event(BoxEvent::End).unwrap();
        drained(&mut writer);
        writer
            .handle_event(BoxEvent::Header(compact_header(*b"free", 8)))
            .unwrap();

        assert_eq!(writer.event_extent(), Some(12..20));
    }

    #[test]
    fn an_event_handed_over_after_finishing_is_rejected() {
        let mut writer = BoxWriter::new();

        writer.finish().unwrap();

        assert_eq!(
            writer.handle_event(BoxEvent::Header(compact_header(*b"free", 8))),
            Err(Error::already_finished())
        );
    }

    #[test]
    fn finishing_a_file_that_is_already_over_is_rejected() {
        let mut writer = BoxWriter::new();

        writer.finish().unwrap();

        assert_eq!(writer.finish(), Err(Error::already_finished()));
    }
}
