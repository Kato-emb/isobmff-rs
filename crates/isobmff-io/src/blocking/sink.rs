//! [`Sink`], the chunks a mux FSM made written to a sink

use std::io::{self, Write};

use isobmff_sequence::EventBytes;

use crate::transfer::ChunkBuffer;

/// The chunks a mux FSM made, written to a sink that is `Write`
///
/// # Contract
///
/// * A [`write`](Self::write) writes the chunk part-written first, then each
///   chunk the iterator yields, by `write`s of the sink made again where
///   interrupted. It takes the next chunk off the iterator only once the
///   chunk before is written whole, so the chunks it did not take stay with
///   the iterator.
/// * A sink refusing bytes, or taking none of them
///   ([`WriteZero`](io::ErrorKind::WriteZero)), fails the
///   [`write`](Self::write): this keeps the chunk the sink was taking and how
///   much of it the sink took, and the next [`write`](Self::write) carries on
///   from there, with no byte written twice and none lost.
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
/// sink.write(core::iter::from_fn(|| fsm.poll_output()))?;
/// sink.flush()?;
///
/// // The file opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Sink<S> {
    sink: S,
    buffer: Option<ChunkBuffer>,
}

impl<S: Write> Sink<S> {
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
    pub fn write(&mut self, chunks: impl IntoIterator<Item = EventBytes>) -> io::Result<()> {
        let mut chunks = chunks.into_iter();
        while let Some(buffer) = ChunkBuffer::unwritten(&mut self.buffer, &mut chunks) {
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
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::cell::Cell;
    use std::io::{self, Write};

    use super::Sink;

    use crate::transfer::tests::framed;

    /// Sink recording what was written to it and whether it was flushed
    ///
    /// Taking at most so many bytes at a write when asked to, and refusing
    /// the write after each that took some.
    #[derive(Default, PartialEq, Debug)]
    struct Recording {
        written: Vec<u8>,
        flushed: bool,
        taking_at_most: Option<usize>,
        refusing: bool,
    }

    impl Write for Recording {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
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
    fn a_write_writes_every_chunk_and_a_flush_flushes_the_sink() {
        let mut sink = Sink::new(Recording::default());

        sink.write(framed(b"MADE")).unwrap();
        sink.flush().unwrap();

        assert_eq!(
            sink.sink,
            Recording {
                written: b"\0\0\0\x0cfreeMADE".to_vec(),
                flushed: true,
                taking_at_most: None,
                refusing: false,
            }
        );
    }

    #[test]
    fn a_sink_failing_part_way_through_a_chunk_has_the_rest_written_by_the_next_write_and_the_chunks_after_left_untaken()
     {
        let mut sink = Sink::new(Recording {
            taking_at_most: Some(5),
            ..Recording::default()
        });
        let mut chunks = framed(b"MADE").into_iter();
        let taken = Cell::new(0_usize);
        let mut counted = core::iter::from_fn(|| {
            taken.set(taken.get().saturating_add(1));
            chunks.next()
        });

        let first = sink.write(&mut counted).map_err(|failure| failure.kind());
        let taken_by_the_first = taken.get();
        while sink.write(&mut counted).is_err() {}

        assert_eq!(
            (first, taken_by_the_first, sink.sink.written),
            (
                Err(io::ErrorKind::BrokenPipe),
                1,
                b"\0\0\0\x0cfreeMADE".to_vec()
            )
        );
    }

    #[test]
    fn a_sink_taking_no_byte_is_reported_as_the_sink_failing() {
        let mut sink = Sink::new(io::Cursor::new(&mut [][..]));

        assert_eq!(
            sink.write(framed(b"MADE"))
                .map_err(|failure| failure.kind()),
            Err(io::ErrorKind::WriteZero)
        );
    }
}
