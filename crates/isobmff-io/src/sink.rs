//! [`Sink`], the chunks a mux FSM made written to an asynchronous sink
//!
//! The sink over `std::io` is written apart, in [`blocking`](crate::blocking).

use core::future::poll_fn;
use core::pin::Pin;
use std::io;

use futures_io::AsyncWrite;

use crate::transfer::ChunkBuffer;

/// The chunks a mux FSM made, written to an asynchronous sink
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
///   buffering sink.
/// * Every `async fn` here is cancellation safe. The count of what the sink
///   took moves as each write completes, before the next await, so a
///   [`write_all`](Self::write_all) dropped where the sink stood still is
///   carried on by the next one, with no byte written twice and none lost.
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
///     sink.write_all(&mut core::iter::from_fn(|| fsm.poll_output())).await?;
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
pub struct Sink<S, C> {
    sink: S,
    buffer: Option<ChunkBuffer<C>>,
}

impl<S: AsyncWrite + Unpin, C: AsRef<[u8]>> Sink<S, C> {
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
    pub async fn write_all(&mut self, chunks: &mut impl Iterator<Item = C>) -> io::Result<()> {
        while let Some(buffer) = ChunkBuffer::unwritten(&mut self.buffer, chunks) {
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
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use std::io;

    use futures_executor::block_on;
    use futures_io::AsyncWrite;
    use futures_util::io::Cursor;

    use super::Sink;

    use crate::transfer::tests::{framed, poll_once};

    /// Sink recording what was written to it, and whether it was flushed
    ///
    /// Interrupting so many writes first.
    #[derive(Default, PartialEq, Debug)]
    struct Recording {
        written: Vec<u8>,
        flushed: bool,
        interruptions: usize,
    }

    impl AsyncWrite for Recording {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if let Some(left) = self.interruptions.checked_sub(1) {
                self.interruptions = left;

                return Poll::Ready(Err(io::Error::from(io::ErrorKind::Interrupted)));
            }
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

    /// Sink taking one byte at a time
    ///
    /// Standing still once it holds `hesitate_at` of them, and refusing a
    /// write once it holds `refuse_at` of them.
    #[derive(Default, PartialEq, Debug)]
    struct Trickle {
        written: Vec<u8>,
        hesitate_at: Option<usize>,
        refuse_at: Option<usize>,
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
            if self.refuse_at == Some(self.written.len()) {
                self.refuse_at = None;

                return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
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
    fn a_write_all_writes_every_chunk_and_a_flush_flushes_the_sink() {
        let mut sink = Sink::new(Recording::default());

        block_on(async {
            sink.write_all(&mut framed(b"MADE").into_iter())
                .await
                .unwrap();
            sink.flush().await.unwrap();
        });

        assert_eq!(
            sink.sink,
            Recording {
                written: b"\0\0\0\x0cfreeMADE".to_vec(),
                flushed: true,
                interruptions: 0,
            }
        );
    }

    #[test]
    fn an_interrupted_write_is_made_again() {
        let mut sink = Sink::new(Recording {
            interruptions: 2,
            ..Recording::default()
        });

        block_on(sink.write_all(&mut framed(b"MADE").into_iter())).unwrap();

        assert_eq!(
            sink.sink,
            Recording {
                written: b"\0\0\0\x0cfreeMADE".to_vec(),
                ..Recording::default()
            }
        );
    }

    #[test]
    fn a_sink_taking_no_byte_is_reported_as_the_sink_failing() {
        let mut sink = Sink::new(Cursor::new(&mut [][..]));

        assert_eq!(
            block_on(sink.write_all(&mut framed(b"MADE").into_iter()))
                .map_err(|failure| failure.kind()),
            Err(io::ErrorKind::WriteZero)
        );
    }

    #[test]
    fn a_write_all_dropped_part_way_through_a_chunk_leaves_the_chunks_after_in_the_iterator_and_has_the_next_write_all_write_the_rest_once()
     {
        let mut sink = Sink::new(Trickle {
            hesitate_at: Some(3),
            ..Trickle::default()
        });
        let mut chunks = [framed(b"MADE"), framed(b"MORE")].concat().into_iter();

        assert!(poll_once(sink.write_all(&mut chunks)).is_none());
        let left = chunks.as_slice().to_vec();
        block_on(sink.write_all(&mut chunks)).unwrap();

        assert_eq!(
            (left, sink.sink.written),
            (
                framed(b"MADE")
                    .into_iter()
                    .skip(1)
                    .chain(framed(b"MORE"))
                    .collect::<Vec<_>>(),
                b"\0\0\0\x0cfreeMADE\0\0\0\x0cfreeMORE".to_vec()
            )
        );
    }

    #[test]
    fn a_sink_failing_part_way_through_a_chunk_leaves_the_chunks_after_in_the_iterator_and_has_the_next_write_all_carry_on_from_the_failed_byte()
     {
        let mut sink = Sink::new(Trickle {
            refuse_at: Some(5),
            ..Trickle::default()
        });
        let mut chunks = framed(b"MADE").into_iter();

        let failed = block_on(sink.write_all(&mut chunks)).map_err(|failure| failure.kind());
        let left = chunks.as_slice().to_vec();
        block_on(sink.write_all(&mut chunks)).unwrap();

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
        let mut sink = Sink::new(Trickle {
            refuse_at: Some(5),
            ..Trickle::default()
        });

        block_on(sink.write_all(&mut [b"\0\0\0\x0cfreeMADE".to_vec()].into_iter())).unwrap_err();
        assert_eq!(
            sink.into_parts(),
            (
                Trickle {
                    written: b"\0\0\0\x0cf".to_vec(),
                    ..Trickle::default()
                },
                b"reeMADE".to_vec()
            )
        );
    }
}
