//! [`MediaSegmentMuxer`], a media segment written to an asynchronous sink, ISO/IEC 14496-12 §8.16

use futures_io::AsyncWrite;
use isobmff_boxes::SegmentTypeBox;
use isobmff_sample::Sample;
use isobmff_structure::MediaSegmentMuxFsm;

use crate::Error;
use crate::driver::Muxer;

/// Lays a media segment down on an asynchronous sink, taking the samples as they come
///
/// The driver of [`MediaSegmentMuxFsm`] over `futures::io`: it takes the
/// brands and the samples as the writer does, and writes every byte the
/// writer makes of them to the sink before the call returns. A caller hands
/// over brands and samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`MediaSegmentMuxFsm`]'s contract, carried through as
///   [`Structure`](crate::ErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::ErrorKind::Io), unless the writer refused the call
///   too, whose failure is the one reported. The sink is written to a box
///   header or a box payload at a time, as the writer hands them over — the
///   media data of a fragment whole; one that is costly to write to in small
///   pieces is the caller's to wrap in a buffering sink.
/// * [`finish`](Self::finish) declares the segment over and flushes the sink.
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
/// use isobmff_boxes::SampleFlags;
/// use isobmff_io::MediaSegmentMuxer;
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::segment_type;
/// // The segment the muxer lays down
/// let mut segment = Vec::new();
///
/// block_on(async {
///     // A segment opening with its brands
///     let mut muxer = MediaSegmentMuxer::new(&mut segment);
///     muxer.handle_segment_type(segment_type()).await?;
///
///     // One fragment of two samples of track 1, written to the segment as it is closed
///     muxer.begin_fragment(1).await?;
///     muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())).await?;
///     muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec())).await?;
///     muxer.finish_fragment().await?;
///     muxer.finish().await
/// })
/// # .unwrap();
///
/// // The segment opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&segment[4..8], b"styp");
/// assert!(segment.ends_with(b"SAMPDATA"));
/// ```
#[derive(Debug)]
pub struct MediaSegmentMuxer<W> {
    muxer: Muxer<W, MediaSegmentMuxFsm>,
}

impl<W: AsyncWrite + Unpin> MediaSegmentMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a media segment
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, MediaSegmentMuxFsm::new()),
        }
    }

    /// Takes the brands the segment declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::handle_segment_type`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn handle_segment_type(&mut self, segment_type: SegmentTypeBox) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_segment_type(segment_type))
            .await
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::begin_fragment`] makes of the call.
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
    ///   [`MediaSegmentMuxFsm::begin_fragment_continuing`] makes of the call.
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
    ///   [`MediaSegmentMuxFsm::handle_sample`] makes of the call.
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
    ///   [`MediaSegmentMuxFsm::finish_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn finish_fragment(&mut self) -> Result<(), Error> {
        self.muxer.drive(MediaSegmentMuxFsm::finish_fragment).await
    }

    /// Declares the segment over, and flushes the sink
    ///
    /// The step is made once: a [`finish`](Self::finish) whose future was
    /// dropped is carried to the end by a second call, which writes what was
    /// left over and flushes without laying the last boxes down again.
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink does not flush.
    pub async fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(MediaSegmentMuxFsm::finish).await
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use futures_executor::block_on;
    use isobmff_test_support::{segment_type, written};

    use super::MediaSegmentMuxer;

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut segment = Vec::new();
        let mut muxer = MediaSegmentMuxer::new(&mut segment);

        block_on(muxer.handle_segment_type(segment_type())).unwrap();

        assert_eq!(segment, written(&segment_type()));
    }
}
