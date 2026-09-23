//! [`MediaSegmentDemuxer`] and [`MediaSegmentMuxer`], a media segment read off a source that seeks and written to a sink, ISO/IEC 14496-12 §8.16

use std::io::{Read, Seek, Write};

use isobmff_boxes::{MovieBox, SegmentTypeBox};
use isobmff_sample::{Sample, SegmentIndex};
use isobmff_structure::{MediaSegmentReader, MediaSegmentWriter};

use super::driver::{Demuxer, Muxer};
use crate::Error;

/// Reads the samples a media segment carries off a source that seeks
///
/// The driver of [`MediaSegmentReader`] over `std::io`: it reads the segment
/// off the source a cut at a time and hands each over, fetches the bytes the
/// reader names as lacking wherever the segment passed them by — the media
/// data of a fragment addressing bytes before it — by seeking to them, and
/// yields the samples as they come whole. A caller takes [`Sample`]s and
/// nothing else moves.
///
/// # Contract
///
/// * The segment begins where the source stands when the demuxer is created,
///   and every seek is made from there: a segment lying at some position in a
///   larger resource is read by seeking the source to it first.
/// * The samples come as `Iterator` items, in the order the reader completes
///   them — for a fragment followed by the media data it addresses, as the
///   segment lays them down. The brands the reader read are there once they
///   have come: [`segment_type`](Self::segment_type); the movie the segment
///   continues at [`movie`](Self::movie).
/// * The bytes fetched for a want come off the source a cut at a time, as
///   the segment does — a cut reaching at least to the end of the want — so
///   every extent the cut reaches is filled by it. A source ending before
///   bytes the reader lacks is the reader's to report at the end of the
///   segment, as [`Structure`](crate::ErrorKind::Structure); one
///   ending before where it had already been read to is
///   [`Io`](crate::ErrorKind::Io) with
///   [`UnexpectedEof`](std::io::ErrorKind::UnexpectedEof).
/// * A failure ends the iteration: the samples the reader had completed
///   before it come first, then the failure once, then `None` until the
///   reading is resumed. The end of the segment is the same
///   without the failure.
/// * Where an index points is the caller's to choose. The indexes the
///   segment carries are there to read once they have come:
///   [`segment_indexes`](Self::segment_indexes).
///   [`resume_at`](Self::resume_at) restarts the reading at an offset one of
///   them names.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::blocking::{MediaSegmentDemuxer, MediaSegmentMuxer};
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{fragmented_movie, segment_type};
/// // A segment of one fragment carrying two samples of track 1
/// let mut segment = Vec::new();
/// let mut muxer = MediaSegmentMuxer::new(&mut segment);
/// muxer.handle_segment_type(segment_type())?;
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The samples are read off the segment as they were laid out
/// let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO));
/// let mut demuxer = MediaSegmentDemuxer::new(Cursor::new(segment), movie)?;
/// let mut read_back = Vec::new();
/// for sample in &mut demuxer {
///     read_back.push(sample?.into_data());
/// }
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
///
/// // The brands the segment declared are there to read
/// assert_eq!(demuxer.segment_type().map(|styp| styp.major_brand()), Some(segment_type().major_brand()));
/// # Ok::<(), isobmff_io::Error>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentDemuxer<S> {
    demuxer: Demuxer<S, MediaSegmentReader>,
}

impl<S: Read + Seek> MediaSegmentDemuxer<S> {
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
    pub fn new(source: S, movie: MovieBox) -> Result<Self, Error> {
        Self::with_reader(source, MediaSegmentReader::new(movie))
    }

    /// Creates a demuxer over `source` driving `reader`, the segment beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn with_reader(source: S, reader: MediaSegmentReader) -> Result<Self, Error> {
        Ok(Self {
            demuxer: Demuxer::new(source, reader)?,
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

    /// Returns the subsegments of every `sidx` read so far, as [`MediaSegmentReader::segment_indexes`] holds them
    #[must_use]
    pub fn segment_indexes(&self) -> &[SegmentIndex] {
        self.demuxer.reader().segment_indexes()
    }

    /// Restarts the reading at `offset` of the segment, a place an index names
    ///
    /// The reader is resumed at `offset` as [`MediaSegmentReader::resume_at`]
    /// resumes it, and the source is sought there from where the segment
    /// begins: the samples not yet taken are dropped, and the ones that
    /// come next are those the segment carries from `offset` on — the `moof`
    /// or `sidx` it is to start with. The demuxer resumes from reading and
    /// from the end of the segment alike, and from a failure of the source.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): `offset`, counted from where the
    ///   source stood when the demuxer was created, lies past `u64::MAX`,
    ///   which leaves the demuxer as it was, or the source does not seek
    ///   there.
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentReader::resume_at`] makes of the call.
    ///
    /// A failure after the offset is checked ends the samples, until a
    /// resume succeeds.
    pub fn resume_at(&mut self, offset: u64) -> Result<(), Error> {
        self.demuxer.resume_at(offset)
    }
}

impl<S: Read + Seek> Iterator for MediaSegmentDemuxer<S> {
    type Item = Result<Sample, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.demuxer.next()
    }
}

/// Lays a media segment down on a sink, taking the samples as they come
///
/// The driver of [`MediaSegmentWriter`] over `std::io`: it takes the brands
/// and the samples as the writer does, and writes every byte the writer makes
/// of them to the sink before the call returns. A caller hands over brands
/// and samples and nothing else moves.
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
///   pieces is the caller's to wrap in a `BufWriter`.
/// * [`finish`](Self::finish) declares the segment over and flushes the sink.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::SampleFlags;
/// use isobmff_io::blocking::MediaSegmentMuxer;
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::segment_type;
/// // A segment opening with its brands
/// let mut segment = Vec::new();
/// let mut muxer = MediaSegmentMuxer::new(&mut segment);
/// muxer.handle_segment_type(segment_type())?;
///
/// // One fragment of two samples of track 1, written to the segment as it is closed
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The segment opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&segment[4..8], b"styp");
/// assert!(segment.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff_io::Error>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentMuxer<W> {
    muxer: Muxer<W, MediaSegmentWriter>,
}

impl<W: Write> MediaSegmentMuxer<W> {
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
    pub fn handle_segment_type(&mut self, segment_type: SegmentTypeBox) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_segment_type(segment_type))
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentWriter::begin_fragment`] makes of the call.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment(sequence_number))
    }

    /// Opens a fragment in which every track continues where the samples written for it reach
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentWriter::begin_fragment_continuing`] makes of the call.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment_continuing(sequence_number))
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentWriter::handle_sample`] makes of the call.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_sample(sample))
    }

    /// Closes the fragment that is open, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentWriter::finish_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.muxer.drive(MediaSegmentWriter::finish_fragment)
    }

    /// Declares the segment over, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentWriter::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink does not flush.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(MediaSegmentWriter::finish)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use std::io;

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, SampleFlags, TrackFragmentBox,
        TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
    };
    use isobmff_sample::Sample;
    use isobmff_test_support::{
        presentation_movie, segment_file_samples, segment_file_with_samples, segment_type, written,
    };

    use super::{MediaSegmentDemuxer, MediaSegmentMuxer};

    /// The media segment, followed by a fragment addressing the bytes of its last sample again
    fn segment_addressing_back() -> Vec<u8> {
        let mut segment = segment_file_with_samples();
        let sample_len = segment_file_samples()
            .last()
            .map(|sample| sample.data().len())
            .unwrap();
        let media_data_start = u64::try_from(segment.len().saturating_sub(sample_len)).unwrap();
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::ZERO,
                1,
                Some(media_data_start),
                None,
                None,
                Some(u32::try_from(sample_len).unwrap()),
                None,
            ),
            vec![
                TrackRunBox::new(
                    Some(0),
                    None,
                    vec![TrackRunSample::new(None, None, None, None)],
                )
                .unwrap(),
            ],
        );
        let fragment = MovieFragmentBox::new(MovieFragmentHeaderBox::new(3), vec![track_fragment]);
        segment.extend_from_slice(&written(&fragment));

        segment
    }

    #[test]
    fn a_fragment_addressing_media_data_before_it_has_the_bytes_fetched() {
        let read_back: Vec<Sample> = MediaSegmentDemuxer::new(
            io::Cursor::new(segment_addressing_back()),
            presentation_movie(),
        )
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

        let mut expected = segment_file_samples();
        let repeated = expected.last().unwrap();
        let read_again = Sample::new(
            1,
            repeated
                .decode_time()
                .saturating_add(u64::from(repeated.sample_duration())),
            repeated.sample_duration(),
            0,
            SampleFlags::ZERO,
            1,
            repeated.data().to_vec(),
        );
        expected.push(read_again);

        assert_eq!(read_back, expected);
    }

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut segment = Vec::new();
        let mut muxer = MediaSegmentMuxer::new(&mut segment);

        muxer.handle_segment_type(segment_type()).unwrap();

        assert_eq!(segment, written(&segment_type()));
    }
}
