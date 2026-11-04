//! [`BoxReader`], which frames the boxes of a file out of bytes that arrive in pieces

use alloc::vec::Vec;

use isobmff_core::{BoxHeader, ErrorKind, RawBox};

use crate::byte_range::ByteRange;
use crate::byte_range::to_u64;
use crate::error::DemuxError;
use crate::framing::box_event::BoxEvent;

/// Length of the `size` and `boxtype` fields every header opens with
const SHORTEST_HEADER: u64 = 8;

/// Reader that frames the boxes laid end to end in a file, ISO/IEC 14496-12 §4.2.2
///
/// The reader does no I/O. It names the bytes it wants next as
/// [`wanted_input`](Self::wanted_input), takes them with the file offset they
/// start at in [`handle_input`](Self::handle_input), and reports what they
/// hold as [`BoxEvent`]s, one a call. Bytes may arrive cut anywhere, a byte
/// at a time if need be.
///
/// Each box opens with [`BoxEvent::Header`], which the caller answers with
/// what to do with the payload: [`gather`](Self::gather) it into one
/// [`RawBox`], [`stream`](Self::stream) it as [`BoxEvent::Payload`]s borrowed
/// from the input, [`skip`](Self::skip) it, or
/// [`gather_head`](Self::gather_head): gather the first bytes of it, as many
/// as a box needs to tell what it is, and skip the rest. The reader takes no
/// more input until it has the answer. Each box closes with
/// [`BoxEvent::End`].
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
///     while let Some(event) = reader.handle_input(wanted.offset(), input)? {
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
/// assert_eq!(reader.finish()?, Some(BoxEvent::End));
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
    /// Box of header `header` at file offset `offset` in `gathered`, until the next input: whole where `whole`, otherwise the head of its payload
    Gathered {
        header: BoxHeader,
        offset: u64,
        whole: bool,
    },
    Failed(DemuxError),
}

/// What the caller answered a header with
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Answer {
    Gather {
        limit_bytes: u64,
    },
    /// Gathers the first `head_bytes` of the payload, and skips the rest
    GatherHead {
        head_bytes: u64,
    },
    Stream,
    Skip,
}

impl BoxReader {
    /// Creates a reader that wants the file from its first byte
    #[must_use]
    pub const fn new() -> Self {
        Self::starting_at(0)
    }

    /// Creates a reader that wants the file from file offset `position`, where a box starts
    ///
    /// The boxes at file level lie end to end (§4.2.2), so a reader can start
    /// at any of them, as after a seek: the boxes before it are not read.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::BoxType;
    /// use isobmff::framing::{BoxEvent, BoxReader};
    ///
    /// // The `ftyp` takes the first bytes of the file, which its size gives
    /// let file: &[u8] = include_bytes!("../../tests/data/movie.mp4");
    /// let after_ftyp = u32::from_be_bytes(file[..4].try_into()?);
    ///
    /// let mut reader = BoxReader::starting_at(after_ftyp.into());
    /// let wanted = reader.wanted_input().ok_or("no input wanted")?;
    /// let input = &file[usize::try_from(wanted.offset())?..];
    /// let Some(BoxEvent::Header(header)) = reader.handle_input(wanted.offset(), input)? else {
    ///     return Err("no header".into());
    /// };
    /// assert_eq!(header.box_type(), BoxType::compact(*b"moov"));
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    #[must_use]
    pub const fn starting_at(position: u64) -> Self {
        Self {
            position,
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
    /// failed, and once it skips a box that runs to the end of the file, or
    /// has the head of one: such a box ends only with the file, so a caller
    /// that has not called [`finish`](Self::finish) still calls it then.
    #[must_use]
    pub fn wanted_input(&self) -> Option<ByteRange> {
        if self.at_end_of_file {
            return None;
        }
        let length = match self.progress {
            Progress::Header { .. } | Progress::Gathered { .. } => None,
            Progress::Opened { header, .. } => header.payload_len(),
            Progress::Payload {
                end: None,
                answer: Answer::Skip,
                ..
            }
            | Progress::Failed(_) => return None,
            Progress::Payload {
                header,
                end,
                answer: Answer::GatherHead { head_bytes },
                ..
            } => {
                let wanted = self.head_wanted(header, end, head_bytes);
                if wanted == 0 && end.is_none() {
                    return None;
                }
                Some(wanted).filter(|&wanted| wanted > 0)
            }
            Progress::Payload { end, .. } => self.remaining(end).filter(|&remaining| remaining > 0),
        };
        Some(ByteRange::new(self.position, length))
    }

    /// Takes `input`, the bytes of the file from file offset `offset`, and returns what they hold
    ///
    /// `input` is taken from the offset [`wanted_input`](Self::wanted_input)
    /// names, which it must start at or before. Bytes before that offset are
    /// passed over, so a caller that cannot seek can pass the file on as it
    /// comes: an input that ends before the wanted offset holds no event.
    /// Bytes passed again must be the bytes passed before.
    ///
    /// A call reads up to the next event and returns it, or `None` once the
    /// input holds no more or the reader wants no more input; the caller
    /// passes the same input again while events come. The event borrows
    /// `input` alone, so the caller answers a [`BoxEvent::Header`] on the
    /// reader before the next call. A
    /// [`BoxEvent::Payload`] is the rest of the input up to the end of the
    /// payload, and after the [`BoxEvent::End`] of a gathered box,
    /// [`gathered_box`](Self::gathered_box) returns the box until the next call.
    ///
    /// A failure leaves the reader failed: it wants no more input, and every
    /// call after it returns the same failure.
    ///
    /// # Errors
    ///
    /// * [`UnwantedInput`](DemuxError::UnwantedInput): `input` starts past the
    ///   wanted offset. The reader is left as it was.
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
    ) -> Result<Option<BoxEvent<'a>>, DemuxError> {
        self.check_answered()?;
        let Some(wanted) = self.wanted_input() else {
            return Ok(None);
        };
        let Some(passed_over) = wanted.offset().checked_sub(offset) else {
            return Err(DemuxError::UnwantedInput { offset, wanted });
        };
        self.leave_gathered();
        let Some(rest) = usize::try_from(passed_over)
            .ok()
            .and_then(|passed_over| input.get(passed_over..))
            .filter(|rest| !rest.is_empty())
        else {
            return Ok(None);
        };
        self.position = offset.saturating_add(passed_over);
        self.read(rest)
    }

    /// Ends the file at the offset the reader wants, and returns the event that closes the box being read
    ///
    /// The event is a [`BoxEvent::End`], or `None` when the file ends between
    /// boxes. A box whose size runs to the end of the file ends here, and so
    /// may one whose payload has nothing left to arrive, so the event goes
    /// where those of [`handle_input`](Self::handle_input) go. A skipped box
    /// ends where its header says. The reader then wants no more input.
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
    pub fn finish(&mut self) -> Result<Option<BoxEvent<'static>>, DemuxError> {
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

    /// Gathers the first `length_bytes` of the payload of the box just opened, or all of a shorter payload, and skips the rest; [`gathered_head`](Self::gathered_head) returns them after its [`BoxEvent::End`]
    ///
    /// The reader wants the bytes of the head alone, and next the file offset
    /// just past the box. A box whose size runs to the end of the file ends
    /// at [`finish`](Self::finish), and the reader wants no more input once
    /// it has the head.
    ///
    /// # Errors
    ///
    /// * [`NoOpenBox`](DemuxError::NoOpenBox): no header waits for an answer.
    /// * The failure the reader failed with before.
    pub fn gather_head(&mut self, length_bytes: u64) -> Result<(), DemuxError> {
        let (header, offset) = self.open_header()?;
        let head_bytes = header
            .payload_len()
            .map_or(length_bytes, |payload_len| payload_len.min(length_bytes));
        // Why not reserve: a head asked as long as an address reaches would abort the allocation, and a head that falls short only grows.
        if let Ok(head_bytes) = usize::try_from(head_bytes) {
            let _ = self.gathered.try_reserve(head_bytes);
        }
        self.answer(header, offset, Answer::GatherHead { head_bytes });
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
            Progress::Header { .. } | Progress::Gathered { .. } | Progress::Failed(_) => None,
        }
    }

    /// Returns the box just gathered, from its [`BoxEvent::End`] until the next call to [`handle_input`](Self::handle_input) or [`finish`](Self::finish)
    #[must_use]
    pub fn gathered_box(&self) -> Option<RawBox<'_>> {
        if !matches!(self.progress, Progress::Gathered { whole: true, .. }) {
            return None;
        }
        RawBox::split_first(&self.gathered)
            .ok()
            .map(|(raw_box, _)| raw_box)
    }

    /// Returns the header of the box [`gather_head`](Self::gather_head) gathered the head of, and that head, from its [`BoxEvent::End`] until the next call to [`handle_input`](Self::handle_input) or [`finish`](Self::finish)
    #[must_use]
    pub fn gathered_head(&self) -> Option<(BoxHeader, &[u8])> {
        let Progress::Gathered {
            header,
            whole: false,
            ..
        } = self.progress
        else {
            return None;
        };
        let head = self
            .gathered
            .get(usize::try_from(header.encoded_len()).ok()?..)?;
        Some((header, head))
    }

    /// Returns where the box [`gathered_box`](Self::gathered_box) or [`gathered_head`](Self::gathered_head) returns lies in the file, header included
    ///
    /// The length is `None` for a box whose head was gathered and whose size
    /// runs to the end of the file.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::ByteRange;
    /// use isobmff::framing::{BoxEvent, BoxReader};
    ///
    /// let file = b"\0\0\0\x0cfreeABCD";
    /// let mut reader = BoxReader::new();
    /// assert!(matches!(reader.handle_input(0, file)?, Some(BoxEvent::Header(_))));
    /// reader.gather(1024)?;
    /// assert_eq!(reader.handle_input(0, file)?, Some(BoxEvent::End));
    /// assert_eq!(reader.gathered_box_range(), Some(ByteRange::new(0, Some(12))));
    /// # Ok::<(), isobmff::DemuxError>(())
    /// ```
    #[must_use]
    pub fn gathered_box_range(&self) -> Option<ByteRange> {
        let Progress::Gathered {
            header,
            offset,
            whole,
        } = self.progress
        else {
            return None;
        };
        let length = header
            .size()
            .total_bytes()
            .or_else(|| whole.then(|| to_u64(self.gathered.len())));
        Some(ByteRange::new(offset, length))
    }

    /// Refuses a call while the reader has failed or waits for a header to be answered
    fn check_answered(&self) -> Result<(), DemuxError> {
        match self.progress {
            Progress::Failed(failure) => Err(failure),
            Progress::Opened { header, .. } => Err(DemuxError::UndecidedBox {
                box_type: header.box_type(),
            }),
            Progress::Header { .. } | Progress::Payload { .. } | Progress::Gathered { .. } => {
                Ok(())
            }
        }
    }

    /// Returns the header waiting for an answer, and the file offset of its box
    fn open_header(&self) -> Result<(BoxHeader, u64), DemuxError> {
        match self.progress {
            Progress::Opened { header, offset } => Ok((header, offset)),
            Progress::Failed(failure) => Err(failure),
            Progress::Header { .. } | Progress::Payload { .. } | Progress::Gathered { .. } => {
                Err(DemuxError::NoOpenBox)
            }
        }
    }

    fn answer(&mut self, header: BoxHeader, offset: u64, answer: Answer) {
        let end = header
            .size()
            .total_bytes()
            .map(|total| offset.saturating_add(total));
        // Why not leave a head of no bytes to `read_payload`: it moves past the payload only as input comes, and the reader would want the payload it skips.
        if let (Answer::Skip | Answer::GatherHead { head_bytes: 0 }, Some(end)) = (answer, end) {
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
        if let Progress::Gathered { .. } = self.progress {
            self.gathered.clear();
            self.progress = Progress::Header {
                needed: SHORTEST_HEADER,
            };
        }
    }

    /// Fails the reader, which then reads no event
    fn fail<'a>(&mut self, failure: DemuxError) -> Option<BoxEvent<'a>> {
        self.progress = Progress::Failed(failure);
        None
    }

    /// Returns how many bytes of the payload of the box of `header` are gathered
    fn gathered_payload_bytes(&self, header: BoxHeader) -> u64 {
        to_u64(self.gathered.len()).saturating_sub(header.encoded_len())
    }

    /// Returns how many bytes the reader wants next of the head of `head_bytes` of the payload of the box of `header`, up to the file offset `end` or, for `None`, the end of the file
    fn head_wanted(&self, header: BoxHeader, end: Option<u64>, head_bytes: u64) -> u64 {
        let head_left = head_bytes.saturating_sub(self.gathered_payload_bytes(header));
        self.remaining(end)
            .map_or(head_left, |remaining| remaining.min(head_left))
    }

    /// Returns the bytes from the reader's position up to the file offset `end`, or `None` up to the end of the file
    fn remaining(&self, end: Option<u64>) -> Option<u64> {
        end.map(|end| end.saturating_sub(self.position))
    }

    /// Takes from the front of `input` as far as the next event, and returns it, or the failure the reader fails with
    fn read<'a>(&mut self, mut input: &'a [u8]) -> Result<Option<BoxEvent<'a>>, DemuxError> {
        let event = match self.progress {
            Progress::Header { needed } => self.read_header(&mut input, needed),
            Progress::Payload {
                header,
                offset,
                end,
                answer,
            } => self.read_payload(&mut input, header, offset, end, answer),
            Progress::Opened { .. } | Progress::Gathered { .. } | Progress::Failed(_) => None,
        };
        if let Progress::Failed(failure) = self.progress {
            return Err(failure);
        }
        Ok(event)
    }

    /// Takes the next header bytes off `input`, and opens the box once the header is whole
    fn read_header<'a>(&mut self, input: &mut &'a [u8], mut needed: u64) -> Option<BoxEvent<'a>> {
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
                return None;
            };
            self.gathered.extend_from_slice(bytes);
            if to_u64(self.gathered.len()) < needed {
                continue;
            }
            match BoxHeader::decode(&self.gathered) {
                Ok((header, _)) => {
                    let offset = self.position.saturating_sub(to_u64(self.gathered.len()));
                    self.progress = Progress::Opened { header, offset };
                    return Some(BoxEvent::Header(header));
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
        offset: u64,
        end: Option<u64>,
        answer: Answer,
    ) -> Option<BoxEvent<'a>> {
        match answer {
            Answer::Skip => {}
            Answer::Stream => {
                if let Some(bytes) = self.take(input, self.remaining(end)) {
                    return Some(BoxEvent::Payload(bytes));
                }
            }
            Answer::GatherHead { head_bytes } => {
                let wanted = self.head_wanted(header, end, head_bytes);
                if let Some(bytes) = self.take(input, Some(wanted)) {
                    self.gathered.extend_from_slice(bytes);
                }
                if let Some(end) = end.filter(|_| self.gathered_payload_bytes(header) >= head_bytes)
                {
                    self.position = end;
                }
            }
            Answer::Gather { limit_bytes } => {
                let gathered_bytes = self.gathered_payload_bytes(header);
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
            if let Answer::Gather { .. } | Answer::GatherHead { .. } = answer {
                self.progress = Progress::Gathered {
                    header,
                    offset,
                    whole: matches!(answer, Answer::Gather { .. }),
                };
                return Some(BoxEvent::End);
            }
            self.gathered.clear();
            self.progress = Progress::Header {
                needed: SHORTEST_HEADER,
            };
            return Some(BoxEvent::End);
        }
        if self.at_end_of_file {
            let needed_bytes = header.size().total_bytes().unwrap_or(u64::MAX);
            return self.fail(DemuxError::UnfinishedBox {
                box_type: header.box_type(),
                needed_bytes,
                available_bytes: needed_bytes.saturating_sub(remaining.unwrap_or(0)),
            });
        }
        None
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
        GatherHead(u64),
        Stream,
        Skip,
    }

    /// Event as a caller records it, the payload pieces of one box joined
    #[derive(PartialEq, Eq, Debug)]
    enum Recorded {
        Header(BoxHeader, Option<ByteRange>),
        Payload(Vec<u8>),
        Gathered(BoxHeader, Vec<u8>),
        /// Header, place and head of a box whose head was gathered
        Head(BoxHeader, Option<ByteRange>, Vec<u8>),
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
            while let Some(event) = reader.handle_input(wanted.offset(), input)? {
                record(&mut reader, event, answer, &mut recorded)?;
            }
        }
        if let Some(event) = reader.finish()? {
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
                    Answer::GatherHead(length_bytes) => reader.gather_head(length_bytes)?,
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
                } else if let Some((header, head)) = reader.gathered_head() {
                    recorded.push(Recorded::Head(
                        header,
                        reader.gathered_box_range(),
                        head.to_vec(),
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
    fn the_head_of_a_box_of_every_header_form_arrives_as_far_as_it_is_asked_wherever_the_input_is_cut()
     {
        for file in every_header_form() {
            let (header, byte_range) = header_and_range(&file);
            for (length_bytes, head) in
                [(3, PAYLOAD.get(..3).unwrap()), (100, PAYLOAD), (0, &[][..])]
            {
                for piece_len in every_cut(&file) {
                    assert_eq!(
                        read(&file, piece_len, Answer::GatherHead(length_bytes)),
                        Ok(vec![
                            Recorded::Header(header, byte_range),
                            Recorded::Head(header, byte_range, head.to_vec()),
                            Recorded::End,
                        ])
                    );
                }
            }
        }
    }

    #[test]
    fn the_rest_of_a_payload_past_its_head_is_not_wanted() {
        let file = [b"\0\0\0\x10free".as_slice(), PAYLOAD, b"\0\0\0\x08skip"].concat();
        let mut reader = BoxReader::new();
        reader.handle_input(0, file.get(..8).unwrap()).unwrap();
        reader.gather_head(3).unwrap();
        let head_wanted = reader.wanted_input();
        let end = reader.handle_input(8, file.get(8..11).unwrap());
        let head = reader
            .gathered_head()
            .map(|(header, head)| (header.box_type(), head.to_vec()));
        let whole = reader.gathered_box().is_some();

        assert_eq!(
            (head_wanted, end, head, whole, reader.wanted_input()),
            (
                Some(ByteRange::new(8, Some(3))),
                Ok(Some(BoxEvent::End)),
                Some((
                    BoxType::compact(*b"free"),
                    PAYLOAD.get(..3).unwrap().to_vec()
                )),
                false,
                Some(ByteRange::new(16, None))
            )
        );
    }

    #[test]
    fn a_head_of_no_bytes_wants_the_file_offset_just_past_the_box() {
        let file = [b"\0\0\0\x10free".as_slice(), PAYLOAD, b"\0\0\0\x08skip"].concat();
        let next_box = file.get(16..).unwrap();
        let mut reader = BoxReader::new();
        reader.handle_input(0, file.get(..8).unwrap()).unwrap();
        reader.gather_head(0).unwrap();
        let wanted = reader.wanted_input();
        let end = reader.handle_input(16, next_box);
        let head = reader.gathered_head().map(|(_, head)| head.to_vec());

        assert_eq!(
            (wanted, end, head, reader.handle_input(16, next_box)),
            (
                Some(ByteRange::new(16, None)),
                Ok(Some(BoxEvent::End)),
                Some(Vec::new()),
                Ok(Some(BoxEvent::Header(
                    BoxHeader::decode(next_box).unwrap().0
                )))
            )
        );
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
        while let Some(event) = reader.handle_input(0, &file).unwrap() {
            record(&mut reader, event, Answer::Stream, &mut recorded).unwrap();
        }
        if let Some(event) = reader.finish().unwrap() {
            record(&mut reader, event, Answer::Stream, &mut recorded).unwrap();
        }
        assert_eq!(Ok(recorded), read(&file, |_| 1, Answer::Stream));
    }

    #[test]
    fn input_ending_before_the_wanted_offset_holds_no_events() {
        let mut reader = BoxReader::new();
        if let Some(BoxEvent::Header(_)) = reader.handle_input(0, b"\0\0\0\x10freeAAAA").unwrap() {
            reader.skip().unwrap();
        }
        let wanted = reader.wanted_input();
        assert_eq!(reader.handle_input(8, b"AAAA"), Ok(None));
        assert_eq!(reader.handle_input(12, b"AAAA"), Ok(None));
        assert_eq!(reader.wanted_input(), wanted);
        let input = b"AAAA\0\0\0\x08skip";
        assert_eq!(reader.handle_input(12, input), Ok(Some(BoxEvent::End)));
        assert_eq!(
            reader.handle_input(12, input),
            Ok(Some(BoxEvent::Header(
                BoxHeader::decode(b"\0\0\0\x08skip").unwrap().0
            )))
        );
    }

    #[test]
    fn a_skipped_box_moves_the_wanted_input_past_its_end() {
        let mut reader = BoxReader::new();
        if let Some(BoxEvent::Header(_)) = reader.handle_input(0, b"\0\0\0\x10freeAAAA").unwrap() {
            reader.skip().unwrap();
        }
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(16, None)));
    }

    #[test]
    fn a_skipped_box_running_to_the_end_of_the_file_leaves_no_input_wanted() {
        let mut reader = BoxReader::new();
        if let Some(BoxEvent::Header(_)) = reader.handle_input(0, b"\0\0\0\0mdatAAAA").unwrap() {
            reader.skip().unwrap();
        }
        assert_eq!(reader.wanted_input(), None);
    }

    #[test]
    fn a_payload_declared_past_the_gather_limit_is_refused_and_the_box_stays_open() {
        let mut reader = BoxReader::new();
        assert!(matches!(
            reader.handle_input(0, b"\0\0\0\x10freeAAAAAAAA"),
            Ok(Some(BoxEvent::Header(_)))
        ));
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
    fn a_failure_is_returned_by_the_call_that_reads_it_after_the_events_before_it() {
        let mut reader = BoxReader::new();
        assert!(matches!(
            reader.handle_input(0, b"\0\0\0\x0cfree"),
            Ok(Some(BoxEvent::Header(_)))
        ));
        reader.stream().unwrap();
        let failure = DemuxError::Box {
            error: Error::size_below_header(8, 4),
        };
        let input = b"AAAA\0\0\0\x04free";
        assert_eq!(
            reader.handle_input(8, input),
            Ok(Some(BoxEvent::Payload(b"AAAA")))
        );
        assert_eq!(reader.handle_input(8, input), Ok(Some(BoxEvent::End)));
        assert_eq!(reader.handle_input(8, input), Err(failure));
        assert_eq!(reader.wanted_input(), None);
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn a_file_ending_inside_a_header_is_unfinished() {
        let mut reader = BoxReader::new();
        assert_eq!(reader.handle_input(0, b"\0\0\0\x01mdat\0\0"), Ok(None));
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
        assert_eq!(reader.finish(), Ok(None));
        assert_eq!(reader.wanted_input(), None);
    }

    #[test]
    fn input_past_the_wanted_offset_is_refused_and_leaves_the_reader_as_it_was() {
        let mut reader = BoxReader::new();
        assert_eq!(
            reader.handle_input(1, b"\0\0\x08free").err(),
            Some(DemuxError::UnwantedInput {
                offset: 1,
                wanted: ByteRange::new(0, None),
            })
        );
        assert!(matches!(
            reader.handle_input(0, b"\0\0\0\x08free"),
            Ok(Some(BoxEvent::Header(_)))
        ));
    }

    #[test]
    fn empty_input_leaves_the_reader_as_it_was() {
        let mut reader = BoxReader::new();
        assert!(matches!(
            reader.handle_input(0, b"\0\0\0\x0cfree"),
            Ok(Some(BoxEvent::Header(_)))
        ));
        reader.stream().unwrap();
        let wanted = reader.wanted_input();
        assert_eq!(reader.handle_input(8, &[]), Ok(None));
        assert_eq!(reader.wanted_input(), wanted);
    }

    #[test]
    fn input_before_the_header_is_answered_is_refused() {
        let mut reader = BoxReader::new();
        assert!(matches!(
            reader.handle_input(0, b"\0\0\0\x0cfree"),
            Ok(Some(BoxEvent::Header(_)))
        ));
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
        assert!(matches!(
            reader.handle_input(0, b"\0\0\0\x0cfree"),
            Ok(Some(BoxEvent::Header(_)))
        ));
        reader.stream().unwrap();
        assert_eq!(reader.gather(8), Err(DemuxError::NoOpenBox));
    }

    #[test]
    fn a_gathered_box_is_returned_until_the_next_input() {
        let file = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
        let mut reader = BoxReader::new();
        assert!(matches!(
            reader.handle_input(0, file),
            Ok(Some(BoxEvent::Header(_)))
        ));
        reader.gather(8).unwrap();
        assert_eq!(reader.handle_input(0, file), Ok(Some(BoxEvent::End)));
        assert_eq!(
            reader.gathered_box().map(|raw_box| raw_box.payload()),
            Some(b"AAAA".as_slice())
        );
        assert!(matches!(
            reader.handle_input(0, file),
            Ok(Some(BoxEvent::Header(_)))
        ));
        assert_eq!(reader.gathered_box(), None);
    }
}
