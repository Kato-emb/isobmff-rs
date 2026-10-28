//! [`BoxReader`], which frames the boxes of a file out of bytes that arrive in pieces

use alloc::vec::Vec;

use isobmff_core::{BoxHeader, ErrorKind, RawBox};

use crate::byte_range::ByteRange;
use crate::error::DemuxError;
use crate::framing::box_event::{BoxEvent, BoxEvents};
use crate::framing::to_u64;

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
/// Each box opens with [`BoxEvent::Header`], which the caller answers with
/// what to do with the payload: [`gather`](Self::gather) it into one
/// [`RawBox`], [`stream`](Self::stream) it as [`BoxEvent::Payload`]s borrowed
/// from the input, or [`skip`](Self::skip) it. The reader takes no more input
/// until it has the answer. Each box closes with [`BoxEvent::End`].
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
///         match event {
///             BoxEvent::Header(header) if header.box_type() == BoxType::compact(*b"ftyp") => {
///                 reader.gather(1024)?
///             }
///             BoxEvent::Header(header) if header.box_type() == BoxType::compact(*b"mdat") => {
///                 reader.stream()?
///             }
///             BoxEvent::Header(_) => reader.skip()?,
///             BoxEvent::Payload(bytes) => media_data.extend_from_slice(bytes),
///             BoxEvent::End => {
///                 if let Some(ftyp) = reader.gathered_box() {
///                     brands.extend_from_slice(ftyp.payload());
///                 }
///             }
///             _ => {}
///         }
///     }
/// }
///
/// // The file has no more bytes, which ends the `mdat`
/// for event in reader.finish()? {
///     assert!(matches!(event, BoxEvent::End));
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
    /// Reading a header that is `needed` bytes long as far as it is known, what is read of it in `gathered`
    Header {
        needed: u64,
    },
    /// Header of the box at file offset `offset` read, waiting for the caller to answer it
    Opened {
        header: BoxHeader,
        offset: u64,
    },
    /// Payload of the box at file offset `offset` read as `answer` says, up to the file offset `end` or, for `None`, the end of the file
    Payload {
        header: BoxHeader,
        offset: u64,
        end: Option<u64>,
        answer: Answer,
    },
    /// Box whole in `gathered`, until the next input
    Gathered,
    Failed(DemuxError),
}

/// What the caller answered a header with
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
            Progress::Header { .. } | Progress::Gathered => None,
            Progress::Opened { header, .. } => header.payload_len(),
            Progress::Payload {
                end: None,
                answer: Answer::Skip,
                ..
            }
            | Progress::Failed(_) => return None,
            Progress::Payload { end, .. } => self.remaining(end).filter(|&remaining| remaining > 0),
        };
        Some(ByteRange::new(self.position, length))
    }

    /// Takes `input`, the bytes of the file from file offset `offset`, and returns what they hold
    ///
    /// `input` is taken from the offset [`wanted_input`](Self::wanted_input)
    /// names, which it must start at or before. Bytes before that offset are
    /// passed over, so a caller can pass the same input again after the events
    /// stop, and a caller that cannot seek can pass the file on as it comes:
    /// an input that ends before the wanted offset holds no events. Bytes passed
    /// again must be the bytes passed before.
    ///
    /// The events borrow `input` alone, so the caller answers a
    /// [`BoxEvent::Header`] on the reader while it goes through them. They stop
    /// after a [`BoxEvent::Header`], which waits for its answer, and after the
    /// [`BoxEvent::End`] of a gathered box, which
    /// [`gathered_box`](Self::gathered_box) then returns.
    ///
    /// A failure after the first event is returned by the next call, and the
    /// reader then wants no more input: the events before it are returned
    /// first.
    ///
    /// # Errors
    ///
    /// * [`UnwantedInput`](DemuxError::UnwantedInput): `input` starts past the
    ///   wanted offset, or the reader wants no more. The reader is left as it
    ///   was.
    /// * [`UndecidedBox`](DemuxError::UndecidedBox): the last header is not
    ///   answered yet. The reader is left as it was.
    /// * [`Box`](DemuxError::Box): a header declares a size smaller than
    ///   itself.
    /// * [`PayloadLimitExceeded`](DemuxError::PayloadLimitExceeded): a box
    ///   gathered up to the end of the file outgrows its limit.
    /// * The failure the reader failed with before.
    pub fn handle_input<'a>(
        &mut self,
        offset: u64,
        input: &'a [u8],
    ) -> Result<BoxEvents<'a>, DemuxError> {
        self.check_answered()?;
        let wanted = self.wanted_input();
        let Some(passed_over) = wanted.and_then(|wanted| wanted.offset().checked_sub(offset))
        else {
            return Err(DemuxError::UnwantedInput { offset, wanted });
        };
        self.leave_gathered();
        let Some(rest) = usize::try_from(passed_over)
            .ok()
            .and_then(|passed_over| input.get(passed_over..))
            .filter(|rest| !rest.is_empty())
        else {
            return Ok(BoxEvents::default());
        };
        self.position = offset.saturating_add(passed_over);
        self.read(rest)
    }

    /// Ends the file at the offset the reader wants, and returns the events that close the box being read
    ///
    /// The events are a [`BoxEvent::End`], or none when the file ends between
    /// boxes. A box whose size runs to the end of the file ends here, and so
    /// may one whose payload has nothing left to arrive, so the events go where
    /// those of [`handle_input`](Self::handle_input) go. A skipped box ends
    /// where its header says. The reader then wants no more input.
    ///
    /// # Errors
    ///
    /// * [`UndecidedBox`](DemuxError::UndecidedBox): the last header is not
    ///   answered yet. The reader is left as it was.
    /// * [`UnfinishedHeader`](DemuxError::UnfinishedHeader): the file ends
    ///   inside a header.
    /// * [`UnfinishedBox`](DemuxError::UnfinishedBox): the file ends before the
    ///   total a streamed or gathered box declares.
    /// * The failure the reader failed with before.
    pub fn finish(&mut self) -> Result<BoxEvents<'static>, DemuxError> {
        self.check_answered()?;
        self.leave_gathered();
        self.at_end_of_file = true;
        self.read(&[])
    }

    /// Gathers the payload of the box just opened into one [`RawBox`], which [`gathered_box`](Self::gathered_box) returns after its [`BoxEvent::End`]
    ///
    /// # Errors
    ///
    /// * [`PayloadLimitExceeded`](DemuxError::PayloadLimitExceeded): the
    ///   payload the header declares is longer than `limit_bytes`. The box
    ///   stays open, to be answered again. A payload that runs to the end of
    ///   the file fails the reader instead, once it outgrows the limit.
    /// * [`NoOpenBox`](DemuxError::NoOpenBox): no header waits for an answer.
    /// * The failure the reader failed with before.
    pub fn gather(&mut self, limit_bytes: u64) -> Result<(), DemuxError> {
        let (header, offset) = self.open_header()?;
        let payload_len = header.payload_len();
        if let Some(payload_len) = payload_len
            && payload_len > limit_bytes
        {
            return Err(DemuxError::PayloadLimitExceeded {
                box_type: header.box_type(),
                reached_bytes: payload_len,
                limit_bytes,
            });
        }
        if let Some(payload_len) = payload_len.and_then(|len| usize::try_from(len).ok()) {
            self.gathered.reserve(payload_len);
        }
        self.answer(header, offset, Answer::Gather { limit_bytes });
        Ok(())
    }

    /// Streams the payload of the box just opened as [`BoxEvent::Payload`]s, each a piece of the input it arrives in
    ///
    /// # Errors
    ///
    /// * [`NoOpenBox`](DemuxError::NoOpenBox): no header waits for an answer.
    /// * The failure the reader failed with before.
    pub fn stream(&mut self) -> Result<(), DemuxError> {
        let (header, offset) = self.open_header()?;
        self.answer(header, offset, Answer::Stream);
        Ok(())
    }

    /// Skips the payload of the box just opened: the reader next wants the file offset just past the box
    ///
    /// A box whose size runs to the end of the file leaves the reader wanting
    /// no more input.
    ///
    /// # Errors
    ///
    /// * [`NoOpenBox`](DemuxError::NoOpenBox): no header waits for an answer.
    /// * The failure the reader failed with before.
    pub fn skip(&mut self) -> Result<(), DemuxError> {
        let (header, offset) = self.open_header()?;
        self.answer(header, offset, Answer::Skip);
        Ok(())
    }

    /// Returns where the box being read lies in the file, header included, or `None` between boxes
    ///
    /// The length is `None` for a box whose size runs to the end of the file.
    #[must_use]
    pub fn open_box(&self) -> Option<ByteRange> {
        match self.progress {
            Progress::Opened { header, offset } | Progress::Payload { header, offset, .. } => {
                Some(ByteRange::new(offset, header.size().total_bytes()))
            }
            Progress::Header { .. } | Progress::Gathered | Progress::Failed(_) => None,
        }
    }

    /// Returns the box just gathered, from its [`BoxEvent::End`] until the next call to [`handle_input`](Self::handle_input) or [`finish`](Self::finish)
    #[must_use]
    pub fn gathered_box(&self) -> Option<RawBox<'_>> {
        if !matches!(self.progress, Progress::Gathered) {
            return None;
        }
        RawBox::split_first(&self.gathered)
            .ok()
            .map(|(raw_box, _)| raw_box)
    }

    /// Refuses a call while the reader has failed or waits for a header to be answered
    fn check_answered(&self) -> Result<(), DemuxError> {
        match self.progress {
            Progress::Failed(failure) => Err(failure),
            Progress::Opened { header, .. } => Err(DemuxError::UndecidedBox {
                box_type: header.box_type(),
            }),
            Progress::Header { .. } | Progress::Payload { .. } | Progress::Gathered => Ok(()),
        }
    }

    /// Returns the header waiting for an answer, and the file offset of its box
    fn open_header(&self) -> Result<(BoxHeader, u64), DemuxError> {
        match self.progress {
            Progress::Opened { header, offset } => Ok((header, offset)),
            Progress::Failed(failure) => Err(failure),
            Progress::Header { .. } | Progress::Payload { .. } | Progress::Gathered => {
                Err(DemuxError::NoOpenBox)
            }
        }
    }

    fn answer(&mut self, header: BoxHeader, offset: u64, answer: Answer) {
        let end = header
            .size()
            .total_bytes()
            .map(|total| offset.saturating_add(total));
        if let (Answer::Skip, Some(end)) = (answer, end) {
            self.position = end;
        }
        self.progress = Progress::Payload {
            header,
            offset,
            end,
            answer,
        };
    }

    /// Makes ready for the header after a gathered box, which [`gathered_box`](Self::gathered_box) no longer returns
    fn leave_gathered(&mut self) {
        if let Progress::Gathered = self.progress {
            self.gathered.clear();
            self.progress = Progress::Header {
                needed: SHORTEST_HEADER,
            };
        }
    }

    /// Fails the reader, and stops the events there
    fn fail<'a>(&mut self, failure: DemuxError) -> Step<'a> {
        self.progress = Progress::Failed(failure);
        Step::Stop(None)
    }

    /// Returns the bytes from the reader's position up to the file offset `end`, or `None` up to the end of the file
    fn remaining(&self, end: Option<u64>) -> Option<u64> {
        end.map(|end| end.saturating_sub(self.position))
    }

    /// Reads `input` as far as the events stop, and returns them
    fn read<'a>(&mut self, mut input: &'a [u8]) -> Result<BoxEvents<'a>, DemuxError> {
        let mut events = BoxEvents::default();
        let stopped_at = loop {
            match self.step(&mut input) {
                Step::Event(event) => events.push(event),
                Step::Stop(event) => break event,
            }
        };
        if let Some(event) = stopped_at {
            events.push(event);
        }
        if let Progress::Failed(failure) = self.progress
            && events.is_empty()
        {
            return Err(failure);
        }
        Ok(events)
    }

    /// Takes from the front of `input` as far as the next event, and returns it
    fn step<'a>(&mut self, input: &mut &'a [u8]) -> Step<'a> {
        match self.progress {
            Progress::Header { needed } => self.read_header(input, needed),
            Progress::Payload {
                header,
                end,
                answer,
                ..
            } => self.read_payload(input, header, end, answer),
            Progress::Opened { .. } | Progress::Gathered | Progress::Failed(_) => Step::Stop(None),
        }
    }

    /// Takes the next header bytes off `input`, and opens the box once the header is whole
    fn read_header<'a>(&mut self, input: &mut &'a [u8], mut needed: u64) -> Step<'a> {
        // Why not bound the turns: each takes at least a byte or returns, and
        // `BoxHeader::decode` asks only for more than it was given, so `needed`
        // stays above what is read until the header is whole.
        loop {
            let read = to_u64(self.gathered.len());
            let Some(bytes) = self.take(input, Some(needed.saturating_sub(read))) else {
                self.progress = Progress::Header { needed };
                if self.at_end_of_file && read > 0 {
                    return self.fail(DemuxError::UnfinishedHeader {
                        needed_bytes: needed,
                        available_bytes: read,
                    });
                }
                return Step::Stop(None);
            };
            self.gathered.extend_from_slice(bytes);
            if to_u64(self.gathered.len()) < needed {
                continue;
            }
            match BoxHeader::decode(&self.gathered) {
                Ok((header, _)) => {
                    let offset = self.position.saturating_sub(to_u64(self.gathered.len()));
                    self.progress = Progress::Opened { header, offset };
                    return Step::Stop(Some(BoxEvent::Header(header)));
                }
                Err(error) => {
                    let ErrorKind::TruncatedHeader { needed_bytes, .. } = error.kind() else {
                        return self.fail(DemuxError::Box { error });
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
                        return self.fail(DemuxError::PayloadLimitExceeded {
                            box_type: header.box_type(),
                            reached_bytes,
                            limit_bytes,
                        });
                    }
                    self.gathered.extend_from_slice(bytes);
                }
            }
        }
        let remaining = self.remaining(end);
        if remaining == Some(0) || remaining.is_none() && self.at_end_of_file {
            if let Answer::Gather { .. } = answer {
                self.progress = Progress::Gathered;
                return Step::Stop(Some(BoxEvent::End));
            }
            self.gathered.clear();
            self.progress = Progress::Header {
                needed: SHORTEST_HEADER,
            };
            return Step::Event(BoxEvent::End);
        }
        if self.at_end_of_file {
            let needed_bytes = header.size().total_bytes().unwrap_or(u64::MAX);
            return self.fail(DemuxError::UnfinishedBox {
                box_type: header.box_type(),
                needed_bytes,
                available_bytes: needed_bytes.saturating_sub(remaining.unwrap_or(0)),
            });
        }
        Step::Stop(None)
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
    /// An event, after which the reader reads on
    Event(BoxEvent<'a>),
    /// The point the events stop at, with the event that stops them if any
    Stop(Option<BoxEvent<'a>>),
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxType, Error};

    use super::BoxReader;
    use crate::byte_range::ByteRange;
    use crate::error::DemuxError;
    use crate::framing::box_event::BoxEvent;
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
        Header(BoxHeader, Option<ByteRange>),
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
                record(&mut reader, event, answer, &mut recorded)?;
            }
        }
        for event in reader.finish()? {
            record(&mut reader, event, answer, &mut recorded)?;
        }
        Ok(recorded)
    }

    fn record(
        reader: &mut BoxReader,
        event: BoxEvent<'_>,
        answer: Answer,
        recorded: &mut Vec<Recorded>,
    ) -> Result<(), DemuxError> {
        match event {
            BoxEvent::Header(header) => {
                recorded.push(Recorded::Header(header, reader.open_box()));
                match answer {
                    Answer::Gather(limit_bytes) => reader.gather(limit_bytes)?,
                    Answer::Stream => reader.stream()?,
                    Answer::Skip => reader.skip()?,
                }
            }
            BoxEvent::Payload(bytes) => match recorded.last_mut() {
                Some(Recorded::Payload(joined)) => joined.extend_from_slice(bytes),
                Some(_) | None => recorded.push(Recorded::Payload(bytes.to_vec())),
            },
            BoxEvent::End => {
                if let Some(raw_box) = reader.gathered_box() {
                    recorded.push(Recorded::Gathered(
                        raw_box.header(),
                        raw_box.payload().to_vec(),
                    ));
                }
                recorded.push(Recorded::End);
            }
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

    fn header_and_range(file: &[u8]) -> (BoxHeader, Option<ByteRange>) {
        let (header, _) = BoxHeader::decode(file).unwrap();
        (header, Some(ByteRange::new(0, header.size().total_bytes())))
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
                Recorded::Header(free, Some(ByteRange::new(0, Some(12)))),
                Recorded::Payload(b"AAAA".to_vec()),
                Recorded::End,
                Recorded::Header(skip, Some(ByteRange::new(12, Some(8)))),
                Recorded::End,
                Recorded::Header(mdat, Some(ByteRange::new(20, Some(11)))),
                Recorded::Payload(b"BBB".to_vec()),
                Recorded::End,
            ])
        );
    }

    #[test]
    fn the_same_input_passed_again_reads_on_from_where_the_events_stopped() {
        let file = [b"\0\0\0\x0cfreeAAAA".as_slice(), b"\0\0\0\x0bmdatBBB"].concat();
        let mut reader = BoxReader::new();
        let mut recorded = Vec::new();
        loop {
            let events: Vec<_> = reader.handle_input(0, &file).unwrap().collect();
            if events.is_empty() {
                break;
            }
            for event in events {
                record(&mut reader, event, Answer::Stream, &mut recorded).unwrap();
            }
        }
        for event in reader.finish().unwrap() {
            record(&mut reader, event, Answer::Stream, &mut recorded).unwrap();
        }
        assert_eq!(Ok(recorded), read(&file, |_| 1, Answer::Stream));
    }

    #[test]
    fn input_ending_before_the_wanted_offset_holds_no_events() {
        let mut reader = BoxReader::new();
        for event in reader.handle_input(0, b"\0\0\0\x10freeAAAA").unwrap() {
            if let BoxEvent::Header(_) = event {
                reader.skip().unwrap();
            }
        }
        let wanted = reader.wanted_input();
        assert_eq!(reader.handle_input(8, b"AAAA").unwrap().count(), 0);
        assert_eq!(reader.handle_input(12, b"AAAA").unwrap().count(), 0);
        assert_eq!(reader.wanted_input(), wanted);
        assert_eq!(
            reader
                .handle_input(12, b"AAAA\0\0\0\x08skip")
                .unwrap()
                .collect::<Vec<_>>(),
            [
                BoxEvent::End,
                BoxEvent::Header(BoxHeader::decode(b"\0\0\0\x08skip").unwrap().0)
            ]
        );
    }

    #[test]
    fn a_skipped_box_moves_the_wanted_input_past_its_end() {
        let mut reader = BoxReader::new();
        for event in reader.handle_input(0, b"\0\0\0\x10freeAAAA").unwrap() {
            if let BoxEvent::Header(_) = event {
                reader.skip().unwrap();
            }
        }
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(16, None)));
    }

    #[test]
    fn a_skipped_box_running_to_the_end_of_the_file_leaves_no_input_wanted() {
        let mut reader = BoxReader::new();
        for event in reader.handle_input(0, b"\0\0\0\0mdatAAAA").unwrap() {
            if let BoxEvent::Header(_) = event {
                reader.skip().unwrap();
            }
        }
        assert_eq!(reader.wanted_input(), None);
    }

    #[test]
    fn a_payload_declared_past_the_gather_limit_is_refused_and_the_box_stays_open() {
        let mut reader = BoxReader::new();
        assert_eq!(
            reader
                .handle_input(0, b"\0\0\0\x10freeAAAAAAAA")
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            reader.gather(7),
            Err(DemuxError::PayloadLimitExceeded {
                box_type: BoxType::compact(*b"free"),
                reached_bytes: 8,
                limit_bytes: 7,
            })
        );
        assert_eq!(reader.skip(), Ok(()));
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(16, None)));
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
        assert_eq!(
            reader.handle_input(0, b"\0\0\0\x04free").err(),
            Some(failure)
        );
        assert_eq!(reader.wanted_input(), None);
        assert_eq!(reader.handle_input(8, b"more").err(), Some(failure));
        assert_eq!(reader.finish().err(), Some(failure));
    }

    #[test]
    fn a_failure_after_events_is_returned_by_the_next_call() {
        let mut reader = BoxReader::new();
        assert_eq!(
            reader.handle_input(0, b"\0\0\0\x0cfree").unwrap().count(),
            1
        );
        reader.stream().unwrap();
        let events: Vec<_> = reader
            .handle_input(8, b"AAAA\0\0\0\x04free")
            .unwrap()
            .collect();
        assert_eq!(events, [BoxEvent::Payload(b"AAAA"), BoxEvent::End]);
        assert_eq!(reader.wanted_input(), None);
        assert_eq!(
            reader.finish().err(),
            Some(DemuxError::Box {
                error: Error::size_below_header(8, 4),
            })
        );
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
        assert_eq!(reader.finish().err(), Some(failure));
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
    fn input_past_the_wanted_offset_is_refused_and_leaves_the_reader_as_it_was() {
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
        assert_eq!(
            reader.handle_input(0, b"\0\0\0\x0cfree").unwrap().count(),
            1
        );
        reader.stream().unwrap();
        let wanted = reader.wanted_input();
        assert_eq!(reader.handle_input(8, &[]).unwrap().count(), 0);
        assert_eq!(reader.wanted_input(), wanted);
    }

    #[test]
    fn input_before_the_header_is_answered_is_refused() {
        let mut reader = BoxReader::new();
        assert_eq!(
            reader.handle_input(0, b"\0\0\0\x0cfree").unwrap().count(),
            1
        );
        let undecided = DemuxError::UndecidedBox {
            box_type: BoxType::compact(*b"free"),
        };
        assert_eq!(reader.handle_input(8, b"AAAA").err(), Some(undecided));
        assert_eq!(reader.finish().err(), Some(undecided));
    }

    #[test]
    fn an_answer_with_no_header_waiting_is_refused() {
        let mut reader = BoxReader::new();
        assert_eq!(reader.skip(), Err(DemuxError::NoOpenBox));
        assert_eq!(
            reader.handle_input(0, b"\0\0\0\x0cfree").unwrap().count(),
            1
        );
        reader.stream().unwrap();
        assert_eq!(reader.gather(8), Err(DemuxError::NoOpenBox));
    }

    #[test]
    fn a_gathered_box_is_returned_until_the_next_input() {
        let file = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
        let mut reader = BoxReader::new();
        assert_eq!(reader.handle_input(0, file).unwrap().count(), 1);
        reader.gather(8).unwrap();
        assert_eq!(
            reader.handle_input(0, file).unwrap().collect::<Vec<_>>(),
            [BoxEvent::End]
        );
        assert_eq!(
            reader.gathered_box().map(|raw_box| raw_box.payload()),
            Some(b"AAAA".as_slice())
        );
        assert_eq!(reader.handle_input(0, file).unwrap().count(), 1);
        assert_eq!(reader.gathered_box(), None);
    }
}
