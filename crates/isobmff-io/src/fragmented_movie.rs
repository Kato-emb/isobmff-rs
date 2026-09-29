//! [`FragmentedMuxer`], a fragmented movie file written to an asynchronous sink, ISO/IEC 14496-12 Annex A.8

use futures_io::AsyncWrite;
use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;
use isobmff_structure::FragmentedMuxFsm;

use crate::Error;
use crate::driver::Muxer;

/// Lays a fragmented movie file down on an asynchronous sink, taking the samples as they come
///
/// The driver of [`FragmentedMuxFsm`] over `futures::io`: it takes the boxes
/// and the samples as the writer does, and writes every byte the writer makes
/// of them to the sink before the call returns. A caller hands over boxes and
/// samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`FragmentedMuxFsm`]'s contract, carried through as
///   [`Structure`](crate::ErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::ErrorKind::Io), unless the writer refused the call
///   too, whose failure is the one reported. The sink is written to a box
///   header or a box payload at a time, as the writer hands them over — the
///   media data of a fragment whole; one that is costly to write to in small
///   pieces is the caller's to wrap in a buffering sink.
/// * [`finish`](Self::finish) declares the file over and flushes the sink.
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
///
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::FragmentedMuxer;
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // The file the muxer lays down
/// let mut file = Vec::new();
///
/// block_on(async {
///     // A file opening with its brands and the movie its fragments continue
///     let mut muxer = FragmentedMuxer::new(&mut file);
///     muxer.handle_file_type(file_type()).await?;
///     muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))).await?;
///
///     // One fragment of two samples of track 1, written to the file as it is closed
///     muxer.begin_fragment(1).await?;
///     muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())).await?;
///     muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec())).await?;
///     muxer.finish_fragment().await?;
///     muxer.finish().await
/// })
/// # .unwrap();
///
/// // The file opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// ```
#[derive(Debug)]
pub struct FragmentedMuxer<W> {
    muxer: Muxer<W, FragmentedMuxFsm>,
}

impl<W: AsyncWrite + Unpin> FragmentedMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a fragmented movie file
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, FragmentedMuxFsm::new()),
        }
    }

    /// Takes the brands the file declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::handle_file_type`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_file_type(file_type))
            .await
    }

    /// Takes the movie the fragments continue, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::handle_movie`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_movie(movie)).await
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::begin_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses bytes a
    ///   dropped call left over.
    pub async fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment(sequence_number))
            .await
    }

    /// Opens a fragment in which every track continues where the samples written for it reach
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::begin_fragment_continuing`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses bytes a
    ///   dropped call left over.
    pub async fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment_continuing(sequence_number))
            .await
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::handle_sample`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses bytes a
    ///   dropped call left over.
    pub async fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_sample(sample))
            .await
    }

    /// Closes the fragment that is open, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::finish_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn finish_fragment(&mut self) -> Result<(), Error> {
        self.muxer.drive(FragmentedMuxFsm::finish_fragment).await
    }

    /// Declares the file over, and flushes the sink
    ///
    /// The step is made once: a [`finish`](Self::finish) whose future was
    /// dropped is carried to the end by a second call, which writes what was
    /// left over and flushes without laying the last boxes down again.
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink does not flush.
    pub async fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(FragmentedMuxFsm::finish).await
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use futures_executor::block_on;
    use isobmff_test_support::{file_type, written};

    use super::FragmentedMuxer;

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = FragmentedMuxer::new(&mut file);

        block_on(muxer.handle_file_type(file_type())).unwrap();

        assert_eq!(file, written(&file_type()));
    }
}
