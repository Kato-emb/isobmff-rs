//! [`MediaSegmentDemuxer`] and [`MediaSegmentMuxer`], a media segment read off an asynchronous source that seeks and written to an asynchronous sink, ISO/IEC 14496-12 §8.16

use core::ops::Range;

use futures_io::{AsyncRead, AsyncSeek, AsyncWrite};
use isobmff_boxes::{MovieBox, SegmentTypeBox};
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;
use isobmff_structure::{MediaSegmentReader, MediaSegmentWriter};

use crate::Error;
use crate::driver::{Demuxer, Muxer};
use crate::stack::{PollOutput, ReadSamples};

/// Reads the samples a media segment carries off an asynchronous source that seeks
///
/// The driver of [`MediaSegmentReader`] over `futures::io`: it reads the
/// segment off the source a cut at a time and hands each over, fetches the
/// bytes the reader names as lacking wherever the segment passed them by —
/// the media data of a fragment addressing bytes before it — by seeking to
/// them, and hands over the samples as they come whole. A caller takes
/// [`Sample`]s and nothing else moves.
///
/// # Contract
///
/// * The segment begins where the source stands when the demuxer is created,
///   and every seek is made from there: a segment lying at some position in a
///   larger resource is read by seeking the source to it first.
/// * The samples come out of [`next`](Self::next), in the order the reader
///   completes them — for a fragment followed by the media data it
///   addresses, as the segment lays them down. The brands the reader read are
///   there once they have come: [`segment_type`](Self::segment_type); the
///   movie the segment continues at [`movie`](Self::movie).
/// * The bytes fetched for a want come off the source a cut at a time, as
///   the segment does, so every extent it reaches is filled by it; a want the
///   read did not cover whole — one longer than a cut, or one the source
///   handed over in pieces — is named again by the reader and fetched again.
///   A source ending before bytes the reader lacks is the reader's to report
///   at the end of the segment, as
///   [`Structure`](crate::ErrorKind::Structure); one ending before
///   where it had already been read to is
///   [`Io`](crate::ErrorKind::Io) with
///   [`UnexpectedEof`](std::io::ErrorKind::UnexpectedEof).
/// * A failure ends the samples: the ones the reader had completed before it
///   come first, then the failure once, then `None` for good. The end of the
///   segment is the same without the failure.
/// * Every `async fn` here is cancellation safe: a future dropped where the
///   source stood still is carried on by the call that follows.
///
/// # Examples
///
/// ```
/// use futures_executor::block_on;
/// use futures_util::io::Cursor;
///
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::{MediaSegmentDemuxer, MediaSegmentMuxer};
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{fragmented_movie, segment_type};
/// block_on(async {
///     // A segment of one fragment carrying two samples of track 1
///     let mut segment = Vec::new();
///     let mut muxer = MediaSegmentMuxer::new(&mut segment);
///     muxer.handle_segment_type(segment_type()).await?;
///     muxer.begin_fragment(1).await?;
///     muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())).await?;
///     muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec())).await?;
///     muxer.finish_fragment().await?;
///     muxer.finish().await?;
///
///     // The samples are read off the segment as they were laid out
///     let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO));
///     let mut demuxer = MediaSegmentDemuxer::new(Cursor::new(segment), movie).await?;
///     let mut read_back = Vec::new();
///     while let Some(sample) = demuxer.next().await {
///         read_back.push(sample?.into_data());
///     }
///     assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
///
///     // The brands the segment declared are there to read
///     assert_eq!(demuxer.segment_type().map(|styp| styp.major_brand()), Some(segment_type().major_brand()));
/// #   Ok::<(), isobmff_io::Error>(())
/// })
/// # .unwrap();
/// ```
#[derive(Debug)]
pub struct MediaSegmentDemuxer<S> {
    demuxer: Demuxer<S, MediaSegmentReader>,
}

impl<S: AsyncRead + AsyncSeek + Unpin> MediaSegmentDemuxer<S> {
    /// Creates a demuxer over `source`, the segment continuing `movie` and beginning where the source stands
    ///
    /// The reader beneath is [`MediaSegmentReader::new`]; one holding the
    /// segment to other limits is driven through
    /// [`with_reader`](Self::with_reader).
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub async fn new(source: S, movie: MovieBox) -> Result<Self, Error> {
        Self::with_reader(source, MediaSegmentReader::new(movie)).await
    }

    /// Creates a demuxer over `source` driving `reader`, the segment beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub async fn with_reader(source: S, reader: MediaSegmentReader) -> Result<Self, Error> {
        Ok(Self {
            demuxer: Demuxer::new(source, reader).await?,
        })
    }

    /// Returns the brands the segment declares itself readable as, once they have come
    #[must_use]
    pub const fn segment_type(&self) -> Option<&SegmentTypeBox> {
        self.demuxer.reader().segment_type()
    }

    /// Returns the movie the fragments of the segment continue
    #[must_use]
    pub const fn movie(&self) -> &MovieBox {
        self.demuxer.reader().movie()
    }

    /// Takes the next sample the segment carries, reading on until one comes
    pub async fn next(&mut self) -> Option<Result<Sample, Error>> {
        self.demuxer.next().await
    }
}

impl ReadSamples for MediaSegmentReader {
    fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error> {
        MediaSegmentReader::handle_input(self, input)
    }

    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), isobmff_structure::Error> {
        MediaSegmentReader::handle_data(self, offset, data)
    }

    fn poll_sample(&mut self) -> Option<Sample> {
        MediaSegmentReader::poll_sample(self)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        MediaSegmentReader::wanted_extent(self)
    }

    fn finish(&mut self) -> Result<(), isobmff_structure::Error> {
        MediaSegmentReader::finish(self)
    }
}

/// Lays a media segment down on an asynchronous sink, taking the samples as they come
///
/// The driver of [`MediaSegmentWriter`] over `futures::io`: it takes the
/// brands and the samples as the writer does, and writes every byte the
/// writer makes of them to the sink before the call returns. A caller hands
/// over brands and samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`MediaSegmentWriter`]'s contract, carried through as
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
    muxer: Muxer<W, MediaSegmentWriter>,
}

impl<W: AsyncWrite + Unpin> MediaSegmentMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a media segment
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, MediaSegmentWriter::new()),
        }
    }

    /// Takes the brands the segment declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentWriter::handle_segment_type`] makes of the call.
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
    ///   [`MediaSegmentWriter::begin_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses bytes a
    ///   dropped call left over.
    pub async fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment(sequence_number))
            .await
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentWriter::handle_sample`] makes of the call.
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
    ///   [`MediaSegmentWriter::finish_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub async fn finish_fragment(&mut self) -> Result<(), Error> {
        self.muxer.drive(MediaSegmentWriter::finish_fragment).await
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
    ///   [`MediaSegmentWriter::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink does not flush.
    pub async fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(MediaSegmentWriter::finish).await
    }
}

impl PollOutput for MediaSegmentWriter {
    fn poll_output(&mut self) -> Option<EventBytes> {
        MediaSegmentWriter::poll_output(self)
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
