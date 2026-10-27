//! [`BoxEvent`] and [`OpenBox`], what a [`BoxReader`] reports as it reads

use isobmff_core::{BoxHeader, RawBox};

use super::{Answer, BoxReader, Progress, Step, to_u64};
use crate::byte_range::ByteRange;
use crate::error::DemuxError;

/// What the bytes of a file hold, as a [`BoxReader`] reads them
#[non_exhaustive]
#[derive(Debug)]
pub enum BoxEvent<'a> {
    /// Header of a box, to be answered with what to do with its payload
    Header(OpenBox<'a>),
    /// Piece of the payload of a box being streamed, borrowed from the input
    Payload(&'a [u8]),
    /// Box being gathered, now whole, borrowed from the reader
    Gathered(RawBox<'a>),
    /// Last byte of the box has been read
    End,
}

/// Box whose header has been read, waiting to be told what to do with its payload
///
/// It borrows the reader, which takes no more input until one of
/// [`gather`](Self::gather), [`stream`](Self::stream) or
/// [`skip`](Self::skip) answers it.
#[derive(Debug)]
#[must_use = "the reader takes no more input until the box is gathered, streamed or skipped"]
pub struct OpenBox<'a> {
    reader: &'a mut BoxReader,
    header: BoxHeader,
}

impl OpenBox<'_> {
    /// Returns the header of the box
    #[must_use]
    pub const fn header(&self) -> BoxHeader {
        self.header
    }

    /// Returns where the box lies in the file, header included
    ///
    /// The length is `None` for a box whose size runs to the end of the file.
    #[must_use]
    pub fn byte_range(&self) -> ByteRange {
        let header_bytes = to_u64(self.reader.gathered.len());
        ByteRange::new(
            self.reader.position.saturating_sub(header_bytes),
            self.header.size().total_bytes(),
        )
    }

    /// Gathers the payload into one [`RawBox`], reported as [`BoxEvent::Gathered`] once it is whole
    ///
    /// # Errors
    ///
    /// [`PayloadLimitExceeded`](DemuxError::PayloadLimitExceeded): the
    /// payload the header declares is longer than `limit_bytes`. The reader
    /// fails. A payload that runs to the end of the file fails the same way
    /// once it outgrows the limit.
    pub fn gather(self, limit_bytes: u64) -> Result<(), DemuxError> {
        let payload_len = self.header.payload_len();
        if let Some(payload_len) = payload_len
            && payload_len > limit_bytes
        {
            return Err(self.reader.fail(DemuxError::PayloadLimitExceeded {
                box_type: self.header.box_type(),
                reached_bytes: payload_len,
                limit_bytes,
            }));
        }
        if let Some(payload_len) = payload_len.and_then(|len| usize::try_from(len).ok()) {
            self.reader.gathered.reserve(payload_len);
        }
        self.answer(Answer::Gather { limit_bytes });
        Ok(())
    }

    /// Streams the payload as [`BoxEvent::Payload`]s, each a piece of the input it arrives in
    pub fn stream(self) {
        self.answer(Answer::Stream);
    }

    /// Skips the payload: the reader next wants the file offset just past the box
    ///
    /// A box whose size runs to the end of the file leaves the reader wanting
    /// no more input.
    pub fn skip(self) {
        self.answer(Answer::Skip);
    }

    fn answer(self, answer: Answer) {
        let byte_range = self.byte_range();
        self.reader.progress = Progress::Payload {
            header: self.header,
            end: byte_range
                .length()
                .map(|length| byte_range.offset().saturating_add(length)),
            answer,
        };
    }
}

/// Events the bytes given to a [`BoxReader`] hold, in file order
///
/// Returned by [`BoxReader::handle_input`] and [`BoxReader::finish`].
#[derive(Debug)]
#[must_use = "the reader takes the input only as its events are iterated"]
pub struct BoxEvents<'a> {
    reader: Option<&'a mut BoxReader>,
    input: &'a [u8],
    end_queued: bool,
}

impl<'a> BoxEvents<'a> {
    /// Events `reader` finds in `input`, or none without a reader
    pub(super) const fn new(reader: Option<&'a mut BoxReader>, input: &'a [u8]) -> Self {
        Self {
            reader,
            input,
            end_queued: false,
        }
    }
}

impl<'a> Iterator for BoxEvents<'a> {
    type Item = Result<BoxEvent<'a>, DemuxError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.end_queued {
            self.end_queued = false;
            return Some(Ok(BoxEvent::End));
        }
        let reader = self.reader.take()?;
        match reader.step(&mut self.input) {
            Step::Event(event) => {
                self.reader = Some(reader);
                Some(Ok(event))
            }
            Step::Waiting => None,
            Step::Opened(header) => Some(Ok(BoxEvent::Header(OpenBox { reader, header }))),
            Step::Gathered => {
                self.end_queued = true;
                let reader: &'a BoxReader = reader;
                Some(reader.raw_box().map(BoxEvent::Gathered))
            }
            Step::Failed(failure) => Some(Err(failure)),
        }
    }
}
