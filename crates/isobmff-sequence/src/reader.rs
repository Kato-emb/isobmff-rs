//! [`BoxReader`], the sequence of boxes of ISO/IEC 14496-12 §4.2 read as the input arrives

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::error;
use core::fmt;
use core::mem;

use isobmff_core::{BoxHeader, BoxHeaderError, BoxType, CompactType, DecodeError, FourCC};

use crate::event::{BoxEvent, ValueBox};

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

/// Reads the sequence of boxes a file is formed as, taking the input as it arrives
///
/// The reader is handed the input as it arrives and reports the boxes it frames
/// as owned [`BoxEvent`]s. It reaches for no source of its own: when to read and
/// from where stay with the caller.
///
/// The boxes a file is framed by — `ftyp`, `styp`, `moov`, and `moof` — are read
/// into values. Every other box is passed on as it lies, an `mdat` and a box no
/// specification this crate reads names alike, and which of those matter is the
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
/// * The boxes read into values are reported as
///   [`FileType`](BoxEvent::FileType), [`SegmentType`](BoxEvent::SegmentType),
///   [`Movie`](BoxEvent::Movie), and [`MovieFragment`](BoxEvent::MovieFragment),
///   and never as the raw events.
/// * A box read into a value makes no event until it is whole: nothing of it is
///   reported while its payload arrives, and the payload of a box that never
///   completed is dropped rather than reported in part. The events the boxes
///   before it made are taken as ever.
/// * A box declaring no total —
///   [`ToEndOfFile`](isobmff_core::BoxSize::ToEndOfFile) — is passed on as it
///   lies even where its type is one that reads into a value, since the limit on
///   what may be gathered is checked against the payload length a box declares.
///   See [`with_payload_limit`](Self::with_payload_limit).
/// * Where a payload is cut into [`RawPayload`](BoxEvent::RawPayload) events
///   follows how the caller cut the file. What does not follow it: the
///   [`RawStart`](BoxEvent::RawStart) and [`RawEnd`](BoxEvent::RawEnd) events,
///   the offsets they carry, and the payload bytes those events hold end to
///   end.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](BoxReaderError::AlreadyFinished) aside: every later
///   [`handle_read`](Self::handle_read) and [`finish`](Self::finish) reports
///   [`AlreadyFailed`](BoxReaderError::AlreadyFailed). The events made before it
///   are still there to take, and no further one is ever made.
/// * [`finish`](Self::finish) declares the file over. Events are still taken
///   after it, but input handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](BoxReaderError::AlreadyFinished). A file being over is
///   not a failure, so that is what every later call reports as well.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::FileTypeBox;
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize, FourCC};
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
/// // The `ftyp` is gathered across the cuts and read into a value, while the
/// // `mdat` is passed on as it lies
/// let mdat = BoxHeader::new(
///     BoxType::compact(*b"mdat"),
///     BoxSize::Compact(CompactSize::new(12).unwrap()),
/// )
/// .unwrap();
/// assert_eq!(
///     events,
///     [
///         BoxEvent::FileType {
///             ftyp: FileTypeBox::new(FourCC::new(*b"iso6"), 512, vec![FourCC::new(*b"iso6")]),
///             file_offset: 0
///         },
///         BoxEvent::RawStart {
///             header: mdat,
///             file_offset: 20
///         },
///         BoxEvent::RawPayload(b"SAMP".to_vec()),
///         BoxEvent::RawEnd,
///     ]
/// );
/// ```
#[derive(Debug)]
pub struct BoxReader {
    state: State,
    events: VecDeque<BoxEvent>,
    file_offset: u64,
    payload_limit: u64,
}

impl BoxReader {
    /// Payload a box read into a value may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading files whose `moov` reaches past that —
    /// a progressive presentation holds a table entry per sample — names a limit
    /// of its own with [`with_payload_limit`](Self::with_payload_limit).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a reader waiting at the start of a file
    ///
    /// What a box read into a value may declare is bounded by
    /// [`DEFAULT_PAYLOAD_LIMIT`](Self::DEFAULT_PAYLOAD_LIMIT). The offsets the
    /// reader reports count from the first byte handed to it, which it takes as
    /// offset zero.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_payload_limit(Self::DEFAULT_PAYLOAD_LIMIT)
    }

    /// Creates a reader gathering no more than `payload_limit` bytes for one box
    ///
    /// A box read into a value is gathered whole before it is read, so the
    /// payload it declares is memory the reader is about to take. One declaring
    /// more than `payload_limit` bytes of payload is
    /// [`PayloadLimitExceeded`](BoxReaderError::PayloadLimitExceeded) instead,
    /// reported before a byte of it is gathered.
    ///
    /// The limit bounds one box rather than the file: it is checked against the
    /// payload length a box declares, not against its total and not against what
    /// the boxes before it took. A box passed on as it lies is not bounded by it
    /// at all — an `mdat` of any length reads as ever, its payload handed on
    /// rather than held.
    #[must_use]
    pub const fn with_payload_limit(payload_limit: u64) -> Self {
        Self {
            state: State::Header(PartialHeader::new(0)),
            events: VecDeque::new(),
            file_offset: 0,
            payload_limit,
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
    /// * [`PayloadLimitExceeded`](BoxReaderError::PayloadLimitExceeded): a box
    ///   read into a value declares a payload past the limit the reader was
    ///   given.
    /// * [`Decode`](BoxReaderError::Decode): the payload of a box read into a
    ///   value does not decode.
    /// * [`AlreadyFinished`](BoxReaderError::AlreadyFinished): the file was
    ///   declared over by [`finish`](Self::finish).
    /// * [`AlreadyFailed`](BoxReaderError::AlreadyFailed): a previous call
    ///   failed, and the reader takes no more input.
    pub fn handle_read(&mut self, input: &[u8]) -> Result<(), BoxReaderError> {
        let mut unread = input;

        loop {
            match self.state {
                State::Failed => return Err(BoxReaderError::AlreadyFailed),
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
                            match ValueBox::of(header.box_type()).zip(header.payload_len()) {
                                Some((value_box, declared)) => {
                                    if declared > self.payload_limit {
                                        return Err(self.fail(
                                            BoxReaderError::PayloadLimitExceeded {
                                                box_type: header.box_type(),
                                                declared,
                                                limit: self.payload_limit,
                                            },
                                        ));
                                    }

                                    self.state = State::Gathering {
                                        header,
                                        value_box,
                                        // Why not reserve the declared length:
                                        // the file declares it and the limit
                                        // only bounds it, so reserving would
                                        // take memory for bytes that may never
                                        // arrive.
                                        gathered: Vec::new(),
                                        remaining: declared,
                                        began_at: partial.began_at,
                                    };
                                }
                                None => {
                                    self.state = State::Payload {
                                        header,
                                        remaining: header.payload_len(),
                                    };
                                    self.events.push_back(BoxEvent::RawStart {
                                        header,
                                        file_offset: partial.began_at,
                                    });
                                }
                            }
                        }
                        Ok(None) => {
                            self.state = State::Header(partial);
                            return Ok(());
                        }
                        Err(error) => return Err(self.fail(error)),
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
                State::Gathering {
                    header,
                    value_box,
                    began_at,
                    ref mut remaining,
                    ref mut gathered,
                } => {
                    if *remaining == 0 {
                        let payload = mem::take(gathered);

                        self.state = State::Header(PartialHeader::new(self.file_offset));
                        match value_box.read(&payload, began_at) {
                            Ok(event) => self.events.push_back(event),
                            Err(error) => {
                                return Err(self.fail(BoxReaderError::Decode {
                                    box_type: header.box_type(),
                                    source: error,
                                }));
                            }
                        }
                        continue;
                    }
                    if unread.is_empty() {
                        return Ok(());
                    }

                    let wanted = usize::try_from(*remaining)
                        .unwrap_or(usize::MAX)
                        .min(unread.len());
                    let (payload, rest) = unread.split_at(wanted);

                    gathered.extend_from_slice(payload);
                    *remaining =
                        remaining.saturating_sub(u64::try_from(payload.len()).unwrap_or(u64::MAX));
                    unread = rest;
                    self.advance(payload.len());
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
    ///   the declared total of a box was reached, whether that box was being
    ///   passed on or gathered into a value.
    /// * [`AlreadyFinished`](BoxReaderError::AlreadyFinished): the file was
    ///   already declared over.
    /// * [`AlreadyFailed`](BoxReaderError::AlreadyFailed): a previous call
    ///   failed, and the reader takes no more input.
    pub fn finish(&mut self) -> Result<(), BoxReaderError> {
        match self.state {
            State::Failed => Err(BoxReaderError::AlreadyFailed),
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
            State::Gathering {
                header,
                remaining,
                ref gathered,
                ..
            } => {
                let read_so_far = u64::try_from(header.encoded_len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(u64::try_from(gathered.len()).unwrap_or(u64::MAX));

                Err(self.fail(BoxReaderError::UnfinishedBox {
                    needed: read_so_far.saturating_add(remaining),
                    available: read_so_far,
                }))
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
        self.state = State::Failed;

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
    /// Gathering the payload of a box that reads into a value, with `remaining`
    /// bytes of it still to come
    Gathering {
        header: BoxHeader,
        value_box: ValueBox,
        gathered: Vec<u8>,
        remaining: u64,
        began_at: u64,
    },
    /// Told the file is over, and taking no more input
    Finished,
    /// Failed, and taking no more input
    Failed,
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
#[derive(Debug)]
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
    /// Box read into a value declares a payload past the limit the reader holds
    PayloadLimitExceeded {
        /// Box type of the box that declared it
        box_type: BoxType,
        /// Bytes of payload the box declares, its header not counted
        declared: u64,
        /// Bytes of payload the reader gathers for one box at most
        limit: u64,
    },
    /// Payload of a box read into a value does not decode
    Decode {
        /// Box type of the box that failed
        box_type: BoxType,
        /// Failure the box reported
        source: DecodeError,
    },
    /// File was declared over, and takes no more input
    AlreadyFinished,
    /// Reader failed, and takes no more input
    AlreadyFailed,
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
            Self::PayloadLimitExceeded {
                box_type,
                declared,
                limit,
            } => write!(
                formatter,
                "{box_type} box declares a payload of {declared} bytes, past the {limit}-byte limit"
            ),
            Self::Decode { box_type, .. } => write!(formatter, "{box_type} box does not decode"),
            Self::AlreadyFinished => {
                formatter.write_str("file was declared over and takes no more input")
            }
            Self::AlreadyFailed => formatter.write_str("reader failed and takes no more input"),
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
            Self::Decode { ref source, .. } => Some(source),
            Self::UnfinishedHeader { .. }
            | Self::UnfinishedBox { .. }
            | Self::PayloadLimitExceeded { .. }
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

    use isobmff_boxes::{FileTypeBox, MovieFragmentBox, MovieFragmentHeaderBox, SegmentTypeBox};
    use isobmff_core::{
        BoxHeader, BoxHeaderError, BoxSize, BoxType, BoxWrite, CompactSize, ExtendedSize, FourCC,
        Uuid,
    };

    use super::{BoxEvent, BoxReader, BoxReaderError, DecodeError, header_len_from_prefix};

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

    /// Brands a fragmented file declares itself readable as
    fn file_type() -> FileTypeBox {
        FileTypeBox::new(FourCC::new(*b"iso6"), 512, vec![FourCC::new(*b"iso6")])
    }

    /// Brands a segment of a fragmented file declares itself readable as
    fn segment_type() -> SegmentTypeBox {
        SegmentTypeBox::new(FourCC::new(*b"msdh"), 0, vec![FourCC::new(*b"msdh")])
    }

    /// Fragment adding to no track, the shortest `moof` a file can carry
    fn movie_fragment() -> MovieFragmentBox {
        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), Vec::new())
    }

    /// The bytes a box occupies, its header and its payload
    fn written(value: &impl BoxWrite) -> Vec<u8> {
        let mut bytes = vec![0; usize::try_from(value.encoded_len()).unwrap()];
        value.encode(&mut bytes).unwrap();

        bytes
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

        assert!(matches!(
            reader.finish(),
            Err(BoxReaderError::UnfinishedHeader {
                needed: 16,
                available: 9
            })
        ));
    }

    #[test]
    fn a_file_stopping_inside_a_box_is_rejected_as_unfinished() {
        let mut reader = BoxReader::new();

        reader.handle_read(b"\0\0\0\x10freeAAAA").unwrap();

        assert!(matches!(
            reader.finish(),
            Err(BoxReaderError::UnfinishedBox {
                needed: 16,
                available: 12
            })
        ));
    }

    #[test]
    fn a_failed_reader_takes_no_more_input() {
        let mut reader = BoxReader::new();

        assert!(matches!(
            reader.handle_read(b"\0\0\0\x04free"),
            Err(BoxReaderError::Header(BoxHeaderError::SizeBelowHeader {
                declared: 4,
                header_length: 8
            }))
        ));
        assert!(matches!(
            reader.handle_read(b"\0\0\0\x08skip"),
            Err(BoxReaderError::AlreadyFailed)
        ));
        assert!(matches!(
            reader.finish(),
            Err(BoxReaderError::AlreadyFailed)
        ));
    }

    #[test]
    fn a_reader_that_failed_while_finishing_takes_no_more_input() {
        let mut reader = BoxReader::new();

        reader.handle_read(b"\0\0\0\x10freeAAAA").unwrap();

        assert!(matches!(
            reader.finish(),
            Err(BoxReaderError::UnfinishedBox {
                needed: 16,
                available: 12
            })
        ));
        assert!(matches!(
            reader.handle_read(b"AAAA"),
            Err(BoxReaderError::AlreadyFailed)
        ));
        assert!(matches!(
            reader.finish(),
            Err(BoxReaderError::AlreadyFailed)
        ));
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

        assert!(matches!(
            reader.handle_read(b"\0\0\0\x08free"),
            Err(BoxReaderError::AlreadyFinished)
        ));
    }

    #[test]
    fn finishing_a_file_that_is_already_over_is_rejected() {
        let mut reader = BoxReader::new();

        reader.finish().unwrap();

        assert!(matches!(
            reader.finish(),
            Err(BoxReaderError::AlreadyFinished)
        ));
    }

    #[test]
    fn a_box_that_reads_into_a_value_is_reported_as_that_value() {
        let file = [written(&file_type()), written(&movie_fragment())].concat();

        assert_eq!(
            events_of(&file, file.len()),
            vec![
                BoxEvent::FileType {
                    ftyp: file_type(),
                    file_offset: 0
                },
                BoxEvent::MovieFragment {
                    moof: movie_fragment(),
                    file_offset: 20
                },
            ]
        );
    }

    #[test]
    fn a_segment_declares_its_brands_in_a_value_of_its_own() {
        assert_eq!(
            events_of(&written(&segment_type()), 3),
            vec![BoxEvent::SegmentType {
                styp: segment_type(),
                file_offset: 0
            }]
        );
    }

    #[test]
    fn a_value_is_the_same_however_the_input_cut_the_payload_it_was_gathered_from() {
        let file = [written(&file_type()), written(&movie_fragment())].concat();
        let whole = events_of(&file, file.len());

        for cut_length in 1..=file.len() {
            assert_eq!(
                events_of(&file, cut_length),
                whole,
                "cut every {cut_length} bytes"
            );
        }
    }

    #[test]
    fn no_event_is_made_while_the_payload_of_a_value_arrives() {
        let ftyp = written(&file_type());
        let (last, head) = ftyp.split_last().unwrap();
        let mut reader = BoxReader::new();

        reader.handle_read(head).unwrap();

        assert_eq!(reader.poll_event(), None);

        reader.handle_read(&[*last]).unwrap();

        assert_eq!(
            reader.poll_event(),
            Some(BoxEvent::FileType {
                ftyp: file_type(),
                file_offset: 0
            })
        );
    }

    #[test]
    fn a_value_declaring_a_payload_past_the_limit_is_rejected() {
        let mut reader = BoxReader::with_payload_limit(4);

        assert!(matches!(
            reader.handle_read(&written(&file_type())),
            Err(BoxReaderError::PayloadLimitExceeded {
                box_type,
                declared: 12,
                limit: 4
            }) if box_type == BoxType::compact(*b"ftyp")
        ));
    }

    #[test]
    fn a_box_passed_on_as_it_lies_is_not_bounded_by_the_limit() {
        let mut reader = BoxReader::with_payload_limit(0);
        let mut events = Vec::new();

        reader.handle_read(b"\0\0\0\x10mdatPAYLOAD!").unwrap();
        reader.finish().unwrap();
        while let Some(event) = reader.poll_event() {
            events.push(event);
        }

        assert_eq!(
            events,
            vec![
                started(compact_header(*b"mdat", 16), 0),
                BoxEvent::RawPayload(Vec::from(*b"PAYLOAD!")),
                BoxEvent::RawEnd,
            ]
        );
    }

    #[test]
    fn a_box_declaring_no_total_is_passed_on_as_it_lies_though_its_type_reads_into_a_value() {
        assert_eq!(
            events_of(b"\0\0\0\0moovPAYLOAD", 4),
            vec![
                started(
                    BoxHeader::new(BoxType::compact(*b"moov"), BoxSize::ToEndOfFile).unwrap(),
                    0
                ),
                BoxEvent::RawPayload(Vec::from(*b"PAYL")),
                BoxEvent::RawPayload(Vec::from(*b"OAD")),
                BoxEvent::RawEnd,
            ]
        );
    }

    #[test]
    fn a_value_whose_payload_does_not_decode_fails_the_reader_and_reports_no_part_of_it() {
        let mut reader = BoxReader::new();

        assert!(matches!(
            reader.handle_read(b"\0\0\0\x0cmoofAAAA"),
            Err(BoxReaderError::Decode { box_type, .. }) if box_type == BoxType::compact(*b"moof")
        ));
        assert!(matches!(
            reader.handle_read(b"\0\0\0\x08free"),
            Err(BoxReaderError::AlreadyFailed)
        ));
        assert_eq!(reader.poll_event(), None);
    }

    #[test]
    fn a_file_stopping_inside_a_value_is_rejected_as_unfinished() {
        let ftyp = written(&file_type());
        let (_last, head) = ftyp.split_last().unwrap();
        let mut reader = BoxReader::new();

        reader.handle_read(head).unwrap();

        assert!(matches!(
            reader.finish(),
            Err(BoxReaderError::UnfinishedBox {
                needed: 20,
                available: 19
            })
        ));
        assert_eq!(reader.poll_event(), None);
    }

    #[test]
    fn display_of_a_payload_past_the_limit_names_both_lengths() {
        let error = BoxReaderError::PayloadLimitExceeded {
            box_type: BoxType::compact(*b"moov"),
            declared: 32,
            limit: 16,
        };

        assert_eq!(
            error.to_string(),
            "moov box declares a payload of 32 bytes, past the 16-byte limit"
        );
    }

    #[test]
    fn display_of_a_value_that_does_not_decode_leaves_the_reason_to_its_source() {
        let error = BoxReaderError::Decode {
            box_type: BoxType::compact(*b"moof"),
            source: DecodeError::MissingMandatoryBox(BoxType::compact(*b"mfhd")),
        };

        assert_eq!(error.to_string(), "moof box does not decode");
        assert_eq!(
            error.source().map(ToString::to_string),
            Some(String::from("container holds no mandatory mfhd box"))
        );
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
