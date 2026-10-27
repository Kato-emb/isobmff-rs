//! [`BoxWriter`], which lays the boxes of a file end to end as bytes

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use isobmff_core::{BoxHeader, BoxSize};

use crate::byte_range::ByteRange;
use crate::error::MuxError;

/// Writer that lays boxes end to end as the bytes of a file, ISO/IEC 14496-12 §4.2.2
///
/// The writer does no I/O. Each box given to
/// [`handle_box`](Self::handle_box) becomes one run of bytes, which
/// [`poll_output`](Self::poll_output) hands out in file order.
///
/// A box whose size runs to the end of the file is the last box: the writer
/// refuses any box after it.
///
/// # Examples
///
/// ```
/// use isobmff::framing::BoxWriter;
/// use isobmff::{BoxHeader, BoxSize, BoxType, ByteRange, MuxError};
///
/// let mut writer = BoxWriter::new();
///
/// // A `free` box, its payload given in two pieces
/// let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
/// assert_eq!(writer.handle_box(free, [b"AA".as_slice(), b"AA"])?, ByteRange::new(0, Some(12)));
///
/// // An `mdat` whose size runs to the end of the file, the last box there can be
/// let mdat = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();
/// assert_eq!(writer.handle_box(mdat, [b"samples".as_slice()])?, ByteRange::new(12, Some(15)));
/// writer.finish()?;
///
/// // The bytes of the file, one box at a time
/// assert_eq!(writer.poll_output().unwrap(), b"\0\0\0\x0cfreeAAAA");
/// assert_eq!(writer.poll_output().unwrap(), b"\0\0\0\0mdatsamples");
/// assert_eq!(writer.poll_output(), None);
/// # Ok::<(), MuxError>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct BoxWriter {
    /// File offset of the next box
    position: u64,
    output: VecDeque<Vec<u8>>,
    state: State,
}

/// Which boxes the writer still takes
#[derive(Clone, Copy, Default, Debug)]
enum State {
    /// Any box
    #[default]
    Open,
    /// None: a box whose size runs to the end of the file has been written
    PastEndOfFile,
    /// None: the file is finished
    Finished,
}

impl BoxWriter {
    /// Creates a writer whose first box starts the file
    #[must_use]
    pub const fn new() -> Self {
        Self {
            position: 0,
            output: VecDeque::new(),
            state: State::Open,
        }
    }

    /// Writes the box `header` introduces, with the payload laid out in the pieces `payload` gives, and returns where it lies in the file
    ///
    /// The bytes of the box are output as one run, ready for
    /// [`poll_output`](Self::poll_output).
    ///
    /// # Errors
    ///
    /// Nothing is output and the writer is left as it was when:
    ///
    /// * [`PayloadLengthMismatch`](MuxError::PayloadLengthMismatch): the
    ///   pieces add up to a payload other than the one `header` declares.
    /// * [`PastEndOfFile`](MuxError::PastEndOfFile): a box whose size runs to
    ///   the end of the file has been written.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
    /// * [`Box`](MuxError::Box): the header does not write, as when its
    ///   `extended_type` is one ISO/IEC 14496-12 §4.2.3 reserves.
    pub fn handle_box<'a>(
        &mut self,
        header: BoxHeader,
        payload: impl IntoIterator<Item = &'a [u8], IntoIter: Clone>,
    ) -> Result<ByteRange, MuxError> {
        match self.state {
            State::Open => {}
            State::PastEndOfFile => {
                return Err(MuxError::PastEndOfFile {
                    box_type: header.box_type(),
                });
            }
            State::Finished => return Err(MuxError::AlreadyFinished),
        }
        let pieces = payload.into_iter();
        // Why not size the buffer from the length the header declares: a
        // header declaring more than is offered would size an allocation the
        // check below then refuses. The pieces are walked for their lengths
        // first, and copied only once those match.
        let payload_len = pieces
            .clone()
            .map(<[u8]>::len)
            .fold(0, usize::saturating_add);
        let offered_bytes = u64::try_from(payload_len).unwrap_or(u64::MAX);
        if let Some(declared_bytes) = header.payload_len()
            && offered_bytes != declared_bytes
        {
            return Err(MuxError::PayloadLengthMismatch {
                box_type: header.box_type(),
                declared_bytes,
                offered_bytes,
            });
        }
        let header_len = usize::try_from(header.encoded_len()).unwrap_or(usize::MAX);
        let mut bytes = Vec::with_capacity(header_len.saturating_add(payload_len));
        bytes.resize(header_len, 0);
        header
            .encode(&mut bytes)
            .map_err(|error| MuxError::Box { error })?;
        for piece in pieces {
            bytes.extend_from_slice(piece);
        }
        let total = header.encoded_len().saturating_add(offered_bytes);
        let byte_range = ByteRange::new(self.position, Some(total));
        self.position = self.position.saturating_add(total);
        if header.size() == BoxSize::ToEndOfFile {
            self.state = State::PastEndOfFile;
        }
        self.output.push_back(bytes);
        Ok(byte_range)
    }

    /// Returns the bytes of the next box written, in file order, or `None` when all are out
    pub fn poll_output(&mut self) -> Option<Vec<u8>> {
        self.output.pop_front()
    }

    /// Ends the file: the writer takes no more boxes
    ///
    /// Boxes written before are still handed out by
    /// [`poll_output`](Self::poll_output).
    ///
    /// # Errors
    ///
    /// [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
    pub fn finish(&mut self) -> Result<(), MuxError> {
        if matches!(self.state, State::Finished) {
            return Err(MuxError::AlreadyFinished);
        }
        self.state = State::Finished;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxSize, BoxType, Error};

    use super::BoxWriter;
    use crate::byte_range::ByteRange;
    use crate::error::MuxError;
    use crate::framing::header_forms::{PAYLOAD, every_header_form};

    fn header_of(file: &[u8]) -> BoxHeader {
        BoxHeader::decode(file).unwrap().0
    }

    #[test]
    fn a_box_of_every_header_form_writes_as_the_bytes_it_reads_from() {
        for file in every_header_form() {
            let mut writer = BoxWriter::new();
            let (front, back) = PAYLOAD.split_at(3);
            let byte_range = writer.handle_box(header_of(&file), [front, back]).unwrap();
            assert_eq!(
                byte_range,
                ByteRange::new(0, Some(u64::try_from(file.len()).unwrap()))
            );
            assert_eq!(writer.poll_output(), Some(file));
            assert_eq!(writer.poll_output(), None);
        }
    }

    #[test]
    fn boxes_lie_end_to_end_from_the_start_of_the_file() {
        let mut writer = BoxWriter::new();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
        let skip = BoxHeader::with_payload_len(BoxType::compact(*b"skip"), 0).unwrap();
        let ranges = [
            writer.handle_box(free, [b"AAAA".as_slice()]).unwrap(),
            writer.handle_box(skip, []).unwrap(),
            writer.handle_box(free, [b"BB".as_slice(), b"BB"]).unwrap(),
        ];
        assert_eq!(
            ranges,
            [
                ByteRange::new(0, Some(12)),
                ByteRange::new(12, Some(8)),
                ByteRange::new(20, Some(12)),
            ]
        );
        assert_eq!(
            core::iter::from_fn(|| writer.poll_output()).collect::<Vec<_>>(),
            [
                b"\0\0\0\x0cfreeAAAA".to_vec(),
                b"\0\0\0\x08skip".to_vec(),
                b"\0\0\0\x0cfreeBBBB".to_vec(),
            ]
        );
    }

    #[test]
    fn a_payload_other_than_the_declared_one_is_refused_and_nothing_is_output() {
        let mut writer = BoxWriter::new();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
        assert_eq!(
            writer.handle_box(free, [b"AAA".as_slice()]),
            Err(MuxError::PayloadLengthMismatch {
                box_type: free.box_type(),
                declared_bytes: 4,
                offered_bytes: 3,
            })
        );
        assert_eq!(writer.poll_output(), None);
        assert_eq!(
            writer.handle_box(free, [b"AAAA".as_slice()]),
            Ok(ByteRange::new(0, Some(12)))
        );
    }

    #[test]
    fn no_box_follows_one_that_runs_to_the_end_of_the_file() {
        let mut writer = BoxWriter::new();
        let mdat = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();
        writer.handle_box(mdat, [b"samples".as_slice()]).unwrap();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 0).unwrap();
        assert_eq!(
            writer.handle_box(free, []),
            Err(MuxError::PastEndOfFile {
                box_type: free.box_type(),
            })
        );
    }

    #[test]
    fn no_box_is_taken_once_the_file_is_finished() {
        let mut writer = BoxWriter::new();
        writer.finish().unwrap();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 0).unwrap();
        assert_eq!(writer.handle_box(free, []), Err(MuxError::AlreadyFinished));
        assert_eq!(writer.finish(), Err(MuxError::AlreadyFinished));
    }

    #[test]
    fn a_header_with_a_reserved_extended_type_is_refused() {
        let mut writer = BoxWriter::new();
        let reserved = [
            0x00, 0x00, 0x00, 0x18, b'u', b'u', b'i', b'd', b'f', b'r', b'e', b'e', 0x00, 0x11,
            0x00, 0x10, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
        ];
        assert_eq!(
            writer.handle_box(header_of(&reserved), []),
            Err(MuxError::Box {
                error: Error::invalid_value("extended_type"),
            })
        );
    }
}
