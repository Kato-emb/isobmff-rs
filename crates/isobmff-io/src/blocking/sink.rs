//! [`Sink`], the chunks a mux FSM made written to a sink

use std::io::{self, Write};

use crate::transfer::ChunkBuffer;

/// The chunks a mux FSM made, written to a sink that is `Write`
///
/// # Contract
///
/// * A [`write_all`](Self::write_all) writes the chunk part-written first,
///   then each chunk `chunks` yields, by `write`s of the sink made again
///   where interrupted. It takes the next chunk off `chunks` only once the
///   chunk before is written whole: a chunk taken is held here until written
///   whole, and a chunk not taken is still in `chunks`, which the caller
///   holds.
/// * A sink refusing bytes, or taking none of them
///   ([`WriteZero`](io::ErrorKind::WriteZero)), fails the
///   [`write_all`](Self::write_all): this keeps the chunk the sink was taking
///   and how much of it the sink took, and the next
///   [`write_all`](Self::write_all) carries on from there, with no byte
///   written twice and none lost.
/// * [`flush`](Self::flush) flushes the sink, and nothing else. A sink that
///   is costly to write to in small pieces is the caller's to wrap in a
///   `BufWriter`.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::blocking::Sink;
/// use isobmff_sample::Sample;
/// use isobmff_structure::FragmentedMuxFsm;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file opening with its brands and the movie its fragments continue
/// let mut file = Vec::new();
/// let mut sink = Sink::new(&mut file);
/// let mut fsm = FragmentedMuxFsm::new();
/// fsm.handle_file_type(file_type())?;
/// fsm.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
///
/// // One fragment of two samples of track 1, closed and the file declared over
/// fsm.begin_fragment(1)?;
/// fsm.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// fsm.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// fsm.finish_fragment()?;
/// fsm.finish()?;
///
/// // What the FSM made is written to the file
/// sink.write_all(&mut core::iter::from_fn(|| fsm.poll_output()))?;
/// sink.flush()?;
///
/// // The file opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Sink<S, C> {
    sink: S,
    buffer: Option<ChunkBuffer<C>>,
}

impl<S: Write, C: AsRef<[u8]>> Sink<S, C> {
    /// Creates one writing to `sink`
    #[must_use]
    pub const fn new(sink: S) -> Self {
        Self { sink, buffer: None }
    }

    /// Writes the chunk part-written, then every chunk `chunks` yields
    ///
    /// # Errors
    ///
    /// * The sink refuses a chunk, or takes none of it
    ///   ([`WriteZero`](io::ErrorKind::WriteZero)).
    pub fn write_all(&mut self, chunks: &mut impl Iterator<Item = C>) -> io::Result<()> {
        while let Some(buffer) = ChunkBuffer::unwritten(&mut self.buffer, chunks) {
            let written = self.sink.write(buffer.rest());
            buffer.took(written)?;
        }

        Ok(())
    }

    /// Flushes the sink
    ///
    /// # Errors
    ///
    /// * The sink does not flush.
    pub fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }

    /// Returns the sink written to
    #[must_use]
    pub const fn get_ref(&self) -> &S {
        &self.sink
    }

    /// Returns the sink written to, for writing to or changing directly
    ///
    /// Bytes written to it directly while a chunk is part-written fall
    /// within the bytes of that chunk.
    #[must_use]
    pub const fn get_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Returns the sink written to, and the bytes of the chunk part-written that the sink has not taken
    ///
    /// The bytes are empty where no chunk is part-written.
    #[must_use]
    pub fn into_parts(self) -> (S, Vec<u8>) {
        (self.sink, ChunkBuffer::into_rest(self.buffer))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use std::io::{self, Write};

    use super::Sink;

    use crate::transfer::tests::framed;

    /// Sink recording what was written to it and whether it was flushed
    ///
    /// Interrupting so many writes first, taking at most so many bytes at a
    /// write when asked to, and refusing the write after each that took some.
    #[derive(Default, PartialEq, Debug)]
    struct Recording {
        written: Vec<u8>,
        flushed: bool,
        interruptions: usize,
        taking_at_most: Option<usize>,
        refusing: bool,
    }

    impl Write for Recording {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(left) = self.interruptions.checked_sub(1) {
                self.interruptions = left;

                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            if self.refusing {
                self.refusing = false;

                return Err(io::Error::from(io::ErrorKind::BrokenPipe));
            }
            let taken = self
                .taking_at_most
                .map_or(bytes, |most| bytes.get(..most).unwrap_or(bytes));
            self.written.extend_from_slice(taken);
            self.refusing = self.taking_at_most.is_some();

            Ok(taken.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;

            Ok(())
        }
    }

    #[test]
    fn a_write_all_writes_every_chunk_and_a_flush_flushes_the_sink() {
        let mut sink = Sink::new(Recording::default());

        sink.write_all(&mut framed(b"MADE").into_iter()).unwrap();
        sink.flush().unwrap();

        assert_eq!(
            sink.sink,
            Recording {
                written: b"\0\0\0\x0cfreeMADE".to_vec(),
                flushed: true,
                interruptions: 0,
                taking_at_most: None,
                refusing: false,
            }
        );
    }

    #[test]
    fn an_interrupted_write_is_made_again() {
        let mut sink = Sink::new(Recording {
            interruptions: 2,
            ..Recording::default()
        });

        sink.write_all(&mut framed(b"MADE").into_iter()).unwrap();

        assert_eq!(
            sink.sink,
            Recording {
                written: b"\0\0\0\x0cfreeMADE".to_vec(),
                ..Recording::default()
            }
        );
    }

    #[test]
    fn a_sink_failing_part_way_through_a_chunk_leaves_the_chunks_after_in_the_iterator_and_has_the_next_write_all_carry_on_from_the_failed_byte()
     {
        let mut sink = Sink::new(Recording {
            taking_at_most: Some(5),
            ..Recording::default()
        });
        let mut chunks = framed(b"MADE").into_iter();

        let failed = sink
            .write_all(&mut chunks)
            .map_err(|failure| failure.kind());
        let left = chunks.as_slice().to_vec();
        while sink.write_all(&mut chunks).is_err() {}

        assert_eq!(
            (failed, left, sink.sink.written),
            (
                Err(io::ErrorKind::BrokenPipe),
                framed(b"MADE").into_iter().skip(1).collect::<Vec<_>>(),
                b"\0\0\0\x0cfreeMADE".to_vec()
            )
        );
    }

    #[test]
    fn a_sink_failing_part_way_through_a_chunk_is_handed_back_with_the_bytes_of_the_chunk_it_did_not_take()
     {
        let mut sink = Sink::new(Recording {
            taking_at_most: Some(5),
            ..Recording::default()
        });

        sink.write_all(&mut [b"\0\0\0\x0cfreeMADE".to_vec()].into_iter())
            .unwrap_err();
        assert_eq!(
            sink.into_parts(),
            (
                Recording {
                    written: b"\0\0\0\x0cf".to_vec(),
                    taking_at_most: Some(5),
                    ..Recording::default()
                },
                b"reeMADE".to_vec()
            )
        );
    }

    #[test]
    fn a_sink_with_no_chunk_part_written_is_handed_back_with_no_bytes() {
        let mut sink = Sink::new(Recording::default());

        sink.write_all(&mut framed(b"MADE").into_iter()).unwrap();

        assert_eq!(
            sink.into_parts(),
            (
                Recording {
                    written: b"\0\0\0\x0cfreeMADE".to_vec(),
                    ..Recording::default()
                },
                Vec::new()
            )
        );
    }

    #[test]
    fn a_sink_taking_no_byte_is_reported_as_the_sink_failing() {
        let mut sink = Sink::new(io::Cursor::new(&mut [][..]));

        assert_eq!(
            sink.write_all(&mut framed(b"MADE").into_iter())
                .map_err(|failure| failure.kind()),
            Err(io::ErrorKind::WriteZero)
        );
    }
}
