//! [`BoxReader`], which frames the boxes of a file out of bytes that arrive in pieces

use alloc::vec::Vec;

use isobmff_core::{BoxHeader, ErrorKind, RawBox};

use crate::byte_range::ByteRange;
use crate::error::DemuxError;

mod box_event;

pub use box_event::{BoxEvent, BoxEvents, OpenBox};

/// Length of the `size` and `boxtype` fields every header opens with
const SHORTEST_HEADER: u64 = 8;

/// Reader that frames the boxes laid end to end in a file, ISO/IEC 14496-12 §4.2.2
///
/// The reader does no I/O. It names the bytes it wants next as
/// [`wanted_input`](Self::wanted_input), takes them with the file offset they
/// start at in [`handle_input`](Self::handle_input), and reports what they
/// hold as [`BoxEvent`]s. Bytes may arrive cut anywhere, a byte at a time if
/// need be.
///
/// Each box opens with [`BoxEvent::Header`], whose [`OpenBox`] the caller
/// answers with what to do with the payload: [`gather`](OpenBox::gather) it
/// into one [`RawBox`], [`stream`](OpenBox::stream) it as the pieces of input
/// it arrives in, or [`skip`](OpenBox::skip) it. The reader takes no more
/// input until it has the answer. Each box closes with [`BoxEvent::End`].
///
/// A box whose size runs to the end of the file is closed by
/// [`finish`](Self::finish), which the caller calls once the file has no
/// more bytes.
///
/// # Examples
///
/// A caller that can seek reads wherever the reader wants, and never reads
/// the payload of a box it skips.
///
/// ```
/// use isobmff::framing::{BoxEvent, BoxReader};
/// use isobmff::{BoxType, DemuxError};
///
/// // `ftyp`, `free`, and an `mdat` whose size runs to the end of the file
/// let file = b"\0\0\0\x10ftypisom\0\0\0\0\0\0\0\x0cfreeAAAA\0\0\0\0mdatsamples";
/// let mut reader = BoxReader::new();
/// let (mut brands, mut media_data) = (Vec::new(), Vec::new());
///
/// // Seven bytes at a time, from wherever the reader wants them
/// while let Some(wanted) = reader.wanted_input() {
///     let rest = file.get(usize::try_from(wanted.offset()).unwrap()..).unwrap_or_default();
///     let input = &rest[..rest.len().min(7)];
///     if input.is_empty() {
///         break;
///     }
///     for event in reader.handle_input(wanted.offset(), input)? {
///         match event? {
///             BoxEvent::Header(open) if open.header().box_type() == BoxType::compact(*b"ftyp") => {
///                 open.gather(1024)?
///             }
///             BoxEvent::Header(open) if open.header().box_type() == BoxType::compact(*b"mdat") => {
///                 open.stream()
///             }
///             BoxEvent::Header(open) => open.skip(),
///             BoxEvent::Gathered(ftyp) => brands.extend_from_slice(ftyp.payload()),
///             BoxEvent::Payload(bytes) => media_data.extend_from_slice(bytes),
///             _ => {}
///         }
///     }
/// }
///
/// // The file has no more bytes, which ends the `mdat`
/// for event in reader.finish()? {
///     assert!(matches!(event?, BoxEvent::End));
/// }
/// assert_eq!(brands, b"isom\0\0\0\0");
/// assert_eq!(media_data, b"samples");
/// # Ok::<(), DemuxError>(())
/// ```
#[derive(Clone, Debug)]
pub struct BoxReader {
    /// File offset of the next byte the reader takes
    position: u64,
    progress: Progress,
    /// Header of the box being read, followed by its payload when it is gathered
    gathered: Vec<u8>,
    /// Whether [`finish`](Self::finish) has ended the file
    at_end_of_file: bool,
}

/// How far the reader is through the box it reads
#[derive(Clone, Copy, Debug)]
enum Progress {
    /// Reading a header that is `needed` bytes long as far as it is known, `started` once a byte of it is read
    Header {
        needed: u64,
        started: bool,
    },
    /// Header read, waiting for the caller to answer the [`OpenBox`]
    Opened {
        header: BoxHeader,
    },
    /// Payload read as `answer` says, up to the file offset `end` or, for `None`, the end of the file
    Payload {
        header: BoxHeader,
        end: Option<u64>,
        answer: Answer,
    },
    Failed(DemuxError),
}

/// What the caller answered an [`OpenBox`] with
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Answer {
    Gather { limit_bytes: u64 },
    Stream,
    Skip,
}

impl BoxReader {
    /// Creates a reader that wants the file from its first byte
    #[must_use]
    pub const fn new() -> Self {
        Self {
            position: 0,
            progress: Progress::Header {
                needed: SHORTEST_HEADER,
                started: false,
            },
            gathered: Vec::new(),
            at_end_of_file: false,
        }
    }

    /// Returns the bytes the reader wants next, or `None` once it wants no more
    ///
    /// The length is that of the payload left to stream or gather. It is
    /// `None` where the reader cannot tell how far it reads before it next
    /// stops: inside a header, or in a box that runs to the end of the file.
    /// Input may run past the length; the reader takes what follows as the
    /// next box.
    ///
    /// The reader wants no more once the file has ended or the reader has
    /// failed, and once it skips a box that runs to the end of the file.
    #[must_use]
    pub fn wanted_input(&self) -> Option<ByteRange> {
        if self.at_end_of_file {
            return None;
        }
        let length = match self.progress {
            Progress::Header { .. } => None,
            Progress::Opened { header } => header.payload_len(),
            Progress::Payload {
                end,
                answer: Answer::Skip,
                ..
            } => return end.map(|end| ByteRange::new(end, None)),
            Progress::Payload { end, .. } => self.remaining(end).filter(|&remaining| remaining > 0),
            Progress::Failed(_) => return None,
        };
        Some(ByteRange::new(self.position, length))
    }

    /// Takes `input`, the bytes of the file from file offset `offset`, and returns what they hold
    ///
    /// `offset` is that of [`wanted_input`](Self::wanted_input). A caller that
    /// cannot seek drops the bytes it holds before that offset, and passes on
    /// from there.
    ///
    /// The events borrow both the reader and `input`. They stop after a
    /// [`BoxEvent::Header`] and after the [`BoxEvent::End`] that follows a
    /// [`BoxEvent::Gathered`], and leave the rest of `input` untaken: the
    /// reader then wants it again, from the file offset it names. An empty
    /// `input` holds no events.
    ///
    /// # Errors
    ///
    /// * [`UnwantedInput`](DemuxError::UnwantedInput): `offset` is not one the
    ///   reader takes. The reader is left as it was.
    /// * [`UndecidedBox`](DemuxError::UndecidedBox): the [`OpenBox`] of the
    ///   last header was dropped unanswered. The reader is left as it was.
    /// * The failure the reader failed with before.
    ///
    /// Each event is itself a `Result`, failing as the reader fails:
    ///
    /// * [`Box`](DemuxError::Box): a header declares a size smaller than
    ///   itself.
    /// * [`PayloadLimitExceeded`](DemuxError::PayloadLimitExceeded): a box
    ///   gathered up to the end of the file outgrows its limit.
    pub fn handle_input<'a>(
        &'a mut self,
        offset: u64,
        input: &'a [u8],
    ) -> Result<BoxEvents<'a>, DemuxError> {
        self.check_answered()?;
        let wanted = self.wanted_input();
        if wanted.map(ByteRange::offset) != Some(offset) {
            return Err(DemuxError::UnwantedInput { offset, wanted });
        }
        if input.is_empty() {
            return Ok(BoxEvents::new(None, &[]));
        }
        self.position = offset;
        Ok(BoxEvents::new(Some(self), input))
    }

    /// Ends the file at the offset the reader wants, and returns the events that closes
    ///
    /// The events close the box being read: a [`BoxEvent::End`], after a
    /// [`BoxEvent::Gathered`] when the box is gathered. There are none when the
    /// file ends between boxes. A box whose size runs to the end of the file
    /// ends here, and a skipped box ends where its header says. The reader
    /// then wants no more input.
    ///
    /// # Errors
    ///
    /// * [`UndecidedBox`](DemuxError::UndecidedBox): the [`OpenBox`] of the
    ///   last header was dropped unanswered. The reader is left as it was.
    /// * The failure the reader failed with before.
    ///
    /// Each event is itself a `Result`, failing as the reader fails:
    ///
    /// * [`UnfinishedHeader`](DemuxError::UnfinishedHeader): the file ends
    ///   inside a header.
    /// * [`UnfinishedBox`](DemuxError::UnfinishedBox): the file ends before the
    ///   total a streamed or gathered box declares.
    pub fn finish(&mut self) -> Result<BoxEvents<'_>, DemuxError> {
        self.check_answered()?;
        self.at_end_of_file = true;
        Ok(BoxEvents::new(Some(self), &[]))
    }

    /// Refuses a call while the reader has failed or waits for an [`OpenBox`] to be answered
    fn check_answered(&self) -> Result<(), DemuxError> {
        match self.progress {
            Progress::Failed(failure) => Err(failure),
            Progress::Opened { header } => Err(DemuxError::UndecidedBox {
                box_type: header.box_type(),
            }),
            Progress::Header { .. } | Progress::Payload { .. } => Ok(()),
        }
    }

    fn fail(&mut self, failure: DemuxError) -> DemuxError {
        self.progress = Progress::Failed(failure);
        failure
    }

    /// Returns the bytes from the reader's position up to the file offset `end`, or `None` up to the end of the file
    fn remaining(&self, end: Option<u64>) -> Option<u64> {
        end.map(|end| end.saturating_sub(self.position))
    }

    fn raw_box(&self) -> Result<RawBox<'_>, DemuxError> {
        RawBox::split_first(&self.gathered)
            .map(|(raw_box, _)| raw_box)
            .map_err(|error| DemuxError::Box { error })
    }

    /// Takes from the front of `input` as far as the next event, and returns it
    fn step<'a>(&mut self, input: &mut &'a [u8]) -> Step<'a> {
        match self.progress {
            Progress::Header { needed, started } => self.read_header(input, needed, started),
            Progress::Payload {
                header,
                end,
                answer,
            } => self.read_payload(input, header, end, answer),
            Progress::Opened { .. } | Progress::Failed(_) => Step::Waiting,
        }
    }

    /// Takes the next header bytes off `input`, and opens the box once the header is whole
    fn read_header<'a>(
        &mut self,
        input: &mut &'a [u8],
        mut needed: u64,
        started: bool,
    ) -> Step<'a> {
        if !started {
            self.gathered.clear();
        }
        loop {
            let read = to_u64(self.gathered.len());
            let Some(bytes) = self.take(input, Some(needed.saturating_sub(read))) else {
                self.progress = Progress::Header {
                    needed,
                    started: read > 0,
                };
                if self.at_end_of_file && read > 0 {
                    return Step::Failed(self.fail(DemuxError::UnfinishedHeader {
                        needed_bytes: needed,
                        available_bytes: read,
                    }));
                }
                return Step::Waiting;
            };
            self.gathered.extend_from_slice(bytes);
            if to_u64(self.gathered.len()) < needed {
                continue;
            }
            match BoxHeader::decode(&self.gathered) {
                Ok((header, _)) => {
                    self.progress = Progress::Opened { header };
                    return Step::Opened(header);
                }
                Err(error) => {
                    let ErrorKind::TruncatedHeader { needed_bytes, .. } = error.kind() else {
                        return Step::Failed(self.fail(DemuxError::Box { error }));
                    };
                    needed = needed_bytes;
                }
            }
        }
    }

    /// Takes the next payload bytes off `input` as `answer` says, and closes the box once its payload is through
    fn read_payload<'a>(
        &mut self,
        input: &mut &'a [u8],
        header: BoxHeader,
        end: Option<u64>,
        answer: Answer,
    ) -> Step<'a> {
        let closes = |reader: &Self| match reader.remaining(end) {
            Some(remaining) => remaining == 0 || answer == Answer::Skip && reader.at_end_of_file,
            None => reader.at_end_of_file,
        };
        match answer {
            Answer::Skip => {}
            Answer::Stream => {
                if let Some(bytes) = self.take(input, self.remaining(end)) {
                    return Step::Event(BoxEvent::Payload(bytes));
                }
            }
            Answer::Gather { limit_bytes } => {
                let gathered_bytes =
                    to_u64(self.gathered.len()).saturating_sub(header.encoded_len());
                // Why not take all of the input up to the end of the payload: a
                // payload that runs to the end of the file would then fail at a
                // length that depends on where the input was cut. One byte past
                // the limit is enough to fail, and the same wherever the cut.
                let past_limit = limit_bytes.saturating_sub(gathered_bytes).saturating_add(1);
                let wanted = self
                    .remaining(end)
                    .map_or(past_limit, |remaining| remaining.min(past_limit));
                if let Some(bytes) = self.take(input, Some(wanted)) {
                    let reached_bytes = gathered_bytes.saturating_add(to_u64(bytes.len()));
                    if reached_bytes > limit_bytes {
                        return Step::Failed(self.fail(DemuxError::PayloadLimitExceeded {
                            box_type: header.box_type(),
                            reached_bytes,
                            limit_bytes,
                        }));
                    }
                    self.gathered.extend_from_slice(bytes);
                }
            }
        }
        if closes(self) {
            self.progress = Progress::Header {
                needed: SHORTEST_HEADER,
                started: false,
            };
            return match answer {
                Answer::Gather { .. } => Step::Gathered,
                Answer::Stream | Answer::Skip => Step::Event(BoxEvent::End),
            };
        }
        if self.at_end_of_file {
            let needed_bytes = header.size().total_bytes().unwrap_or(u64::MAX);
            return Step::Failed(self.fail(DemuxError::UnfinishedBox {
                box_type: header.box_type(),
                needed_bytes,
                available_bytes: needed_bytes.saturating_sub(self.remaining(end).unwrap_or(0)),
            }));
        }
        Step::Waiting
    }

    /// Takes up to `limit` bytes off the front of `input`, or every byte for `None`
    ///
    /// Returns `None` when it takes nothing.
    fn take<'a>(&mut self, input: &mut &'a [u8], limit: Option<u64>) -> Option<&'a [u8]> {
        let length = limit
            .and_then(|limit| usize::try_from(limit).ok())
            .map_or(input.len(), |limit| limit.min(input.len()));
        let taken = input
            .split_off(..length)
            .filter(|taken| !taken.is_empty())?;
        self.position = self.position.saturating_add(to_u64(taken.len()));
        Some(taken)
    }
}

impl Default for BoxReader {
    fn default() -> Self {
        Self::new()
    }
}

/// What one step of a reader through its input comes to
enum Step<'a> {
    Event(BoxEvent<'a>),
    Opened(BoxHeader),
    Gathered,
    /// Input used up before the next event
    Waiting,
    Failed(DemuxError),
}

/// Converts a length in memory to one in a file, where it always fits
fn to_u64(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxType, Error};

    use super::{BoxEvent, BoxReader};
    use crate::byte_range::ByteRange;
    use crate::error::DemuxError;
    use crate::framing::header_forms::{PAYLOAD, every_header_form};

    /// What a caller answers each header with
    #[derive(Clone, Copy)]
    enum Answer {
        Gather(u64),
        Stream,
        Skip,
    }

    /// Event as a caller records it, the payload pieces of one box joined
    #[derive(PartialEq, Eq, Debug)]
    enum Recorded {
        Header(BoxHeader, ByteRange),
        Payload(Vec<u8>),
        Gathered(BoxHeader, Vec<u8>),
        End,
    }

    /// Reads `file` from wherever the reader wants it, `piece_len(offset)` bytes at a time, and finishes it where the reader wants no more or the file ends
    fn read(
        file: &[u8],
        piece_len: impl Fn(u64) -> usize,
        answer: Answer,
    ) -> Result<Vec<Recorded>, DemuxError> {
        let mut reader = BoxReader::new();
        let mut recorded = Vec::new();
        while let Some(wanted) = reader.wanted_input() {
            let rest = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            let input = rest.get(..piece_len(wanted.offset())).unwrap_or(rest);
            if input.is_empty() {
                break;
            }
            for event in reader.handle_input(wanted.offset(), input)? {
                record(event?, answer, &mut recorded)?;
            }
        }
        for event in reader.finish()? {
            record(event?, answer, &mut recorded)?;
        }
        Ok(recorded)
    }

    fn record(
        event: BoxEvent<'_>,
        answer: Answer,
        recorded: &mut Vec<Recorded>,
    ) -> Result<(), DemuxError> {
        match event {
            BoxEvent::Header(open) => {
                recorded.push(Recorded::Header(open.header(), open.byte_range()));
                match answer {
                    Answer::Gather(limit_bytes) => open.gather(limit_bytes)?,
                    Answer::Stream => open.stream(),
                    Answer::Skip => open.skip(),
                }
            }
            BoxEvent::Payload(bytes) => match recorded.last_mut() {
                Some(Recorded::Payload(joined)) => joined.extend_from_slice(bytes),
                Some(_) | None => recorded.push(Recorded::Payload(bytes.to_vec())),
            },
            BoxEvent::Gathered(raw_box) => {
                recorded.push(Recorded::Gathered(
                    raw_box.header(),
                    raw_box.payload().to_vec(),
                ));
            }
            BoxEvent::End => recorded.push(Recorded::End),
        }
        Ok(())
    }

    /// Every way to cut `file` in two, then `file` a byte at a time
    fn every_cut(file: &[u8]) -> impl Iterator<Item = impl Fn(u64) -> usize> {
        let length = u64::try_from(file.len()).unwrap();
        (0..=length).map(Some).chain([None]).map(|cut| {
            move |offset: u64| match cut {
                Some(cut) if offset < cut => usize::try_from(cut.abs_diff(offset)).unwrap(),
                Some(_) => usize::MAX,
                None => 1,
            }
        })
    }

    fn header_and_range(file: &[u8]) -> (BoxHeader, ByteRange) {
        let (header, _) = BoxHeader::decode(file).unwrap();
        (header, ByteRange::new(0, header.size().total_bytes()))
    }

    #[test]
    fn a_streamed_box_of_every_header_form_arrives_as_its_payload_wherever_the_input_is_cut() {
        for file in every_header_form() {
            let (header, byte_range) = header_and_range(&file);
            for piece_len in every_cut(&file) {
                assert_eq!(
                    read(&file, piece_len, Answer::Stream),
                    Ok(vec![
                        Recorded::Header(header, byte_range),
                        Recorded::Payload(PAYLOAD.to_vec()),
                        Recorded::End,
                    ])
                );
            }
        }
    }

    #[test]
    fn a_gathered_box_of_every_header_form_arrives_whole_wherever_the_input_is_cut() {
        for file in every_header_form() {
            let (header, byte_range) = header_and_range(&file);
            for piece_len in every_cut(&file) {
                assert_eq!(
                    read(&file, piece_len, Answer::Gather(8)),
                    Ok(vec![
                        Recorded::Header(header, byte_range),
                        Recorded::Gathered(header, PAYLOAD.to_vec()),
                        Recorded::End,
                    ])
                );
            }
        }
    }

    #[test]
    fn a_skipped_box_of_every_header_form_closes_without_its_payload_wherever_the_input_is_cut() {
        for file in every_header_form() {
            let (header, byte_range) = header_and_range(&file);
            for piece_len in every_cut(&file) {
                assert_eq!(
                    read(&file, piece_len, Answer::Skip),
                    Ok(vec![Recorded::Header(header, byte_range), Recorded::End])
                );
            }
        }
    }

    #[test]
    fn a_skipped_box_moves_the_wanted_input_past_its_end() {
        let mut reader = BoxReader::new();
        for event in reader.handle_input(0, b"\0\0\0\x10freeAAAA").unwrap() {
            if let BoxEvent::Header(open) = event.unwrap() {
                open.skip();
            }
        }
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(16, None)));
    }

    #[test]
    fn a_skipped_box_running_to_the_end_of_the_file_leaves_no_input_wanted() {
        let mut reader = BoxReader::new();
        for event in reader.handle_input(0, b"\0\0\0\0mdatAAAA").unwrap() {
            if let BoxEvent::Header(open) = event.unwrap() {
                open.skip();
            }
        }
        assert_eq!(reader.wanted_input(), None);
    }

    #[test]
    fn boxes_laid_end_to_end_are_read_in_file_order() {
        let file = [
            b"\0\0\0\x0cfreeAAAA".as_slice(),
            b"\0\0\0\x08skip",
            b"\0\0\0\x0bmdatBBB",
        ]
        .concat();
        let free = BoxHeader::decode(&file).unwrap().0;
        let skip = BoxHeader::decode(file.get(12..).unwrap()).unwrap().0;
        let mdat = BoxHeader::decode(file.get(20..).unwrap()).unwrap().0;
        assert_eq!(
            read(&file, |_| usize::MAX, Answer::Stream),
            Ok(vec![
                Recorded::Header(free, ByteRange::new(0, Some(12))),
                Recorded::Payload(b"AAAA".to_vec()),
                Recorded::End,
                Recorded::Header(skip, ByteRange::new(12, Some(8))),
                Recorded::End,
                Recorded::Header(mdat, ByteRange::new(20, Some(11))),
                Recorded::Payload(b"BBB".to_vec()),
                Recorded::End,
            ])
        );
    }

    #[test]
    fn a_payload_declared_past_the_gather_limit_fails_the_reader() {
        let file = b"\0\0\0\x10freeAAAAAAAA";
        let failure = DemuxError::PayloadLimitExceeded {
            box_type: BoxType::compact(*b"free"),
            reached_bytes: 8,
            limit_bytes: 7,
        };
        assert_eq!(read(file, |_| usize::MAX, Answer::Gather(7)), Err(failure));
    }

    #[test]
    fn a_payload_running_to_the_end_of_the_file_fails_the_reader_once_it_outgrows_the_gather_limit()
    {
        let file = b"\0\0\0\0mdatAAAAAAAAAAAA";
        let failure = DemuxError::PayloadLimitExceeded {
            box_type: BoxType::compact(*b"mdat"),
            reached_bytes: 8,
            limit_bytes: 7,
        };
        for piece_len in every_cut(file) {
            assert_eq!(read(file, piece_len, Answer::Gather(7)), Err(failure));
        }
    }

    #[test]
    fn a_failed_reader_returns_its_failure_and_wants_no_more_input() {
        let mut reader = BoxReader::new();
        let failure = DemuxError::Box {
            error: Error::size_below_header(8, 4),
        };
        let events: Vec<_> = reader.handle_input(0, b"\0\0\0\x04free").unwrap().collect();
        assert_eq!(events.len(), 1);
        assert!(matches!(events.first(), Some(Err(found)) if *found == failure));
        assert_eq!(reader.wanted_input(), None);
        assert_eq!(reader.handle_input(8, b"more").err(), Some(failure));
        assert_eq!(reader.finish().err(), Some(failure));
    }

    #[test]
    fn a_file_ending_inside_a_header_is_unfinished() {
        let mut reader = BoxReader::new();
        assert_eq!(
            reader
                .handle_input(0, b"\0\0\0\x01mdat\0\0")
                .unwrap()
                .count(),
            0
        );
        let failure = DemuxError::UnfinishedHeader {
            needed_bytes: 16,
            available_bytes: 10,
        };
        let events: Vec<_> = reader.finish().unwrap().collect();
        assert!(matches!(events.as_slice(), [Err(found)] if *found == failure));
        assert_eq!(reader.finish().err(), Some(failure));
    }

    #[test]
    fn a_file_ending_inside_a_box_is_unfinished() {
        let failure = DemuxError::UnfinishedBox {
            box_type: BoxType::compact(*b"free"),
            needed_bytes: 16,
            available_bytes: 12,
        };
        assert_eq!(
            read(b"\0\0\0\x10freeAAAA", |_| usize::MAX, Answer::Stream),
            Err(failure)
        );
    }

    #[test]
    fn a_file_ending_between_boxes_finishes_with_no_events() {
        let mut reader = BoxReader::new();
        assert_eq!(reader.finish().unwrap().count(), 0);
        assert_eq!(reader.wanted_input(), None);
    }

    #[test]
    fn input_at_an_unwanted_offset_is_refused_and_leaves_the_reader_as_it_was() {
        let mut reader = BoxReader::new();
        assert_eq!(
            reader.handle_input(1, b"\0\0\x08free").err(),
            Some(DemuxError::UnwantedInput {
                offset: 1,
                wanted: Some(ByteRange::new(0, None)),
            })
        );
        assert_eq!(
            reader.handle_input(0, b"\0\0\0\x08free").unwrap().count(),
            1
        );
    }

    #[test]
    fn empty_input_leaves_the_reader_as_it_was() {
        let mut reader = BoxReader::new();
        for event in reader.handle_input(0, b"\0\0\0\x0cfree").unwrap() {
            if let BoxEvent::Header(open) = event.unwrap() {
                open.stream();
            }
        }
        let wanted = reader.wanted_input();
        assert_eq!(reader.handle_input(8, &[]).unwrap().count(), 0);
        assert_eq!(reader.wanted_input(), wanted);
    }

    #[test]
    fn input_before_the_open_box_is_answered_is_refused() {
        let mut reader = BoxReader::new();
        let header = reader.handle_input(0, b"\0\0\0\x0cfree").unwrap().next();
        assert!(matches!(header, Some(Ok(BoxEvent::Header(_)))));
        assert_eq!(
            reader.handle_input(8, b"AAAA").err(),
            Some(DemuxError::UndecidedBox {
                box_type: BoxType::compact(*b"free"),
            })
        );
    }
}
