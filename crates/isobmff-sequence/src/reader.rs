//! [`BoxReader`], the sequence of boxes of ISO/IEC 14496-12 §4.2 read as the input arrives

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::ops::Range;

use isobmff_core::{BoxHeader, ErrorKind as BoxErrorKind};

use crate::error::Error;
use crate::event::BoxEvent;

/// Takes off `input` the payload bytes the box that started has still to come
///
/// `remaining` is `None` for a box that declares no total, which only the end of
/// the file ends: every byte `input` offers is taken.
fn take_payload<'input>(remaining: Option<u64>, input: &mut &'input [u8]) -> &'input [u8] {
    let wanted = remaining
        .and_then(|remaining| usize::try_from(remaining).ok())
        .unwrap_or(usize::MAX)
        .min(input.len());
    let (taken, rest) = input.split_at(wanted);

    *input = rest;

    taken
}

/// Reads the sequence of boxes a file is formed as, taking the input as it arrives
///
/// The reader is handed the input as it arrives and reports the boxes it frames
/// as owned [`BoxEvent`]s, with the bytes each of them covers named by
/// [`event_extent`](Self::event_extent). It reaches for no source of its own:
/// when to read and from where stay with the caller.
///
/// Every box is passed on as it lies: the reader frames the file and reads no
/// box into a value, so which boxes matter and what their payloads mean stay
/// with the caller.
///
/// # Contract
///
/// * [`handle_input`](Self::handle_input) takes the input whole and owns what it
///   made of it, so the buffer is the caller's again once the call returns. The
///   events are taken one at a time from [`poll_event`](Self::poll_event),
///   which reports `None` once the input handed over so far is used up.
/// * The caller drains before handing over more input. Events are held until
///   they are taken, so reading on without polling has the reader hold the
///   whole file.
/// * A [`Payload`](BoxEvent::Payload) is never empty. A box with no payload is
///   a [`Header`](BoxEvent::Header) followed by an [`End`](BoxEvent::End).
/// * The events partition the input: each one covers the bytes it was made
///   from, and the extent of one ends where the extent of the next begins.
///   [`event_extent`](Self::event_extent) names it for the event last taken.
/// * A box declaring no total —
///   [`ToEndOfFile`](isobmff_core::BoxSize::ToEndOfFile) — takes every byte
///   that arrives after its header, and only [`finish`](Self::finish) closes
///   it.
/// * Where a payload is cut into [`Payload`](BoxEvent::Payload) events follows
///   how the caller cut the file, and so does the extent each of those events
///   covers. What does not follow it: the [`Header`](BoxEvent::Header) and
///   [`End`](BoxEvent::End) events with the extents they cover, and the payload
///   bytes those events hold end to end.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside: every later
///   [`handle_input`](Self::handle_input) and [`finish`](Self::finish) reports
///   that same failure again. The events made before it are still there to take,
///   and no further one is ever made.
/// * [`finish`](Self::finish) declares the file over. Events are still taken
///   after it, but input handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished). A file being over is
///   not a failure, so that is what every later call reports as well.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize};
/// use isobmff_sequence::{BoxEvent, BoxReader};
///
/// // A file opening with an `ftyp` box and carrying one `mdat`, arriving cut
/// // across both of them
/// let arriving: [&[u8]; 3] = [
///     b"\0\0\0\x14ftypiso6\0\0\x02",
///     b"\0iso6\0\0\0\x0cmdat",
///     b"SAMP",
/// ];
/// let mut reader = BoxReader::new();
/// let mut events = Vec::new();
///
/// // Input is handed over whole, then what it completed is drained, each event
/// // with the bytes of the file it was read from
/// for input in arriving {
///     reader.handle_input(input).unwrap();
///     while let Some(event) = reader.poll_event() {
///         events.push((reader.event_extent().unwrap(), event));
///     }
/// }
///
/// // The file ended on a box boundary, so nothing was left open
/// reader.finish().unwrap();
/// assert_eq!(reader.poll_event(), None);
///
/// // Both boxes are framed and passed on as they lie, the `ftyp` payload
/// // arriving in the two parts the cuts made of it
/// let whole_box = |box_type, total| {
///     BoxHeader::new(box_type, BoxSize::Compact(CompactSize::new(total).unwrap())).unwrap()
/// };
/// assert_eq!(
///     events,
///     [
///         (0..8, BoxEvent::Header(whole_box(BoxType::compact(*b"ftyp"), 20))),
///         (8..15, BoxEvent::Payload(b"iso6\0\0\x02".to_vec())),
///         (15..20, BoxEvent::Payload(b"\0iso6".to_vec())),
///         (20..20, BoxEvent::End),
///         (20..28, BoxEvent::Header(whole_box(BoxType::compact(*b"mdat"), 12))),
///         (28..32, BoxEvent::Payload(b"SAMP".to_vec())),
///         (32..32, BoxEvent::End),
///     ]
/// );
/// ```
#[derive(Debug)]
pub struct BoxReader {
    state: State,
    events: VecDeque<(Range<u64>, BoxEvent)>,
    event_extent: Option<Range<u64>>,
    position: u64,
    queued_position: u64,
}

impl BoxReader {
    /// Creates a reader waiting at the start of a file
    ///
    /// The extents the reader reports count from the first byte handed to it,
    /// which it takes as offset zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::GATHERING_HEADER,
            events: VecDeque::new(),
            event_extent: None,
            position: 0,
            queued_position: 0,
        }
    }

    /// Takes the input that arrived, and makes the events it completes
    ///
    /// The input is taken whole. What it completed is then taken from
    /// [`poll_event`](Self::poll_event); empty input completes nothing and
    /// leaves the reader where it was.
    ///
    /// How the caller cuts the file is its own to choose. Around a megabyte at
    /// a time is what reads fastest; handing the whole file over at once is the
    /// slowest way to offer it.
    ///
    /// # Errors
    ///
    /// * The failures of [`BoxHeader::decode`], carried on
    ///   [`Box`](crate::ErrorKind::Box): a header does not decode.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the file was declared
    ///   over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_input(&mut self, input: &[u8]) -> Result<(), Error> {
        let mut unread = input;

        loop {
            match self.state {
                State::Failed(failure) => return Err(failure),
                State::Finished => return Err(Error::already_finished()),
                State::Header { mut bytes, filled } => {
                    let reached = match BoxHeader::decode(bytes.get(..filled).unwrap_or(&[])) {
                        Ok((header, _nothing_beyond)) => {
                            self.state = State::Payload {
                                header,
                                remaining: header.payload_len(),
                            };
                            self.push_event(BoxEvent::Header(header));
                            continue;
                        }
                        Err(error) if error.kind() == BoxErrorKind::TruncatedHeader => error
                            .needed_bytes()
                            .and_then(|needed| usize::try_from(needed).ok())
                            .unwrap_or(BoxHeader::MAX_ENCODED_LEN),
                        Err(error) => return Err(self.fail(error.into())),
                    };
                    let wanted = reached.saturating_sub(filled).min(unread.len());

                    if wanted == 0 {
                        return Ok(());
                    }

                    let end = filled.saturating_add(wanted);
                    // Why not unreachable: `reached` is a header length and the
                    // buffer is the longest header a box can carry, and the
                    // fallback is a degenerate value in place of a panic the
                    // lints forbid.
                    let Some(slot) = bytes.get_mut(filled..end) else {
                        return Ok(());
                    };
                    let (taken, rest) = unread.split_at(wanted);

                    slot.copy_from_slice(taken);
                    unread = rest;
                    self.state = State::Header { bytes, filled: end };
                    // Why count here rather than under the event: a header the
                    // input cut across takes bytes off every part of it while
                    // completing no event, so counting only what an event took
                    // would leave the offset short by the head of that header.
                    self.advance(wanted);
                }
                State::Payload { header, remaining } => {
                    if remaining == Some(0) {
                        self.state = State::GATHERING_HEADER;
                        self.push_event(BoxEvent::End);
                        continue;
                    }
                    if unread.is_empty() {
                        return Ok(());
                    }

                    let payload = take_payload(remaining, &mut unread);

                    self.state = State::Payload {
                        header,
                        remaining: remaining
                            .map(|remaining| remaining.saturating_sub(payload.len() as u64)),
                    };
                    self.advance(payload.len());
                    self.push_event(BoxEvent::Payload(Vec::from(payload)));
                }
            }
        }
    }

    /// Takes the next event the input handed over so far completed
    ///
    /// Reports `None` once it is used up: more input is needed, or
    /// [`finish`](Self::finish) is. Failure is reported by
    /// [`handle_input`](Self::handle_input) and [`finish`](Self::finish) alone, so
    /// this call never fails — a failed reader hands over the events it had
    /// already made, then `None` from there on.
    ///
    /// The bytes the event taken was read from are named by
    /// [`event_extent`](Self::event_extent) until the next event is taken.
    pub fn poll_event(&mut self) -> Option<BoxEvent> {
        let (extent, event) = self.events.pop_front()?;

        self.event_extent = Some(extent);

        Some(event)
    }

    /// Returns the bytes of the file the event last taken was read from
    ///
    /// The extent counts from the first byte handed to the reader, and covers
    /// the bytes that event was read from — see [`BoxEvent`]. A sample layer
    /// resolves the offsets a box declares against it, adding the origin the
    /// file was read from.
    ///
    /// It is the event [`poll_event`](Self::poll_event) reported last that it
    /// names, and `None` until the first is taken. Where in the file the reader
    /// stands otherwise is no report of its own: an extent belongs to an event
    /// and the events partition the input, so the end of one is where the next
    /// begins.
    #[must_use]
    pub fn event_extent(&self) -> Option<Range<u64>> {
        self.event_extent.clone()
    }

    /// Declares the file over, and closes the box left open
    ///
    /// A box is left open either because the file reached its declared total
    /// without the [`End`](BoxEvent::End) that follows having been made, or
    /// because it declares no total at all —
    /// [`ToEndOfFile`](isobmff_core::BoxSize::ToEndOfFile), which only the end
    /// of the file ends. Both are closed with an [`End`](BoxEvent::End), taken
    /// from [`poll_event`](Self::poll_event) like any other event.
    ///
    /// # Errors
    ///
    /// * [`UnfinishedHeader`](crate::ErrorKind::UnfinishedHeader): the file ended
    ///   inside a box header.
    /// * [`UnfinishedBox`](crate::ErrorKind::UnfinishedBox): the file ended before
    ///   the declared total of a box was reached.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            State::Failed(failure) => Err(failure),
            State::Finished => Err(Error::already_finished()),
            State::Header { filled: 0, .. } => {
                self.state = State::Finished;
                Ok(())
            }
            State::Header { bytes, filled } => {
                let reached = BoxHeader::decode(bytes.get(..filled).unwrap_or(&[]))
                    .err()
                    .filter(|error| error.kind() == BoxErrorKind::TruncatedHeader)
                    .and_then(|error| error.needed_bytes())
                    .unwrap_or(BoxHeader::MAX_ENCODED_LEN as u64);

                Err(self.fail(Error::unfinished_header(reached, filled as u64)))
            }
            State::Payload { header, remaining } => {
                let unfinished = remaining
                    .filter(|remaining| *remaining != 0)
                    .zip(header.size().total_bytes());

                match unfinished {
                    Some((remaining, total)) => Err(self.fail(Error::unfinished_box(
                        total,
                        total.saturating_sub(remaining),
                    ))),
                    None => {
                        self.state = State::Finished;
                        self.push_event(BoxEvent::End);
                        Ok(())
                    }
                }
            }
        }
    }

    /// Queues `event`, over the input read since the event before it
    ///
    /// The events partition the input, so the extent of `event` reaches from
    /// where the last one ended to where the reader stands: every byte it was
    /// made of is counted by [`advance`](Self::advance) before it is queued.
    fn push_event(&mut self, event: BoxEvent) {
        let extent = self.queued_position..self.position;

        self.queued_position = self.position;
        self.events.push_back((extent, event));
    }

    /// Counts `taken` bytes as read, moving the position the next box begins at
    fn advance(&mut self, taken: usize) {
        self.position = self.position.saturating_add(taken as u64);
    }

    /// Fails the reader for good, and hands the failure back to report
    fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for BoxReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the reader stands between calls
#[derive(Clone, Debug)]
enum State {
    /// Gathering the header of the box that starts next, `filled` bytes of it
    /// in `bytes` — the longest header a box can carry, so any form fits whole
    Header {
        bytes: [u8; BoxHeader::MAX_ENCODED_LEN],
        filled: usize,
    },
    /// Passing on the payload of the box that started, with `remaining` bytes
    /// of it still to come — `None` while the box declares no total
    Payload {
        header: BoxHeader,
        remaining: Option<u64>,
    },
    /// Told the file is over, and taking no more input
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

impl State {
    /// Waiting for the header of the box that starts next, none of it gathered
    const GATHERING_HEADER: Self = Self::Header {
        bytes: [0; BoxHeader::MAX_ENCODED_LEN],
        filled: 0,
    };
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize, ExtendedSize, Uuid};

    use super::{BoxEvent, BoxReader, Error, Range};

    /// Every form a header takes: the two size fields against the two box types
    const EVERY_HEADER_FORM: [&[u8]; 6] = [
        &[0x00, 0x00, 0x00, 0x10, b'f', b'r', b'e', b'e'],
        &[
            0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x20,
        ],
        &[
            0x00, 0x00, 0x00, 0x20, b'u', b'u', b'i', b'd', 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ],
        &[
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x20, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98,
            0x76, 0x54, 0x32, 0x10,
        ],
        &[0x00, 0x00, 0x00, 0x00, b'm', b'd', b'a', b't'],
        &[
            0x00, 0x00, 0x00, 0x00, b'u', b'u', b'i', b'd', 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ],
    ];

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

    /// The box `header` introduces, its header beginning `began_at` bytes into the file
    fn started(header: BoxHeader, began_at: u64) -> (Range<u64>, BoxEvent) {
        let header_length = u64::try_from(header.encoded_len()).unwrap();

        (
            began_at..began_at.checked_add(header_length).unwrap(),
            BoxEvent::Header(header),
        )
    }

    /// The part of a payload that lay `began_at` bytes into the file
    fn passed_on(payload: &[u8], began_at: u64) -> (Range<u64>, BoxEvent) {
        let length = u64::try_from(payload.len()).unwrap();

        (
            began_at..began_at.checked_add(length).unwrap(),
            BoxEvent::Payload(Vec::from(payload)),
        )
    }

    /// The end of a box, standing empty just past it
    fn ended(ends_at: u64) -> (Range<u64>, BoxEvent) {
        (ends_at..ends_at, BoxEvent::End)
    }

    /// The next event the reader reports, with the bytes it was read from
    fn polled(reader: &mut BoxReader) -> Option<(Range<u64>, BoxEvent)> {
        let event = reader.poll_event()?;

        Some((reader.event_extent().unwrap(), event))
    }

    /// Every event a reader reports for `input`, handed over `cut_length` bytes at a time
    fn events_of(input: &[u8], cut_length: usize) -> Vec<(Range<u64>, BoxEvent)> {
        let mut reader = BoxReader::new();
        let mut events = Vec::new();

        for arriving in input.chunks(cut_length) {
            reader.handle_input(arriving).unwrap();
            while let Some(event) = polled(&mut reader) {
                events.push(event);
            }
        }
        reader.finish().unwrap();
        while let Some(event) = polled(&mut reader) {
            events.push(event);
        }

        events
    }

    #[test]
    fn a_header_of_any_form_is_read_however_the_input_was_cut() {
        for encoded in EVERY_HEADER_FORM {
            let (header, _nothing_beyond) = BoxHeader::decode(encoded).unwrap();

            for cut_length in 1..=encoded.len() {
                let mut reader = BoxReader::new();

                for arriving in encoded.chunks(cut_length) {
                    reader.handle_input(arriving).unwrap();
                }

                assert_eq!(
                    reader.poll_event(),
                    Some(BoxEvent::Header(header)),
                    "{encoded:02x?} cut every {cut_length}"
                );
                assert_eq!(reader.event_extent(), Some(0..encoded.len() as u64));
            }
        }
    }

    #[test]
    fn the_extent_reported_is_the_one_of_the_event_taken_last() {
        let mut reader = BoxReader::new();

        reader.handle_input(b"\0\0\0\x0cfreeAAAA").unwrap();

        assert_eq!(reader.event_extent(), None);
        assert_eq!(
            reader.poll_event(),
            Some(BoxEvent::Header(compact_header(*b"free", 12)))
        );
        assert_eq!(reader.event_extent(), Some(0..8));
        assert_eq!(
            reader.poll_event(),
            Some(BoxEvent::Payload(Vec::from(*b"AAAA")))
        );
        assert_eq!(reader.event_extent(), Some(8..12));
        assert_eq!(reader.poll_event(), Some(BoxEvent::End));
        assert_eq!(reader.poll_event(), None);
        assert_eq!(reader.event_extent(), Some(12..12));
    }

    #[test]
    fn a_box_arriving_whole_is_reported_as_a_start_a_payload_and_an_end() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA", 12),
            vec![
                started(compact_header(*b"free", 12), 0),
                passed_on(b"AAAA", 8),
                ended(12),
            ]
        );
    }

    #[test]
    fn a_box_with_no_payload_is_reported_as_a_start_and_an_end() {
        assert_eq!(
            events_of(b"\0\0\0\x08skip", 8),
            vec![started(compact_header(*b"skip", 8), 0), ended(8)]
        );
    }

    #[test]
    fn a_payload_is_passed_on_as_the_input_cut_it() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA", 5),
            vec![
                started(compact_header(*b"free", 12), 0),
                passed_on(b"AA", 8),
                passed_on(b"AA", 10),
                ended(12),
            ]
        );
    }

    #[test]
    fn boxes_laid_end_to_end_are_reported_over_the_extent_each_one_covers() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip", 5),
            vec![
                started(compact_header(*b"free", 12), 0),
                passed_on(b"AA", 8),
                passed_on(b"AA", 10),
                ended(12),
                started(compact_header(*b"skip", 8), 12),
                ended(20),
            ]
        );
    }

    #[test]
    fn a_box_behind_a_long_header_is_reported_from_where_that_header_begins() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x21, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98,
            0x76, 0x54, 0x32, 0x10, b'!', 0x00, 0x00, 0x00, 0x08, b's', b'k', b'i', b'p',
        ];
        let header = BoxHeader::new(
            BoxType::Extended(USER_TYPE),
            BoxSize::Extended(ExtendedSize::new(33).unwrap()),
        )
        .unwrap();

        assert_eq!(
            events_of(&input, 3),
            vec![
                started(header, 0),
                passed_on(b"!", 32),
                ended(33),
                started(compact_header(*b"skip", 8), 33),
                ended(41),
            ]
        );
    }

    #[test]
    fn a_box_running_to_the_end_of_the_file_is_closed_by_finishing() {
        assert_eq!(
            events_of(b"\0\0\0\0mdatPAYLOAD", 4),
            vec![
                started(
                    BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap(),
                    0
                ),
                passed_on(b"PAYL", 8),
                passed_on(b"OAD", 12),
                ended(15),
            ]
        );
    }

    #[test]
    fn empty_input_completes_nothing_and_leaves_the_reader_where_it_was() {
        let mut reader = BoxReader::new();

        reader.handle_input(b"\0\0\0\x08fr").unwrap();
        reader.handle_input(b"").unwrap();

        assert_eq!(reader.poll_event(), None);

        reader.handle_input(b"ee").unwrap();

        assert_eq!(
            polled(&mut reader),
            Some(started(compact_header(*b"free", 8), 0))
        );
    }

    #[test]
    fn empty_input_arriving_inside_a_payload_asks_for_more() {
        let mut reader = BoxReader::new();

        reader.handle_input(b"\0\0\0\x0cfreeAA").unwrap();
        while reader.poll_event().is_some() {}
        reader.handle_input(b"").unwrap();

        assert_eq!(reader.poll_event(), None);

        reader.handle_input(b"AA").unwrap();

        assert_eq!(polled(&mut reader), Some(passed_on(b"AA", 10)));
    }

    #[test]
    fn a_file_stopping_inside_a_header_is_rejected_as_unfinished() {
        let mut reader = BoxReader::new();

        reader
            .handle_input(&[0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00])
            .unwrap();

        assert_eq!(reader.finish(), Err(Error::unfinished_header(16, 9)));
    }

    #[test]
    fn a_file_stopping_inside_a_box_is_rejected_as_unfinished() {
        let mut reader = BoxReader::new();

        reader.handle_input(b"\0\0\0\x10freeAAAA").unwrap();

        assert_eq!(reader.finish(), Err(Error::unfinished_box(16, 12)));
    }

    #[test]
    fn a_failed_reader_reports_the_same_failure_for_every_call_after_it() {
        let mut reader = BoxReader::new();
        let failure = Error::from(isobmff_core::Error::size_below_header(8, 4));

        assert_eq!(reader.handle_input(b"\0\0\0\x04free"), Err(failure));
        assert_eq!(reader.handle_input(b"\0\0\0\x08skip"), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn a_reader_that_failed_while_finishing_reports_that_failure_again() {
        let mut reader = BoxReader::new();
        let failure = Error::unfinished_box(16, 12);

        reader.handle_input(b"\0\0\0\x10freeAAAA").unwrap();

        assert_eq!(reader.finish(), Err(failure));
        assert_eq!(reader.handle_input(b"AAAA"), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn a_failed_reader_hands_over_the_events_it_had_already_made() {
        let mut reader = BoxReader::new();

        assert!(
            reader
                .handle_input(b"\0\0\0\x08free\0\0\0\x04free")
                .is_err()
        );

        assert_eq!(
            polled(&mut reader),
            Some(started(compact_header(*b"free", 8), 0))
        );
        assert_eq!(polled(&mut reader), Some(ended(8)));
        assert_eq!(polled(&mut reader), None);
    }

    #[test]
    fn the_events_a_finished_file_had_made_are_still_taken_after_it() {
        let mut reader = BoxReader::new();

        reader.handle_input(b"\0\0\0\x08free").unwrap();
        reader.finish().unwrap();

        assert_eq!(
            polled(&mut reader),
            Some(started(compact_header(*b"free", 8), 0))
        );
        assert_eq!(polled(&mut reader), Some(ended(8)));
        assert_eq!(polled(&mut reader), None);
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut reader = BoxReader::new();

        reader.finish().unwrap();

        assert_eq!(
            reader.handle_input(b"\0\0\0\x08free"),
            Err(Error::already_finished())
        );
    }

    #[test]
    fn finishing_a_file_that_is_already_over_is_rejected() {
        let mut reader = BoxReader::new();

        reader.finish().unwrap();

        assert_eq!(reader.finish(), Err(Error::already_finished()));
    }
}
