//! [`BoxWriter`], which lays the boxes of a file end to end as bytes

use isobmff_core::{BoxHeader, BoxSize};

use crate::byte_range::ByteRange;
use crate::error::MuxError;
use crate::framing::box_event::{BoxEvent, BoxEventKind};
use crate::framing::to_u64;

/// Length of the header with both a `largesize` and an `extended_type` field, the longest there is
const LONGEST_HEADER: usize = 32;

/// Writer that lays boxes end to end as the bytes of a file, ISO/IEC 14496-12 §4.2.2
///
/// The writer does no I/O. It takes a file as the same [`BoxEvent`]s a
/// [`BoxReader`](super::BoxReader) reports, and returns for each the bytes to
/// write next: the header a [`BoxEvent::Header`] opens a box with, and a
/// [`BoxEvent::Payload`] as it is, never copied. Writing them in order, from
/// file offset 0, gives the file. The events of a gathered or streamed box
/// that a reader reports write back to the bytes it was read from; a skipped
/// box has no payload events, and is not passed on.
///
/// A box whose size runs to the end of the file is the last box: the writer
/// refuses any box after it.
///
/// # Examples
///
/// ```
/// use isobmff::framing::{BoxEvent, BoxWriter};
/// use isobmff::{BoxHeader, BoxSize, BoxType, ByteRange, MuxError};
///
/// let mut writer = BoxWriter::new();
/// let mut file = Vec::new();
///
/// // A `free` box, its payload given in two pieces
/// let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
/// file.extend_from_slice(writer.handle_event(BoxEvent::Header(free))?);
/// assert_eq!(writer.open_box(), Some(ByteRange::new(0, Some(12))));
/// for piece in [b"AA".as_slice(), b"AA"] {
///     file.extend_from_slice(writer.handle_event(BoxEvent::Payload(piece))?);
/// }
/// file.extend_from_slice(writer.handle_event(BoxEvent::End)?);
///
/// // An `mdat` whose size runs to the end of the file, the last box there can be
/// let mdat = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();
/// for event in [BoxEvent::Header(mdat), BoxEvent::Payload(b"samples"), BoxEvent::End] {
///     file.extend_from_slice(writer.handle_event(event)?);
/// }
/// writer.finish()?;
///
/// assert_eq!(file, b"\0\0\0\x0cfreeAAAA\0\0\0\0mdatsamples");
/// # Ok::<(), MuxError>(())
/// ```
#[derive(Clone, Debug)]
pub struct BoxWriter {
    /// File offset of the next box
    next_offset: u64,
    state: State,
    header_bytes: [u8; LONGEST_HEADER],
}

/// Which events the writer takes next
#[derive(Clone, Copy, Debug)]
enum State {
    /// A header
    Open,
    /// Payload or the end of the box `header` opened at file offset `offset`, `written` bytes of its payload in
    Writing {
        header: BoxHeader,
        offset: u64,
        written: u64,
    },
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
            next_offset: 0,
            state: State::Open,
            header_bytes: [0; LONGEST_HEADER],
        }
    }

    /// Takes the next event of the file, and returns the bytes to write for it
    ///
    /// A [`BoxEvent::Header`] returns the header, a [`BoxEvent::Payload`]
    /// returns the payload it carries, and a [`BoxEvent::End`] returns no
    /// bytes.
    ///
    /// # Errors
    ///
    /// Nothing is returned and the writer is left as it was when:
    ///
    /// * [`UnexpectedEvent`](MuxError::UnexpectedEvent): a payload or an end
    ///   arrives with no box open, or a header while one is.
    /// * [`PayloadLengthMismatch`](MuxError::PayloadLengthMismatch): a payload
    ///   would run past the length its header declares, or the box ends short
    ///   of it.
    /// * [`PastEndOfFile`](MuxError::PastEndOfFile): a header follows a box
    ///   whose size runs to the end of the file.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
    /// * [`Box`](MuxError::Box): the header does not write, as when its
    ///   `extended_type` is one ISO/IEC 14496-12 §4.2.3 reserves.
    pub fn handle_event<'a>(&'a mut self, event: BoxEvent<'a>) -> Result<&'a [u8], MuxError> {
        match (self.state, event) {
            (State::Finished, _) => Err(MuxError::AlreadyFinished),
            (State::PastEndOfFile, BoxEvent::Header(header)) => Err(MuxError::PastEndOfFile {
                box_type: header.box_type(),
            }),
            (State::Open, BoxEvent::Header(header)) => {
                let unused = header
                    .encode(&mut self.header_bytes)
                    .map_err(|error| MuxError::Box { error })?
                    .len();
                self.state = State::Writing {
                    header,
                    offset: self.next_offset,
                    written: 0,
                };
                let header_len = LONGEST_HEADER.saturating_sub(unused);
                Ok(self.header_bytes.get(..header_len).unwrap_or_default())
            }
            (
                State::Writing {
                    header,
                    offset,
                    written,
                },
                BoxEvent::Payload(bytes),
            ) => {
                let written = written.saturating_add(to_u64(bytes.len()));
                if let Some(declared_bytes) = header.payload_len()
                    && written > declared_bytes
                {
                    return Err(MuxError::PayloadLengthMismatch {
                        box_type: header.box_type(),
                        declared_bytes,
                        offered_bytes: written,
                    });
                }
                self.state = State::Writing {
                    header,
                    offset,
                    written,
                };
                Ok(bytes)
            }
            (
                State::Writing {
                    header,
                    offset,
                    written,
                },
                BoxEvent::End,
            ) => {
                if let Some(declared_bytes) = header.payload_len()
                    && written != declared_bytes
                {
                    return Err(MuxError::PayloadLengthMismatch {
                        box_type: header.box_type(),
                        declared_bytes,
                        offered_bytes: written,
                    });
                }
                self.next_offset = offset
                    .saturating_add(header.encoded_len())
                    .saturating_add(written);
                self.state = if header.size() == BoxSize::ToEndOfFile {
                    State::PastEndOfFile
                } else {
                    State::Open
                };
                Ok(&[])
            }
            (State::Writing { .. }, BoxEvent::Header(_)) => Err(MuxError::UnexpectedEvent {
                expected: BoxEventKind::End,
                found: BoxEventKind::Header,
            }),
            (State::Open | State::PastEndOfFile, BoxEvent::Payload(_) | BoxEvent::End) => {
                Err(MuxError::UnexpectedEvent {
                    expected: BoxEventKind::Header,
                    found: event.kind(),
                })
            }
        }
    }

    /// Returns where the box being written lies in the file, header included, or `None` between boxes
    ///
    /// The length is the one the header declares, `None` for a box whose
    /// size runs to the end of the file.
    #[must_use]
    pub fn open_box(&self) -> Option<ByteRange> {
        match self.state {
            State::Writing { header, offset, .. } => {
                Some(ByteRange::new(offset, header.size().total_bytes()))
            }
            State::Open | State::PastEndOfFile | State::Finished => None,
        }
    }

    /// Returns the file offset the next box starts at, once the box open, if any, ends
    pub(crate) const fn next_offset(&self) -> u64 {
        self.next_offset
    }

    /// Ends the file: the writer takes no more events
    ///
    /// A box whose size runs to the end of the file, still open, ends here.
    ///
    /// # Errors
    ///
    /// The writer is left as it was when:
    ///
    /// * [`UnfinishedBox`](MuxError::UnfinishedBox): a box whose payload
    ///   length is declared is open.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the file is finished.
    pub fn finish(&mut self) -> Result<(), MuxError> {
        match self.state {
            State::Finished => return Err(MuxError::AlreadyFinished),
            State::Writing {
                header, written, ..
            } => {
                if let Some(declared_bytes) = header.payload_len() {
                    return Err(MuxError::UnfinishedBox {
                        box_type: header.box_type(),
                        declared_bytes,
                        written_bytes: written,
                    });
                }
            }
            State::Open | State::PastEndOfFile => {}
        }
        self.state = State::Finished;
        Ok(())
    }
}

impl Default for BoxWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxSize, BoxType, Error};

    use super::BoxWriter;
    use crate::byte_range::ByteRange;
    use crate::error::MuxError;
    use crate::framing::box_event::{BoxEvent, BoxEventKind};
    use crate::framing::header_forms::{PAYLOAD, every_header_form};

    fn header_of(file: &[u8]) -> BoxHeader {
        BoxHeader::decode(file).unwrap().0
    }

    /// Writes `events` and returns the bytes the writer returned for them
    fn write<'a>(
        writer: &mut BoxWriter,
        events: impl IntoIterator<Item = BoxEvent<'a>>,
    ) -> Result<Vec<u8>, MuxError> {
        let mut bytes = Vec::new();
        for event in events {
            bytes.extend_from_slice(writer.handle_event(event)?);
        }
        Ok(bytes)
    }

    #[test]
    fn a_box_of_every_header_form_writes_as_the_bytes_it_reads_from() {
        for file in every_header_form() {
            let header = header_of(&file);
            let (front, back) = PAYLOAD.split_at(3);
            let mut writer = BoxWriter::new();
            let mut written = write(&mut writer, [BoxEvent::Header(header)]).unwrap();
            assert_eq!(
                writer.open_box(),
                Some(ByteRange::new(0, header.size().total_bytes()))
            );
            written.extend(
                write(
                    &mut writer,
                    [
                        BoxEvent::Payload(front),
                        BoxEvent::Payload(back),
                        BoxEvent::End,
                    ],
                )
                .unwrap(),
            );
            assert_eq!(written, file);
            assert_eq!(writer.finish(), Ok(()));
        }
    }

    #[test]
    fn boxes_lie_end_to_end_from_the_start_of_the_file() {
        let mut writer = BoxWriter::new();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
        let skip = BoxHeader::with_payload_len(BoxType::compact(*b"skip"), 0).unwrap();
        let mut file = Vec::new();
        let mut opened = Vec::new();
        for (header, pieces) in [
            (free, [b"AAAA".as_slice()].as_slice()),
            (skip, [].as_slice()),
            (free, [b"BB".as_slice(), b"BB"].as_slice()),
        ] {
            file.extend(write(&mut writer, [BoxEvent::Header(header)]).unwrap());
            opened.push(writer.open_box());
            let payload = pieces.iter().map(|piece| BoxEvent::Payload(piece));
            file.extend(write(&mut writer, payload.chain([BoxEvent::End])).unwrap());
        }
        assert_eq!(
            opened,
            [
                Some(ByteRange::new(0, Some(12))),
                Some(ByteRange::new(12, Some(8))),
                Some(ByteRange::new(20, Some(12))),
            ]
        );
        assert_eq!(
            file,
            b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip\0\0\0\x0cfreeBBBB".as_slice()
        );
        assert_eq!(writer.open_box(), None);
    }

    #[test]
    fn a_payload_past_the_declared_length_is_refused_and_the_box_goes_on() {
        let mut writer = BoxWriter::new();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
        write(
            &mut writer,
            [BoxEvent::Header(free), BoxEvent::Payload(b"AA")],
        )
        .unwrap();
        assert_eq!(
            writer.handle_event(BoxEvent::Payload(b"AAA")),
            Err(MuxError::PayloadLengthMismatch {
                box_type: free.box_type(),
                declared_bytes: 4,
                offered_bytes: 5,
            })
        );
        assert_eq!(
            write(&mut writer, [BoxEvent::Payload(b"AA"), BoxEvent::End]),
            Ok(b"AA".to_vec())
        );
    }

    #[test]
    fn a_box_ending_short_of_the_declared_length_is_refused_and_the_box_goes_on() {
        let mut writer = BoxWriter::new();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
        write(
            &mut writer,
            [BoxEvent::Header(free), BoxEvent::Payload(b"AAA")],
        )
        .unwrap();
        assert_eq!(
            writer.handle_event(BoxEvent::End),
            Err(MuxError::PayloadLengthMismatch {
                box_type: free.box_type(),
                declared_bytes: 4,
                offered_bytes: 3,
            })
        );
        assert_eq!(
            write(&mut writer, [BoxEvent::Payload(b"A"), BoxEvent::End]),
            Ok(b"A".to_vec())
        );
    }

    #[test]
    fn events_out_of_order_are_refused() {
        let mut writer = BoxWriter::new();
        assert_eq!(
            writer.handle_event(BoxEvent::Payload(b"AA")),
            Err(MuxError::UnexpectedEvent {
                expected: BoxEventKind::Header,
                found: BoxEventKind::Payload,
            })
        );
        assert_eq!(
            writer.handle_event(BoxEvent::End),
            Err(MuxError::UnexpectedEvent {
                expected: BoxEventKind::Header,
                found: BoxEventKind::End,
            })
        );
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 0).unwrap();
        write(&mut writer, [BoxEvent::Header(free)]).unwrap();
        assert_eq!(
            writer.handle_event(BoxEvent::Header(free)),
            Err(MuxError::UnexpectedEvent {
                expected: BoxEventKind::End,
                found: BoxEventKind::Header,
            })
        );
    }

    #[test]
    fn no_box_follows_one_that_runs_to_the_end_of_the_file() {
        let mut writer = BoxWriter::new();
        let mdat = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();
        write(
            &mut writer,
            [
                BoxEvent::Header(mdat),
                BoxEvent::Payload(b"samples"),
                BoxEvent::End,
            ],
        )
        .unwrap();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 0).unwrap();
        assert_eq!(
            writer.handle_event(BoxEvent::Header(free)),
            Err(MuxError::PastEndOfFile {
                box_type: free.box_type(),
            })
        );
    }

    #[test]
    fn finishing_ends_a_box_that_runs_to_the_end_of_the_file() {
        let mut writer = BoxWriter::new();
        let mdat = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();
        write(
            &mut writer,
            [BoxEvent::Header(mdat), BoxEvent::Payload(b"samples")],
        )
        .unwrap();
        assert_eq!(writer.finish(), Ok(()));
    }

    #[test]
    fn finishing_inside_a_box_of_declared_length_is_refused() {
        let mut writer = BoxWriter::new();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
        write(
            &mut writer,
            [BoxEvent::Header(free), BoxEvent::Payload(b"AA")],
        )
        .unwrap();
        assert_eq!(
            writer.finish(),
            Err(MuxError::UnfinishedBox {
                box_type: free.box_type(),
                declared_bytes: 4,
                written_bytes: 2,
            })
        );
        write(&mut writer, [BoxEvent::Payload(b"AA"), BoxEvent::End]).unwrap();
        assert_eq!(writer.finish(), Ok(()));
    }

    #[test]
    fn nothing_is_taken_once_the_file_is_finished() {
        let mut writer = BoxWriter::new();
        writer.finish().unwrap();
        let free = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 0).unwrap();
        assert_eq!(
            writer.handle_event(BoxEvent::Header(free)),
            Err(MuxError::AlreadyFinished)
        );
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
            writer.handle_event(BoxEvent::Header(header_of(&reserved))),
            Err(MuxError::Box {
                error: Error::invalid_value("extended_type"),
            })
        );
        assert_eq!(writer.open_box(), None);
    }
}
