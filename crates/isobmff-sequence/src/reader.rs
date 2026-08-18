//! [`BoxReader`] and [`BoxEvent`], the sequence of boxes of ISO/IEC 14496-12 §4.2 read as the input arrives

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::error;
use core::fmt;

use isobmff_core::{BoxHeader, BoxHeaderError, CompactType, FourCC};

/// Shortest header a box can carry: the `size` and `type` fields alone
const MIN_HEADER_LEN: usize = 8;

/// Total a `size` field declares when the real total is in the `largesize` field
const EXTENDED_SIZE_MARKER: u32 = 1;

/// Returns the length of the header that starts with the given bytes
///
/// The `size` and `type` fields settle whether a `largesize` and a `usertype`
/// follow, so the bytes they occupy name the length of the whole header: 8, 16,
/// 24, or 32. They name that length and nothing more — whether the bytes it
/// spans are a header at all is settled by [`BoxHeader::decode`].
const fn header_len_from_prefix(prefix: &[u8; MIN_HEADER_LEN]) -> usize {
    let [
        size_first,
        size_second,
        size_third,
        size_fourth,
        type_field @ ..,
    ] = *prefix;

    let declared = u32::from_be_bytes([size_first, size_second, size_third, size_fourth]);
    let has_large_size = declared == EXTENDED_SIZE_MARKER;
    let has_user_type = CompactType::new(FourCC::new(type_field)).is_none();

    match (has_large_size, has_user_type) {
        (false, false) => 8,
        (true, false) => 16,
        (false, true) => 24,
        (true, true) => 32,
    }
}

/// Step of the sequence of boxes, owning the bytes it carries
///
/// A box appears as [`RawStart`](Self::RawStart), then as many
/// [`RawPayload`](Self::RawPayload) events as the input cut its payload into,
/// then [`RawEnd`](Self::RawEnd). A container is reported as one box like any
/// other: its payload is passed on whole rather than descended into.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BoxEvent {
    /// Header of a box, whole, and where in the file that box begins
    RawStart {
        /// Header of the box, however the input cut across it
        header: BoxHeader,
        /// Bytes the file carried before the header of this box
        file_offset: u64,
    },
    /// Part of the payload of the box that started, as it lay in the input
    RawPayload(Vec<u8>),
    /// End of the box that started, its declared total reached
    RawEnd,
}

/// Reads the sequence of boxes a file is formed as, taking the input as it arrives
///
/// The reader is handed the input as it arrives and reports the boxes it frames
/// as owned [`BoxEvent`]s. It reaches for no source of its own: when to read and
/// from where stay with the caller. It reads no box type and holds no policy
/// either — every box is passed on the same way, and which ones matter is the
/// caller's.
///
/// # Contract
///
/// * [`handle_read`](Self::handle_read) takes the input whole and owns what it
///   made of it, so the buffer is the caller's again once the call returns. The
///   events are taken one at a time from [`poll_event`](Self::poll_event),
///   which reports `None` once the input handed over so far is used up.
/// * The caller drains before handing over more input. Events are held until
///   they are taken, so reading on without polling has the reader hold the
///   whole file.
/// * A [`RawPayload`](BoxEvent::RawPayload) is never empty. A box with no
///   payload is a [`RawStart`](BoxEvent::RawStart) followed by a
///   [`RawEnd`](BoxEvent::RawEnd).
/// * Where a payload is cut into [`RawPayload`](BoxEvent::RawPayload) events
///   follows how the caller cut the file. What does not follow it: the
///   [`RawStart`](BoxEvent::RawStart) and [`RawEnd`](BoxEvent::RawEnd) events,
///   the offsets they carry, and the payload bytes those events hold end to
///   end.
/// * An `Err` leaves the reader failed for good: every later
///   [`handle_read`](Self::handle_read) and [`finish`](Self::finish) reports
///   that same error. The events made before it are still there to take, and no
///   further one is ever made.
/// * [`finish`](Self::finish) declares the file over. Events are still taken
///   after it, but input handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](BoxReaderError::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize};
/// use isobmff_sequence::{BoxEvent, BoxReader};
///
/// // One twelve-byte box, arriving cut across both its header and its payload
/// let arriving: [&[u8]; 3] = [b"\0\0\0\x0cfr", b"eeAA", b"AA"];
/// let mut reader = BoxReader::new();
/// let mut events = Vec::new();
///
/// // Input is handed over whole, then what it completed is drained
/// for input in arriving {
///     reader.handle_read(input).unwrap();
///     while let Some(event) = reader.poll_event() {
///         events.push(event);
///     }
/// }
///
/// // The file ended on a box boundary, so nothing was left open
/// reader.finish().unwrap();
/// assert_eq!(reader.poll_event(), None);
///
/// // The header the input cut across is gathered before the box is reported
/// let header = BoxHeader::new(
///     BoxType::compact(*b"free"),
///     BoxSize::Compact(CompactSize::new(12).unwrap()),
/// )
/// .unwrap();
/// assert_eq!(
///     events,
///     [
///         BoxEvent::RawStart {
///             header,
///             file_offset: 0
///         },
///         BoxEvent::RawPayload(b"AA".to_vec()),
///         BoxEvent::RawPayload(b"AA".to_vec()),
///         BoxEvent::RawEnd,
///     ]
/// );
/// ```
#[derive(Debug)]
pub struct BoxReader {
    state: State,
    events: VecDeque<BoxEvent>,
    file_offset: u64,
}

impl BoxReader {
    /// Creates a reader waiting at the start of a file
    ///
    /// The offsets it reports count from the first byte handed to it, which it
    /// takes as offset zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::Header(PartialHeader::new(0)),
            events: VecDeque::new(),
            file_offset: 0,
        }
    }

    /// Takes the input that arrived, and makes the events it completes
    ///
    /// The input is taken whole. What it completed is then taken from
    /// [`poll_event`](Self::poll_event); empty input completes nothing and
    /// leaves the reader where it was.
    ///
    /// # Errors
    ///
    /// * [`Header`](BoxReaderError::Header): a header does not decode, as
    ///   [`BoxHeader::decode`] reports it.
    /// * [`AlreadyFinished`](BoxReaderError::AlreadyFinished): the file was
    ///   declared over by [`finish`](Self::finish).
    /// * The error a previous call already reported, once the reader has failed.
    pub fn handle_read(&mut self, input: &[u8]) -> Result<(), BoxReaderError> {
        let mut unread = input;

        loop {
            match self.state {
                State::Failed(ref error) => return Err(error.clone()),
                State::Finished => return Err(BoxReaderError::AlreadyFinished),
                State::Header(mut partial) => {
                    let available = unread.len();
                    let gathered = partial.take_from(&mut unread);
                    // Why count here rather than under the event: a header the
                    // input cut across takes bytes off every part of it while
                    // completing no event, so counting only what an event took
                    // would leave the offset short by the head of that header.
                    self.advance(available.saturating_sub(unread.len()));

                    match gathered {
                        Ok(Some(header)) => {
                            self.state = State::Payload {
                                header,
                                remaining: header.payload_len(),
                            };
                            self.events.push_back(BoxEvent::RawStart {
                                header,
                                file_offset: partial.began_at,
                            });
                        }
                        Ok(None) => {
                            self.state = State::Header(partial);
                            return Ok(());
                        }
                        Err(error) => {
                            self.state = State::Failed(error.clone());
                            return Err(error);
                        }
                    }
                }
                State::Payload { header, remaining } => {
                    if remaining == Some(0) {
                        self.state = State::Header(PartialHeader::new(self.file_offset));
                        self.events.push_back(BoxEvent::RawEnd);
                        continue;
                    }
                    if unread.is_empty() {
                        return Ok(());
                    }

                    let wanted = remaining
                        .and_then(|remaining| usize::try_from(remaining).ok())
                        .unwrap_or(usize::MAX)
                        .min(unread.len());
                    let (payload, rest) = unread.split_at(wanted);

                    unread = rest;
                    self.state = State::Payload {
                        header,
                        remaining: remaining.map(|remaining| {
                            remaining
                                .saturating_sub(u64::try_from(payload.len()).unwrap_or(u64::MAX))
                        }),
                    };
                    self.advance(payload.len());
                    self.events
                        .push_back(BoxEvent::RawPayload(Vec::from(payload)));
                }
            }
        }
    }

    /// Takes the next event the input handed over so far completed
    ///
    /// Reports `None` once it is used up: more input is needed, or
    /// [`finish`](Self::finish) is. Failure is reported by
    /// [`handle_read`](Self::handle_read) and [`finish`](Self::finish) alone, so
    /// this call never fails — a failed reader hands over the events it had
    /// already made, then `None` from there on.
    pub fn poll_event(&mut self) -> Option<BoxEvent> {
        self.events.pop_front()
    }

    /// Declares the file over, and closes the box left open
    ///
    /// A box is left open either because the file reached its declared total
    /// without the [`RawEnd`](BoxEvent::RawEnd) that follows having been made,
    /// or because it declares no total at all —
    /// [`ToEndOfFile`](isobmff_core::BoxSize::ToEndOfFile), which only the end
    /// of the file ends. Both are closed with a [`RawEnd`](BoxEvent::RawEnd),
    /// taken from [`poll_event`](Self::poll_event) like any other event.
    ///
    /// # Errors
    ///
    /// * [`UnfinishedHeader`](BoxReaderError::UnfinishedHeader): the file ended
    ///   inside a box header.
    /// * [`UnfinishedBox`](BoxReaderError::UnfinishedBox): the file ended before
    ///   the declared total of a box was reached.
    /// * [`AlreadyFinished`](BoxReaderError::AlreadyFinished): the file was
    ///   already declared over.
    /// * The error a previous call already reported, once the reader has failed.
    pub fn finish(&mut self) -> Result<(), BoxReaderError> {
        match self.state {
            State::Failed(ref error) => Err(error.clone()),
            State::Finished => Err(BoxReaderError::AlreadyFinished),
            State::Header(partial) if partial.filled == 0 => {
                self.state = State::Finished;
                Ok(())
            }
            State::Header(partial) => Err(self.fail(BoxReaderError::UnfinishedHeader {
                needed: partial.needed,
                available: partial.filled,
            })),
            State::Payload { header, remaining } => {
                let unfinished = remaining
                    .filter(|remaining| *remaining != 0)
                    .zip(header.size().total_bytes());

                match unfinished {
                    Some((remaining, total)) => Err(self.fail(BoxReaderError::UnfinishedBox {
                        needed: total,
                        available: total.saturating_sub(remaining),
                    })),
                    None => {
                        self.state = State::Finished;
                        self.events.push_back(BoxEvent::RawEnd);
                        Ok(())
                    }
                }
            }
        }
    }

    /// Counts `taken` bytes as read, moving the offset the next box begins at
    fn advance(&mut self, taken: usize) {
        self.file_offset = self
            .file_offset
            .saturating_add(u64::try_from(taken).unwrap_or(u64::MAX));
    }

    /// Fails the reader for good, and hands the failure back to report
    fn fail(&mut self, failure: BoxReaderError) -> BoxReaderError {
        self.state = State::Failed(failure.clone());

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
    /// Gathering the header of the box that starts next
    Header(PartialHeader),
    /// Passing on the payload of the box that started, with `remaining` bytes
    /// of it still to come — `None` while the box declares no total
    Payload {
        header: BoxHeader,
        remaining: Option<u64>,
    },
    /// Told the file is over, and taking no more input
    Finished,
    /// Failed, and reporting the same error from here on
    Failed(BoxReaderError),
}

/// Header bytes gathered so far, for a header the input cut across
///
/// `needed` is the length the header reaches: the shortest header until the
/// bytes gathered name the true one, and never past the longest a box can carry.
/// `began_at` is where in the file the header starts, which the box it
/// introduces is reported at.
#[derive(Clone, Copy, Debug)]
struct PartialHeader {
    bytes: [u8; BoxHeader::MAX_ENCODED_LEN],
    filled: usize,
    needed: usize,
    began_at: u64,
}

impl PartialHeader {
    const fn new(began_at: u64) -> Self {
        Self {
            bytes: [0; BoxHeader::MAX_ENCODED_LEN],
            filled: 0,
            needed: MIN_HEADER_LEN,
            began_at,
        }
    }

    /// Takes header bytes off `input`, and decodes the header once it is whole
    ///
    /// Returns `Ok(None)` when `input` runs out first, having taken what it
    /// offered.
    fn take_from(&mut self, input: &mut &[u8]) -> Result<Option<BoxHeader>, BoxReaderError> {
        self.gather(input);
        if self.filled < MIN_HEADER_LEN {
            return Ok(None);
        }
        // Why not unreachable: the buffer is the longest header a box can carry,
        // so a prefix of the shortest always splits off, and the fallback is a
        // degenerate value in place of a panic the lints forbid.
        let Some(prefix) = self.bytes.first_chunk() else {
            return Ok(None);
        };

        self.needed = header_len_from_prefix(prefix);
        self.gather(input);
        if self.filled < self.needed {
            return Ok(None);
        }
        // Why not unreachable: `gather` takes no more than `needed`, which is a
        // header length and so never past the buffer, for the same reason.
        let Some(gathered) = self.bytes.get(..self.filled) else {
            return Ok(None);
        };

        // Why nest rather than re-declare the one failure that reaches here:
        // `BoxHeaderError` is `non_exhaustive` and comes from another crate, so
        // a match on it cannot be complete, and the wildcard arm that closes it
        // would have to name some other failure in place of the one that was
        // actually reported.
        match BoxHeader::decode(gathered) {
            Ok((header, _nothing_beyond)) => Ok(Some(header)),
            Err(error) => Err(BoxReaderError::Header(error)),
        }
    }

    /// Takes off `input` as many bytes as the length being reached for is short
    fn gather(&mut self, input: &mut &[u8]) {
        let wanted = self.needed.saturating_sub(self.filled).min(input.len());
        let filled = self.filled.saturating_add(wanted);

        let Some(slot) = self.bytes.get_mut(self.filled..filled) else {
            return;
        };
        let (taken, rest) = input.split_at(wanted);

        slot.copy_from_slice(taken);
        self.filled = filled;
        *input = rest;
    }
}

/// Reason input does not read as a sequence of boxes
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BoxReaderError {
    /// Header of a box does not decode
    Header(BoxHeaderError),
    /// File ended inside a box header
    UnfinishedHeader {
        /// Bytes the header occupies, as far as the bytes read so far tell
        needed: usize,
        /// Bytes of the header the file carried
        available: usize,
    },
    /// File ended before the declared total of a box was reached
    UnfinishedBox {
        /// Bytes the box occupies, as the `size` or `largesize` field declares
        needed: u64,
        /// Bytes of the box the file carried, header included
        available: u64,
    },
    /// File was declared over, and takes no more input
    AlreadyFinished,
}

impl fmt::Display for BoxReaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Header(_) => formatter.write_str("box header does not decode"),
            Self::UnfinishedHeader { needed, available } => write!(
                formatter,
                "input ended {available} bytes into a box header of {needed}"
            ),
            Self::UnfinishedBox { needed, available } => write!(
                formatter,
                "input ended {available} bytes into a box of {needed}"
            ),
            Self::AlreadyFinished => {
                formatter.write_str("file was declared over and takes no more input")
            }
        }
    }
}

impl From<BoxHeaderError> for BoxReaderError {
    fn from(error: BoxHeaderError) -> Self {
        Self::Header(error)
    }
}

impl error::Error for BoxReaderError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            Self::Header(ref error) => Some(error),
            Self::UnfinishedHeader { .. } | Self::UnfinishedBox { .. } | Self::AlreadyFinished => {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use alloc::vec;
    use alloc::vec::Vec;
    use core::error::Error as _;

    use isobmff_core::{
        BoxHeader, BoxHeaderError, BoxSize, BoxType, CompactSize, ExtendedSize, Uuid,
    };

    use super::{BoxEvent, BoxReader, BoxReaderError, header_len_from_prefix};

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

    /// The box that begins at `file_offset`, as the reader reports it
    fn started(header: BoxHeader, file_offset: u64) -> BoxEvent {
        BoxEvent::RawStart {
            header,
            file_offset,
        }
    }

    /// Every event a reader reports for `input`, handed over `cut_length` bytes at a time
    fn events_of(input: &[u8], cut_length: usize) -> Vec<BoxEvent> {
        let mut reader = BoxReader::new();
        let mut events = Vec::new();

        for arriving in input.chunks(cut_length) {
            reader.handle_read(arriving).unwrap();
            while let Some(event) = reader.poll_event() {
                events.push(event);
            }
        }
        reader.finish().unwrap();
        while let Some(event) = reader.poll_event() {
            events.push(event);
        }

        events
    }

    #[test]
    fn the_length_a_prefix_names_is_the_bytes_the_header_decodes_from() {
        for encoded in EVERY_HEADER_FORM {
            let (_header, after_header) = BoxHeader::decode(encoded).unwrap();
            let prefix = encoded.first_chunk().unwrap();

            assert_eq!(
                header_len_from_prefix(prefix),
                encoded.len().checked_sub(after_header.len()).unwrap(),
                "{encoded:02x?}"
            );
        }
    }

    #[test]
    fn a_box_arriving_whole_is_reported_as_a_start_a_payload_and_an_end() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA", 12),
            vec![
                started(compact_header(*b"free", 12), 0),
                BoxEvent::RawPayload(Vec::from(*b"AAAA")),
                BoxEvent::RawEnd,
            ]
        );
    }

    #[test]
    fn a_box_with_no_payload_is_reported_as_a_start_and_an_end() {
        assert_eq!(
            events_of(b"\0\0\0\x08skip", 8),
            vec![started(compact_header(*b"skip", 8), 0), BoxEvent::RawEnd]
        );
    }

    #[test]
    fn a_payload_is_passed_on_as_the_input_cut_it() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA", 5),
            vec![
                started(compact_header(*b"free", 12), 0),
                BoxEvent::RawPayload(Vec::from(*b"AA")),
                BoxEvent::RawPayload(Vec::from(*b"AA")),
                BoxEvent::RawEnd,
            ]
        );
    }

    #[test]
    fn boxes_laid_end_to_end_carry_the_offset_each_one_begins_at() {
        assert_eq!(
            events_of(b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip", 5),
            vec![
                started(compact_header(*b"free", 12), 0),
                BoxEvent::RawPayload(Vec::from(*b"AA")),
                BoxEvent::RawPayload(Vec::from(*b"AA")),
                BoxEvent::RawEnd,
                started(compact_header(*b"skip", 8), 12),
                BoxEvent::RawEnd,
            ]
        );
    }

    #[test]
    fn a_box_behind_a_long_header_carries_the_offset_that_header_begins_at() {
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
                BoxEvent::RawPayload(Vec::from(*b"!")),
                BoxEvent::RawEnd,
                started(compact_header(*b"skip", 8), 33),
                BoxEvent::RawEnd,
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
                BoxEvent::RawPayload(Vec::from(*b"PAYL")),
                BoxEvent::RawPayload(Vec::from(*b"OAD")),
                BoxEvent::RawEnd,
            ]
        );
    }

    #[test]
    fn empty_input_completes_nothing_and_leaves_the_reader_where_it_was() {
        let mut reader = BoxReader::new();

        reader.handle_read(b"\0\0\0\x08fr").unwrap();
        reader.handle_read(b"").unwrap();

        assert_eq!(reader.poll_event(), None);

        reader.handle_read(b"ee").unwrap();

        assert_eq!(
            reader.poll_event(),
            Some(started(compact_header(*b"free", 8), 0))
        );
    }

    #[test]
    fn empty_input_arriving_inside_a_payload_asks_for_more() {
        let mut reader = BoxReader::new();

        reader.handle_read(b"\0\0\0\x0cfreeAA").unwrap();
        while reader.poll_event().is_some() {}
        reader.handle_read(b"").unwrap();

        assert_eq!(reader.poll_event(), None);

        reader.handle_read(b"AA").unwrap();

        assert_eq!(
            reader.poll_event(),
            Some(BoxEvent::RawPayload(Vec::from(*b"AA")))
        );
    }

    #[test]
    fn a_file_stopping_inside_a_header_is_rejected_as_unfinished() {
        let mut reader = BoxReader::new();

        reader
            .handle_read(&[0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00])
            .unwrap();

        assert_eq!(
            reader.finish(),
            Err(BoxReaderError::UnfinishedHeader {
                needed: 16,
                available: 9
            })
        );
    }

    #[test]
    fn a_file_stopping_inside_a_box_is_rejected_as_unfinished() {
        let mut reader = BoxReader::new();

        reader.handle_read(b"\0\0\0\x10freeAAAA").unwrap();

        assert_eq!(
            reader.finish(),
            Err(BoxReaderError::UnfinishedBox {
                needed: 16,
                available: 12
            })
        );
    }

    #[test]
    fn a_failed_reader_reports_the_same_error_from_every_later_call() {
        let mut reader = BoxReader::new();
        let failure = Err(BoxReaderError::Header(BoxHeaderError::SizeBelowHeader {
            declared: 4,
            header_length: 8,
        }));

        assert_eq!(reader.handle_read(b"\0\0\0\x04free"), failure);
        assert_eq!(reader.handle_read(b"\0\0\0\x08skip"), failure);
        assert_eq!(reader.finish(), failure);
    }

    #[test]
    fn a_reader_that_failed_while_finishing_reports_that_error_from_every_later_call() {
        let mut reader = BoxReader::new();
        let failure = Err(BoxReaderError::UnfinishedBox {
            needed: 16,
            available: 12,
        });

        reader.handle_read(b"\0\0\0\x10freeAAAA").unwrap();

        assert_eq!(reader.finish(), failure);
        assert_eq!(reader.handle_read(b"AAAA"), failure);
        assert_eq!(reader.finish(), failure);
    }

    #[test]
    fn a_failed_reader_hands_over_the_events_it_had_already_made() {
        let mut reader = BoxReader::new();

        assert!(reader.handle_read(b"\0\0\0\x08free\0\0\0\x04free").is_err());

        assert_eq!(
            reader.poll_event(),
            Some(started(compact_header(*b"free", 8), 0))
        );
        assert_eq!(reader.poll_event(), Some(BoxEvent::RawEnd));
        assert_eq!(reader.poll_event(), None);
    }

    #[test]
    fn the_events_a_finished_file_had_made_are_still_taken_after_it() {
        let mut reader = BoxReader::new();

        reader.handle_read(b"\0\0\0\x08free").unwrap();
        reader.finish().unwrap();

        assert_eq!(
            reader.poll_event(),
            Some(started(compact_header(*b"free", 8), 0))
        );
        assert_eq!(reader.poll_event(), Some(BoxEvent::RawEnd));
        assert_eq!(reader.poll_event(), None);
    }

    #[test]
    fn input_handed_over_after_finishing_is_rejected() {
        let mut reader = BoxReader::new();

        reader.finish().unwrap();

        assert_eq!(
            reader.handle_read(b"\0\0\0\x08free"),
            Err(BoxReaderError::AlreadyFinished)
        );
    }

    #[test]
    fn finishing_a_file_that_is_already_over_is_rejected() {
        let mut reader = BoxReader::new();

        reader.finish().unwrap();

        assert_eq!(reader.finish(), Err(BoxReaderError::AlreadyFinished));
    }

    #[test]
    fn display_of_an_unfinished_header_names_both_lengths() {
        let error = BoxReaderError::UnfinishedHeader {
            needed: 16,
            available: 9,
        };

        assert_eq!(
            error.to_string(),
            "input ended 9 bytes into a box header of 16"
        );
    }

    #[test]
    fn display_of_an_unfinished_box_names_both_lengths() {
        let error = BoxReaderError::UnfinishedBox {
            needed: 16,
            available: 12,
        };

        assert_eq!(error.to_string(), "input ended 12 bytes into a box of 16");
    }

    #[test]
    fn display_of_a_header_that_does_not_decode_leaves_the_reason_to_its_source() {
        let error = BoxReaderError::Header(BoxHeaderError::SizeBelowHeader {
            declared: 4,
            header_length: 8,
        });

        assert_eq!(error.to_string(), "box header does not decode");
        assert_eq!(
            error.source().map(ToString::to_string),
            Some(String::from(
                "box declares a total of 4 bytes, below its 8-byte header"
            ))
        );
    }
}
