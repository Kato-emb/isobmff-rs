//! [`Sink`], the chunks a mux FSM made written to an asynchronous sink
//!
//! The sink over `std::io` is written apart, in [`blocking`](crate::blocking).

use core::future::poll_fn;
use core::pin::Pin;
use std::io;

use futures_io::AsyncWrite;
use isobmff_sequence::EventBytes;

use crate::transfer::ChunkBuffer;

/// The chunks a mux FSM made, written to an asynchronous sink
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
///   buffering sink.
/// * Every `async fn` here is cancellation safe. The count of what the sink
///   took moves as each write completes, before the next await, so a
///   [`write`](Self::write) dropped where the sink stood still is carried on
///   by the next one, with no byte written twice and none lost.
///
/// # Examples
///
/// ```
/// use futures_executor::block_on;
///
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::Sink;
/// use isobmff_sample::Sample;
/// use isobmff_structure::FragmentedMuxFsm;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// block_on(async {
///     // A file opening with its brands and the movie its fragments continue
///     let mut file = Vec::new();
///     let mut sink = Sink::new(&mut file);
///     let mut fsm = FragmentedMuxFsm::new();
///     fsm.handle_file_type(file_type())?;
///     fsm.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
///
///     // One fragment of two samples of track 1, closed and the file declared over
///     fsm.begin_fragment(1)?;
///     fsm.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
///     fsm.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
///     fsm.finish_fragment()?;
///     fsm.finish()?;
///
///     // What the FSM made is written to the file
///     sink.write(core::iter::from_fn(|| fsm.poll_output())).await?;
///     sink.flush().await?;
///
///     // The file opens with the brands, and the media data holds the samples end to end
///     assert_eq!(&file[4..8], b"ftyp");
///     assert!(file.ends_with(b"SAMPDATA"));
/// #   Ok::<(), Box<dyn core::error::Error>>(())
/// })
/// # .unwrap();
/// ```
#[derive(Debug)]
pub struct Sink<S> {
    sink: S,
    buffer: Option<ChunkBuffer>,
}

impl<S: AsyncWrite + Unpin> Sink<S> {
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
    pub async fn write(&mut self, chunks: impl IntoIterator<Item = EventBytes>) -> io::Result<()> {
        let mut chunks = chunks.into_iter();
        while let Some(buffer) = ChunkBuffer::unwritten(&mut self.buffer, &mut chunks) {
            let written =
                poll_fn(|context| Pin::new(&mut self.sink).poll_write(context, buffer.rest()))
                    .await;
            buffer.took(written)?;
        }

        Ok(())
    }

    /// Flushes the sink
    ///
    /// # Errors
    ///
    /// * The sink does not flush.
    pub async fn flush(&mut self) -> io::Result<()> {
        poll_fn(|context| Pin::new(&mut self.sink).poll_flush(context)).await
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::cell::Cell;
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use std::io;

    use futures_executor::block_on;
    use futures_io::AsyncWrite;
    use futures_util::io::Cursor;

    use super::Sink;

    use crate::transfer::tests::{framed, poll_once};

    /// Sink recording what was written to it, and whether it was flushed
    #[derive(Default, PartialEq, Debug)]
    struct Recording {
        written: Vec<u8>,
        flushed: bool,
    }

    impl AsyncWrite for Recording {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.written.extend_from_slice(bytes);

            Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            self.flushed = true;

            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// Sink taking one byte at a time, and standing still once it holds `hesitate_at` of them
    #[derive(Default, Debug)]
    struct Trickle {
        written: Vec<u8>,
        hesitate_at: Option<usize>,
    }

    impl AsyncWrite for Trickle {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.hesitate_at == Some(self.written.len()) {
                self.hesitate_at = None;
                context.waker().wake_by_ref();

                return Poll::Pending;
            }
            self.written.extend(bytes.iter().take(1));

            Poll::Ready(Ok(bytes.len().min(1)))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn a_write_writes_every_chunk_and_a_flush_flushes_the_sink() {
        let mut sink = Sink::new(Recording::default());

        block_on(async {
            sink.write(framed(b"MADE")).await.unwrap();
            sink.flush().await.unwrap();
        });

        assert_eq!(
            sink.sink,
            Recording {
                written: b"\0\0\0\x0cfreeMADE".to_vec(),
                flushed: true,
            }
        );
    }

    #[test]
    fn a_sink_taking_no_byte_is_reported_as_the_sink_failing() {
        let mut sink = Sink::new(Cursor::new(&mut [][..]));

        assert_eq!(
            block_on(sink.write(framed(b"MADE"))).map_err(|failure| failure.kind()),
            Err(io::ErrorKind::WriteZero)
        );
    }

    #[test]
    fn a_write_dropped_part_way_through_a_chunk_has_the_next_write_the_rest_once_and_the_chunks_after_left_untaken()
     {
        let mut sink = Sink::new(Trickle {
            hesitate_at: Some(3),
            ..Trickle::default()
        });
        let mut chunks = framed(b"MADE").into_iter().chain(framed(b"MORE"));
        let taken = Cell::new(0_usize);
        let mut counted = core::iter::from_fn(|| {
            taken.set(taken.get().saturating_add(1));
            chunks.next()
        });

        assert!(poll_once(sink.write(&mut counted)).is_none());
        let taken_by_the_dropped = taken.get();
        block_on(sink.write(&mut counted)).unwrap();

        assert_eq!(
            (taken_by_the_dropped, sink.sink.written),
            (1, b"\0\0\0\x0cfreeMADE\0\0\0\x0cfreeMORE".to_vec())
        );
    }
}
