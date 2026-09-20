//! [`WholeBoxReader`], one box gathered whole out of the steps it was framed into and read into a value

use alloc::vec::Vec;
use core::marker::PhantomData;

use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader};

use crate::StructureError;

/// Reads one box into a value, gathering its payload whole out of the steps it arrives in
///
/// The framing of a file reports a box as its header, its payload cut as the
/// input was, then its end. A reader is begun on the header, handed each cut
/// of the payload, and finished at the end, where it reads what it gathered
/// into a `Value`; it is one box's, and is used up by reading it. What a box
/// may declare is bounded, since its payload is memory the reader is about to
/// take. The reader neither frames the file nor settles which box the header
/// introduces: both stay with the caller, which reads the payload as the
/// `Value` it chose.
///
/// # Contract
///
/// * A box declaring more payload than the limit is
///   [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded)
///   at [`begin`](Self::begin), before a byte of it is gathered. A box
///   declaring no total is gathered as far as the limit and refused the same
///   way where it reaches past it.
/// * A payload that does not read as a `Value` is
///   [`Box`](crate::StructureErrorKind::Box), with the box named as the
///   container the failure was reached through.
/// * A failure leaves the reader as it stood: the caller drops it, since the
///   box it was reading is lost.
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
/// // The reader is begun on the header, handed the payload, and finished at the end
/// let mut reader = None;
/// let mut read = None;
/// while let Some(event) = boxes.poll_event() {
///     match event {
///         BoxEvent::Header(header) => {
///             reader = Some(WholeBoxReader::<FileTypeBox>::begin(header, 1_024)?);
///         }
///         BoxEvent::Payload(payload) => reader.as_mut().unwrap().handle_payload(payload)?,
///         BoxEvent::End => read = Some(reader.take().unwrap().finish()?),
///         _later_step => {}
///     }
/// }
///
/// assert_eq!(read, Some(file_type()));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Clone, Debug)]
pub struct WholeBoxReader<Value> {
    payload_limit: u64,
    payload: Vec<u8>,
    value: PhantomData<Value>,
}

impl<Value: BoxDecode + BoxDefinition> WholeBoxReader<Value> {
    /// Begins reading the box `header` introduces, gathering no more than `payload_limit` bytes for it
    ///
    /// # Errors
    ///
    /// * [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded):
    ///   the box declares more payload than `payload_limit`.
    pub fn begin(header: BoxHeader, payload_limit: u64) -> Result<Self, StructureError> {
        if let Some(declared) = header
            .payload_len()
            .filter(|declared| *declared > payload_limit)
        {
            return Err(StructureError::payload_limit_exceeded(
                Value::BOX_TYPE,
                declared,
                payload_limit,
            ));
        }

        Ok(Self {
            payload_limit,
            // Why not reserve the declared length: the file declares it and the
            // limit only bounds it, so reserving would take memory for bytes
            // that may never arrive.
            payload: Vec::new(),
            value: PhantomData,
        })
    }

    /// Takes part of the payload of the box
    ///
    /// # Errors
    ///
    /// * [`PayloadLimitExceeded`](crate::StructureErrorKind::PayloadLimitExceeded):
    ///   a box declaring no total reaches past the limit the reader gathers.
    pub fn handle_payload(&mut self, mut payload: Vec<u8>) -> Result<(), StructureError> {
        // Why not checked_add: the framing cut the payload out of a finite
        // resource, so its length cannot run past what 64 bits carry.
        let reached = (self.payload.len() as u64).saturating_add(payload.len() as u64);
        if reached > self.payload_limit {
            return Err(StructureError::payload_limit_exceeded(
                Value::BOX_TYPE,
                reached,
                self.payload_limit,
            ));
        }
        if self.payload.is_empty() {
            self.payload = payload;
        } else {
            self.payload.append(&mut payload);
        }

        Ok(())
    }

    /// Takes the end of the box, and reads what was gathered into the value it forms
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::StructureErrorKind::Box): the payload does not read
    ///   as a `Value`, with the box named as the container.
    pub fn finish(self) -> Result<Value, StructureError> {
        Value::decode_payload(&self.payload)
            .map_err(|failure| failure.in_container(Value::BOX_TYPE).into())
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

    /// Bytes a box may declare in these tests, unless one states its own limit
    const PAYLOAD_LIMIT: u64 = 1_024;

    /// The values read out of the steps `file` frames into, handed over `cut_length` bytes at a time
    fn read_whole<Value: super::BoxDecode + BoxDefinition>(
        file: &[u8],
        cut_length: usize,
    ) -> Result<Vec<Value>, StructureError> {
        let mut reader = None;
        let mut read = Vec::new();

        for (_extent, event) in events_of(file, cut_length).unwrap() {
            match event {
                BoxEvent::Header(header) => {
                    reader = Some(WholeBoxReader::<Value>::begin(header, PAYLOAD_LIMIT)?);
                }
                BoxEvent::Payload(payload) => reader.as_mut().unwrap().handle_payload(payload)?,
                BoxEvent::End => read.push(reader.take().unwrap().finish()?),
                _later_step => {}
            }
        }

        Ok(read)
    }

    #[test]
    fn a_box_is_read_the_same_however_its_payload_was_cut() {
        let file = written(&file_type());

        for cut_length in [1, 3, 7, file.len()] {
            assert_eq!(
                read_whole::<FileTypeBox>(&file, cut_length),
                Ok(vec![file_type()])
            );
        }
    }

    #[test]
    fn one_box_after_another_is_read_by_a_reader_each() {
        let file = [written(&file_type()), written(&file_type())].concat();

        assert_eq!(
            read_whole::<FileTypeBox>(&file, file.len()),
            Ok(vec![file_type(), file_type()])
        );
    }

    #[test]
    fn a_box_declaring_a_payload_past_the_limit_is_refused_before_it_is_gathered() {
        let header = BoxHeader::with_payload_len(FileTypeBox::BOX_TYPE, 16).unwrap();

        assert_eq!(
            WholeBoxReader::<FileTypeBox>::begin(header, 4).map(drop),
            Err(StructureError::payload_limit_exceeded(
                FileTypeBox::BOX_TYPE,
                16,
                4
            ))
        );
    }

    #[test]
    fn a_box_declaring_no_total_is_refused_where_it_reaches_past_the_limit() {
        let header = BoxHeader::new(FileTypeBox::BOX_TYPE, BoxSize::ToEndOfFile).unwrap();
        let mut reader = WholeBoxReader::<FileTypeBox>::begin(header, 4).unwrap();

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
    fn a_payload_that_does_not_read_as_the_box_names_that_box() {
        let header = BoxHeader::with_payload_len(MovieBox::BOX_TYPE, 4).unwrap();
        let mut reader = WholeBoxReader::<MovieBox>::begin(header, PAYLOAD_LIMIT).unwrap();

        reader.handle_payload(b"AAAA".to_vec()).unwrap();

        assert_eq!(
            reader.finish().map_err(|failure| failure
                .box_error()
                .map(|box_error| box_error.containers().collect::<Vec<_>>())),
            Err(Some(vec![FourCC::new(*b"moov")]))
        );
    }
}
