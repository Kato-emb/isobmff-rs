//! [`NonFragmentedMuxer`], a non-fragmented movie file written to an asynchronous sink, ISO/IEC 14496-12 §8.2.1 and §8.7

use futures_io::AsyncWrite;
use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;
use isobmff_structure::NonFragmentedMuxFsm;

use crate::Error;
use crate::driver::Muxer;

/// Lays a non-fragmented movie file down on an asynchronous sink, taking the samples as they come
///
/// The driver of [`NonFragmentedMuxFsm`] over `futures::io`: it takes the
/// boxes and the samples as the writer does, and writes every byte the writer
/// makes of them to the sink before the call returns. A caller hands over
/// boxes and samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`NonFragmentedMuxFsm`]'s contract, carried through as
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
/// use isobmff_boxes::SampleFlags;
/// use isobmff_io::{DemuxDriver, NonFragmentedMuxer};
/// use isobmff_sample::Sample;
/// use isobmff_structure::NonFragmentedDemuxFsm;
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
///     muxer.handle_sample(Sample::new(1, 0, 3_000, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())).await?;
///     muxer.handle_sample(Sample::new(1, 3_000, 3_000, 0, SampleFlags::ZERO, 1, b"DATA".to_vec())).await?;
///     muxer.begin_chunk().await?;
///     muxer.handle_sample(Sample::new(1, 6_000, 3_000, 0, SampleFlags::ZERO, 1, b"LAST".to_vec())).await?;
///     muxer.finish().await?;
///
///     // Read back, the samples come out as they were laid down
///     let mut driver = DemuxDriver::new(Cursor::new(file), NonFragmentedDemuxFsm::new()).await?;
///     let mut read_back = Vec::new();
///     while let Some(sample) = driver.next().await {
///         read_back.push(sample?.into_data());
///     }
///     assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec(), b"LAST".to_vec()]);
/// #   Ok::<(), isobmff_io::Error>(())
/// })
/// # .unwrap();
/// ```
#[derive(Debug)]
pub struct NonFragmentedMuxer<W> {
    muxer: Muxer<W, NonFragmentedMuxFsm>,
}

impl<W: AsyncWrite + Unpin> NonFragmentedMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a non-fragmented movie file
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, NonFragmentedMuxFsm::new()),
        }
    }

    /// Takes the brands the file declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedMuxFsm::handle_file_type`] makes of the call.
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
    ///   [`NonFragmentedMuxFsm::handle_movie`] makes of the call.
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
    ///   [`NonFragmentedMuxFsm::begin_chunk`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn begin_chunk(&mut self) -> Result<(), Error> {
        self.muxer.drive(NonFragmentedMuxFsm::begin_chunk).await
    }

    /// Takes a sample, and places it at the end of the chunk that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedMuxFsm::handle_sample`] makes of the call.
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
    ///   [`NonFragmentedMuxFsm::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub async fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(NonFragmentedMuxFsm::finish).await
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use futures_executor::block_on;
    use isobmff_test_support::{file_type, written};

    use super::NonFragmentedMuxer;

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = NonFragmentedMuxer::new(&mut file);

        block_on(muxer.handle_file_type(file_type())).unwrap();

        assert_eq!(file, written(&file_type()));
    }
}
