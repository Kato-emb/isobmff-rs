//! [`WholeBoxReader`], one box gathered whole out of the steps it was framed into and read into a value

use alloc::vec::Vec;
use core::marker::PhantomData;
use core::mem;

use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader};

use crate::StructureError;

/// Reads a box into a value, gathering its payload whole out of the steps it arrives in
///
/// The framing of a file reports a box as its header, its payload cut as the
/// input was, then its end. This reader takes those steps for one box of type
/// `Value`, holds the payload until the box ends, and reads it into a `Value`
/// there, so a caller hands over steps and takes a box. What a box may declare
/// is bounded, since its payload is memory the reader is about to take. The
/// reader frames nothing and settles which boxes to read into which type
/// nothing: both stay with the caller.
///
/// # Contract
///
/// * A box is [`handle_header`](Self::handle_header), then
///   [`handle_payload`](Self::handle_payload) as many times as its payload
///   was cut into, then [`handle_end`](Self::handle_end), which returns the
///   value. The reader then waits for the next box. A header handed over
///   while a box is open is
///   [`BoxStillOpen`](crate::StructureErrorKind::BoxStillOpen), and payload
///   or an end handed over while none is open is
///   [`NoBoxOpen`](crate::StructureErrorKind::NoBoxOpen).
/// * A header of another type than `Value` names is refused as
///   [`BoxTypeMismatch`](isobmff_core::ErrorKind::BoxTypeMismatch), carried
///   on [`Box`](crate::StructureErrorKind::Box).
/// * A box declaring more payload than the limit is
///   [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded)
///   at its header, before a byte of it is gathered. A box declaring no total
///   is gathered as far as the limit and refused the same way where it
///   reaches past it — see [`with_payload_limit`](Self::with_payload_limit).
/// * A payload that does not read as a `Value` is
///   [`Box`](crate::StructureErrorKind::Box), with the box named as the
///   container the failure was reached through.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again.
/// * [`finish`](Self::finish) declares the boxes over, and fails if one is
///   still open. Anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::{BoxEvent, BoxReader, FileTypeBox, WholeBoxReader};
/// # use isobmff_test_support::{file_type, written};
///
/// // The steps of an `ftyp` as the framing reports them, cut anywhere
/// let mut boxes = BoxReader::new();
/// boxes.handle_input(&written(&file_type()))?;
/// boxes.finish()?;
///
/// // Each step is handed over, and the end of the box returns the value
/// let mut reader = WholeBoxReader::<FileTypeBox>::new();
/// let mut read = None;
/// while let Some(event) = boxes.poll_event() {
///     match event {
///         BoxEvent::Header(header) => reader.handle_header(header)?,
///         BoxEvent::Payload(payload) => reader.handle_payload(payload)?,
///         BoxEvent::End => read = Some(reader.handle_end()?),
///         _later_step => {}
///     }
/// }
/// reader.finish()?;
///
/// assert_eq!(read, Some(file_type()));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Clone, Debug)]
pub struct WholeBoxReader<Value> {
    payload_limit: u64,
    state: State,
    value: PhantomData<Value>,
}

/// Where the reader stands between calls
#[derive(Clone, Debug)]
enum State {
    /// Between boxes, waiting for the header of the next one
    Between,
    /// Gathering the payload of the box that started
    Gathering(Vec<u8>),
    /// Told the boxes are over, and taking no more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(StructureError),
}

impl<Value: BoxDecode + BoxDefinition> WholeBoxReader<Value> {
    /// Payload a box may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading files whose `moov` reaches past that
    /// — a presentation of many tracks states a sample entry for each — names a
    /// limit of its own with [`with_payload_limit`](Self::with_payload_limit).
    pub const DEFAULT_PAYLOAD_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a reader waiting for the header of a box
    ///
    /// What a box may declare is bounded by
    /// [`DEFAULT_PAYLOAD_LIMIT`](Self::DEFAULT_PAYLOAD_LIMIT).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_payload_limit(Self::DEFAULT_PAYLOAD_LIMIT)
    }

    /// Creates a reader gathering no more than `payload_limit` bytes for one box
    ///
    /// A box is gathered whole before it is read, so the payload it declares is
    /// memory the reader is about to take. A box declaring more than
    /// `payload_limit` bytes is
    /// [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded)
    /// at its header, refused before a byte of it is gathered; one declaring no
    /// total is gathered until it reaches past the limit, and refused the same
    /// way there.
    ///
    /// The limit bounds one box rather than the file: it is checked against
    /// what each box reaches, not against what the boxes before it reached
    /// between them.
    #[must_use]
    pub const fn with_payload_limit(payload_limit: u64) -> Self {
        Self {
            payload_limit,
            state: State::Between,
            value: PhantomData,
        }
    }

    /// Takes the header of a box, and opens it for its payload
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::StructureErrorKind::Box): the header names another
    ///   box than `Value`, as
    ///   [`BoxTypeMismatch`](isobmff_core::ErrorKind::BoxTypeMismatch).
    /// * [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded):
    ///   the box declares more payload than the reader gathers.
    /// * [`BoxStillOpen`](crate::StructureErrorKind::BoxStillOpen): the box
    ///   before it was not ended.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   boxes were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_header(&mut self, header: BoxHeader) -> Result<(), StructureError> {
        self.reading()?;
        if matches!(self.state, State::Gathering(_)) {
            return Err(self.fail(StructureError::box_still_open(Value::BOX_TYPE)));
        }

        let found = header.box_type();
        if found != Value::BOX_TYPE {
            return Err(
                self.fail(isobmff_core::Error::box_type_mismatch(Value::BOX_TYPE, found).into())
            );
        }
        if let Some(declared) = header.payload_len() {
            self.within_limit(declared)?;
        }
        // Why not reserve the declared length: the file declares it and the
        // limit only bounds it, so reserving would take memory for bytes that
        // may never arrive.
        self.state = State::Gathering(Vec::new());

        Ok(())
    }

    /// Takes part of the payload of the box that is open
    ///
    /// # Errors
    ///
    /// * [`NoBoxOpen`](crate::StructureErrorKind::NoBoxOpen): no box was
    ///   opened to carry it.
    /// * [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded):
    ///   a box declaring no total reaches past the limit the reader gathers.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   boxes were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_payload(&mut self, mut payload: Vec<u8>) -> Result<(), StructureError> {
        self.reading()?;
        let State::Gathering(mut gathered) = mem::replace(&mut self.state, State::Between) else {
            return Err(self.fail(StructureError::no_box_open()));
        };

        // Why not checked_add: the framing cut the payload out of a finite
        // resource, so its length cannot run past what 64 bits carry.
        let reached = (gathered.len() as u64).saturating_add(payload.len() as u64);
        self.within_limit(reached)?;
        if gathered.is_empty() {
            gathered = payload;
        } else {
            gathered.append(&mut payload);
        }
        self.state = State::Gathering(gathered);

        Ok(())
    }

    /// Takes the end of the box that is open, and reads it into the value it forms
    ///
    /// # Errors
    ///
    /// * [`NoBoxOpen`](crate::StructureErrorKind::NoBoxOpen): no box was open
    ///   to end.
    /// * [`Box`](crate::StructureErrorKind::Box): the payload does not read
    ///   as a `Value`, with the box named as the container.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   boxes were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_end(&mut self) -> Result<Value, StructureError> {
        self.reading()?;
        let State::Gathering(payload) = mem::replace(&mut self.state, State::Between) else {
            return Err(self.fail(StructureError::no_box_open()));
        };

        Value::decode_payload(&payload)
            .map_err(|failure| self.fail(failure.in_container(Value::BOX_TYPE).into()))
    }

    /// Declares the boxes over
    ///
    /// # Errors
    ///
    /// * [`BoxStillOpen`](crate::StructureErrorKind::BoxStillOpen): a box was
    ///   left open.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   boxes were already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), StructureError> {
        self.reading()?;
        if matches!(self.state, State::Gathering(_)) {
            return Err(self.fail(StructureError::box_still_open(Value::BOX_TYPE)));
        }
        self.state = State::Finished;

        Ok(())
    }

    /// Returns `Ok` while the reader still takes the steps of a box
    const fn reading(&self) -> Result<(), StructureError> {
        match self.state {
            State::Between | State::Gathering(_) => Ok(()),
            State::Finished => Err(StructureError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Returns `Ok` while `reached` bytes of payload stay within the limit
    fn within_limit(&mut self, reached: u64) -> Result<(), StructureError> {
        if reached > self.payload_limit {
            return Err(self.fail(StructureError::payload_limit_exceeded(
                Value::BOX_TYPE,
                reached,
                self.payload_limit,
            )));
        }

        Ok(())
    }

    /// Fails the reader for good, and hands the failure back to report
    fn fail(&mut self, failure: StructureError) -> StructureError {
        self.state = State::Failed(failure);

        failure
    }
}

impl<Value: BoxDecode + BoxDefinition> Default for WholeBoxReader<Value> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::{FileTypeBox, MovieBox};
    use isobmff_core::{BoxHeader, BoxSize, FourCC};
    use isobmff_sequence::BoxEvent;
    use isobmff_test_support::{events_of, file_type, written};

    use super::{BoxDefinition, StructureError, WholeBoxReader};
    use crate::StructureErrorKind;

    /// Header of an `ftyp` declaring `payload_len` bytes of payload
    fn file_type_header(payload_len: u64) -> BoxHeader {
        BoxHeader::with_payload_len(FileTypeBox::BOX_TYPE, payload_len).unwrap()
    }

    /// The value read out of the steps `file` frames into, handed over `cut_length` bytes at a time
    fn read_whole<Value: super::BoxDecode + BoxDefinition>(
        file: &[u8],
        cut_length: usize,
    ) -> Result<Value, StructureError> {
        let mut reader = WholeBoxReader::<Value>::new();
        let mut read = None;

        for (_extent, event) in events_of(file, cut_length).unwrap() {
            match event {
                BoxEvent::Header(header) => reader.handle_header(header)?,
                BoxEvent::Payload(payload) => reader.handle_payload(payload)?,
                BoxEvent::End => read = Some(reader.handle_end()?),
                _later_step => {}
            }
        }
        reader.finish()?;

        Ok(read.unwrap())
    }

    #[test]
    fn a_box_is_read_the_same_however_its_payload_was_cut() {
        let file = written(&file_type());

        for cut_length in [1, 3, 7, file.len()] {
            assert_eq!(
                read_whole::<FileTypeBox>(&file, cut_length),
                Ok(file_type())
            );
        }
    }

    #[test]
    fn a_box_declaring_a_payload_past_the_limit_is_refused_before_it_is_gathered() {
        let mut reader = WholeBoxReader::<FileTypeBox>::with_payload_limit(4);

        assert_eq!(
            reader.handle_header(file_type_header(16)),
            Err(StructureError::payload_limit_exceeded(
                FileTypeBox::BOX_TYPE,
                16,
                4
            ))
        );
    }

    #[test]
    fn a_box_declaring_no_total_is_refused_where_it_reaches_past_the_limit() {
        let mut reader = WholeBoxReader::<FileTypeBox>::with_payload_limit(4);
        let header = BoxHeader::new(FileTypeBox::BOX_TYPE, BoxSize::ToEndOfFile).unwrap();

        reader.handle_header(header).unwrap();
        reader.handle_payload(vec![0; 3]).unwrap();

        assert_eq!(
            reader.handle_payload(vec![0; 2]),
            Err(StructureError::payload_limit_exceeded(
                FileTypeBox::BOX_TYPE,
                5,
                4
            ))
        );
    }

    #[test]
    fn a_header_of_another_box_is_refused() {
        let mut reader = WholeBoxReader::<MovieBox>::new();

        assert_eq!(
            reader.handle_header(file_type_header(16)),
            Err(StructureError::from(
                isobmff_core::Error::box_type_mismatch(MovieBox::BOX_TYPE, FileTypeBox::BOX_TYPE)
            ))
        );
    }

    #[test]
    fn a_payload_that_does_not_read_as_the_box_names_that_box() {
        let mut reader = WholeBoxReader::<MovieBox>::new();

        reader
            .handle_header(BoxHeader::with_payload_len(MovieBox::BOX_TYPE, 4).unwrap())
            .unwrap();
        reader.handle_payload(b"AAAA".to_vec()).unwrap();

        assert_eq!(
            reader.handle_end().map_err(|failure| failure
                .box_error()
                .map(|box_error| box_error.containers().collect::<Vec<_>>())),
            Err(Some(vec![FourCC::new(*b"moov")]))
        );
    }

    #[test]
    fn a_step_of_a_box_handed_over_while_none_is_open_is_rejected() {
        let mut reader = WholeBoxReader::<FileTypeBox>::new();

        assert_eq!(
            reader.handle_payload(b"AAAA".to_vec()),
            Err(StructureError::no_box_open())
        );

        let mut reader = WholeBoxReader::<FileTypeBox>::new();

        assert_eq!(
            reader.handle_end().map(drop),
            Err(StructureError::no_box_open())
        );
    }

    #[test]
    fn a_header_handed_over_while_a_box_is_open_is_rejected() {
        let mut reader = WholeBoxReader::<FileTypeBox>::new();

        reader.handle_header(file_type_header(16)).unwrap();

        assert_eq!(
            reader.handle_header(file_type_header(16)),
            Err(StructureError::box_still_open(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn the_boxes_declared_over_while_one_is_open_are_rejected() {
        let mut reader = WholeBoxReader::<FileTypeBox>::new();

        reader.handle_header(file_type_header(16)).unwrap();

        assert_eq!(
            reader.finish(),
            Err(StructureError::box_still_open(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_failed_reader_reports_the_same_failure_for_every_call_after_it() {
        let mut reader = WholeBoxReader::<FileTypeBox>::new();
        let failure = StructureError::no_box_open();

        assert_eq!(reader.handle_payload(Vec::new()), Err(failure));
        assert_eq!(reader.handle_header(file_type_header(16)), Err(failure));
        assert_eq!(reader.handle_end().map(drop), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn a_step_handed_over_after_finishing_is_rejected() {
        let mut reader = WholeBoxReader::<FileTypeBox>::new();

        reader.finish().unwrap();

        assert_eq!(
            reader
                .handle_header(file_type_header(16))
                .map_err(StructureError::kind),
            Err(StructureErrorKind::AlreadyFinished)
        );
        assert_eq!(reader.finish(), Err(StructureError::already_finished()));
    }

    #[test]
    fn a_reader_takes_one_box_after_another() {
        let file = [written(&file_type()), written(&file_type())].concat();
        let mut reader = WholeBoxReader::<FileTypeBox>::new();
        let mut read = Vec::new();

        for (_extent, event) in events_of(&file, file.len()).unwrap() {
            match event {
                BoxEvent::Header(header) => reader.handle_header(header).unwrap(),
                BoxEvent::Payload(payload) => reader.handle_payload(payload).unwrap(),
                BoxEvent::End => read.push(reader.handle_end().unwrap()),
                _later_step => {}
            }
        }

        assert_eq!(read, vec![file_type(), file_type()]);
    }
}
