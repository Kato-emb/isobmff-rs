//! [`NonFragmentedDemuxer`] and [`NonFragmentedMuxer`], a non-fragmented movie file read off an asynchronous source that seeks and written to an asynchronous sink, ISO/IEC 14496-12 §8.2.1 and §8.7

use core::ops::Range;

use futures_io::{AsyncRead, AsyncSeek, AsyncWrite};
use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;
use isobmff_structure::{NonFragmentedReader, NonFragmentedWriter};

use crate::Error;
use crate::driver::{Demuxer, Muxer, PollOutput, ReadSamples};

/// Reads the samples a non-fragmented movie file carries off an asynchronous source that seeks
///
/// The driver of [`NonFragmentedReader`] over `futures::io`: it reads the
/// file off the source a cut at a time and hands each over, fetches the bytes
/// the reader names as lacking wherever the file passed them by — the media
/// data of a movie lying after it — by seeking to them, and hands over the
/// samples as they come whole. A caller takes [`Sample`]s and nothing else
/// moves.
///
/// # Contract
///
/// * The file begins where the source stands when the demuxer is created,
///   and every seek is made from there: a file lying at some position in a
///   larger resource is read by seeking the source to it first.
/// * The samples come out of [`next`](Self::next), in the order the reader
///   completes them — a movie lying before its media data has them come as
///   the file lays them down; one lying after it has them come in the order
///   the reader holds their extents, as the bytes fetched for the extent
///   held longest complete them. The boxes the reader read into values are
///   there to read once they have come: [`file_type`](Self::file_type) and
///   [`movie`](Self::movie).
/// * The bytes fetched for a want come off the source a cut at a time, as
///   the file does, so every extent it reaches is filled by it; a want the
///   read did not cover whole — one longer than a cut, or one the source
///   handed over in pieces — is named again by the reader and fetched again. A
///   source ending before bytes the reader lacks is the reader's to report
///   at the end of the file, as
///   [`Structure`](crate::ErrorKind::Structure); one ending before
///   where it had already been read to is [`Io`](crate::ErrorKind::Io)
///   with [`UnexpectedEof`](std::io::ErrorKind::UnexpectedEof).
/// * A failure ends the samples: the ones the reader had completed before it
///   come first, then the failure once, then `None` for good. The end of the
///   file is the same without the failure.
/// * Every `async fn` here is cancellation safe: a future dropped where the
///   source stood still is carried on by the call that follows.
///
/// # Examples
///
/// ```
/// use futures_executor::block_on;
/// use futures_util::io::Cursor;
///
/// use isobmff_io::NonFragmentedDemuxer;
/// # use isobmff_test_support::non_fragmented_file;
/// // A file of two chunks of one track, its movie lying after its media data
/// let file = non_fragmented_file(&[&[b"SAMP", b"DATA"], &[b"LAST"]], false);
///
/// block_on(async {
///     // The samples are read off the file, the bytes each lacks fetched by seeking
///     let mut demuxer = NonFragmentedDemuxer::new(Cursor::new(file)).await?;
///     let mut read_back = Vec::new();
///     while let Some(sample) = demuxer.next().await {
///         read_back.push(sample?.into_data());
///     }
///     assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec(), b"LAST".to_vec()]);
///
///     // The movie the file declared is there to read
///     assert_eq!(demuxer.movie().map(|moov| moov.trak().len()), Some(1));
/// #   Ok::<(), isobmff_io::Error>(())
/// })
/// # .unwrap();
/// ```
#[derive(Debug)]
pub struct NonFragmentedDemuxer<S> {
    demuxer: Demuxer<S, NonFragmentedReader>,
}

impl<S: AsyncRead + AsyncSeek + Unpin> NonFragmentedDemuxer<S> {
    /// Creates a demuxer over `source`, the file beginning where it stands
    ///
    /// The reader beneath is [`NonFragmentedReader::new`]; one holding the
    /// file to other limits is driven through
    /// [`with_reader`](Self::with_reader).
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub async fn new(source: S) -> Result<Self, Error> {
        Self::with_reader(source, NonFragmentedReader::new()).await
    }

    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub async fn with_reader(source: S, reader: NonFragmentedReader) -> Result<Self, Error> {
        Ok(Self {
            demuxer: Demuxer::new(source, reader).await?,
        })
    }

    /// Returns the brands the file declares itself readable as, once they have come
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.demuxer.reader().file_type()
    }

    /// Returns the movie whose sample tables declare the samples of the file, once it has come
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.demuxer.reader().movie()
    }

    /// Takes the next sample the file carries, reading on until one comes
    pub async fn next(&mut self) -> Option<Result<Sample, Error>> {
        self.demuxer.next().await
    }
}

impl ReadSamples for NonFragmentedReader {
    fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error> {
        NonFragmentedReader::handle_input(self, input)
    }

    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), isobmff_structure::Error> {
        NonFragmentedReader::handle_data(self, offset, data)
    }

    fn poll_sample(&mut self) -> Option<Sample> {
        NonFragmentedReader::poll_sample(self)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        NonFragmentedReader::wanted_extent(self)
    }

    fn finish(&mut self) -> Result<(), isobmff_structure::Error> {
        NonFragmentedReader::finish(self)
    }
}

/// Lays a non-fragmented movie file down on an asynchronous sink, taking the samples as they come
///
/// The driver of [`NonFragmentedWriter`] over `futures::io`: it takes the
/// boxes and the samples as the writer does, and writes every byte the writer
/// makes of them to the sink before the call returns. A caller hands over
/// boxes and samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`NonFragmentedWriter`]'s contract, carried through as
///   [`Structure`](crate::ErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::ErrorKind::Io), unless the writer refused the call
///   too, whose failure is the one reported. The sink is written to a box
///   header, a box payload, or one sample of a chunk at a time, as the
///   writer hands them over; one that is costly to write to in small pieces
///   is the caller's to wrap in a buffering sink.
/// * [`finish`](Self::finish) declares the file over, writes the movie the
///   writer lays down last, and flushes the sink.
/// * Every `async fn` here is cancellation safe: a call makes its step of the
///   writer before it awaits anything, so a future dropped where the sink
///   stood still has made that step and no more, and the call that follows
///   writes what was left over ahead of its own bytes and reports the refusal
///   the dropped one was carrying instead of making a step of its own.
///
/// # Examples
///
/// ```
/// use futures_executor::block_on;
/// use futures_util::io::Cursor;
///
/// use isobmff_io::{NonFragmentedDemuxer, NonFragmentedMuxer};
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{file_type, unfragmented_movie};
/// block_on(async {
///     // A file opening with its brands, whose movie declares one track and no sample yet
///     let mut file = Vec::new();
///     let mut muxer = NonFragmentedMuxer::new(&mut file);
///     muxer.handle_file_type(file_type()).await?;
///     muxer.handle_movie(unfragmented_movie()).await?;
///
///     // Two chunks of track 1, written to the file as they come
///     muxer.begin_chunk().await?;
///     muxer.handle_sample(Sample::new(1, 0, 3_000, 0, 0, 1, b"SAMP".to_vec())).await?;
///     muxer.handle_sample(Sample::new(1, 3_000, 3_000, 0, 0, 1, b"DATA".to_vec())).await?;
///     muxer.begin_chunk().await?;
///     muxer.handle_sample(Sample::new(1, 6_000, 3_000, 0, 0, 1, b"LAST".to_vec())).await?;
///     muxer.finish().await?;
///
///     // Read back, the samples come out as they were laid down
///     let mut demuxer = NonFragmentedDemuxer::new(Cursor::new(file)).await?;
///     let mut read_back = Vec::new();
///     while let Some(sample) = demuxer.next().await {
///         read_back.push(sample?.into_data());
///     }
///     assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec(), b"LAST".to_vec()]);
/// #   Ok::<(), isobmff_io::Error>(())
/// })
/// # .unwrap();
/// ```
#[derive(Debug)]
pub struct NonFragmentedMuxer<W> {
    muxer: Muxer<W, NonFragmentedWriter>,
}

impl<W: AsyncWrite + Unpin> NonFragmentedMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a non-fragmented movie file
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, NonFragmentedWriter::new()),
        }
    }

    /// Takes the brands the file declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_file_type`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_file_type(file_type))
            .await
    }

    /// Takes the movie the file is laid down against, to be written last
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_movie`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses bytes a
    ///   dropped call left over.
    pub async fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_movie(movie)).await
    }

    /// Opens a chunk, which the samples handed over next are laid out in, writing the chunk open before it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::begin_chunk`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn begin_chunk(&mut self) -> Result<(), Error> {
        self.muxer.drive(NonFragmentedWriter::begin_chunk).await
    }

    /// Takes a sample, and places it at the end of the chunk that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_sample`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses bytes a
    ///   dropped call left over.
    pub async fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_sample(sample))
            .await
    }

    /// Declares the file over, writing the chunk that is open and then the movie, and flushes the sink
    ///
    /// The step is made once: a [`finish`](Self::finish) whose future was
    /// dropped is carried to the end by a second call, which writes what was
    /// left over and flushes without laying the last boxes down again.
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub async fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(NonFragmentedWriter::finish).await
    }
}

impl PollOutput for NonFragmentedWriter {
    fn poll_output(&mut self) -> Option<EventBytes> {
        NonFragmentedWriter::poll_output(self)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use futures_executor::block_on;
    use futures_util::io::Cursor;
    use isobmff_sample::Sample;
    use isobmff_test_support::{SAMPLE_DURATION, file_type, non_fragmented_file, written};

    use super::{NonFragmentedDemuxer, NonFragmentedMuxer};

    #[test]
    fn a_movie_lying_after_its_media_data_has_the_bytes_fetched() {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);

        let read_back = block_on(async {
            let mut demuxer = NonFragmentedDemuxer::new(Cursor::new(file)).await.unwrap();
            let mut read_back = Vec::new();
            while let Some(sample) = demuxer.next().await {
                read_back.push(sample.unwrap());
            }

            read_back
        });

        assert_eq!(
            read_back,
            [Sample::new(
                1,
                0,
                SAMPLE_DURATION,
                0,
                0,
                1,
                b"SAMP".to_vec()
            )]
        );
    }

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = NonFragmentedMuxer::new(&mut file);

        block_on(muxer.handle_file_type(file_type())).unwrap();

        assert_eq!(file, written(&file_type()));
    }
}
