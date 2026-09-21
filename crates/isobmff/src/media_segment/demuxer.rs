//! [`MediaSegmentDemuxer`], a media segment read off a source that seeks

use alloc::vec::Vec;
use std::io::{self, Read, Seek, SeekFrom};

use isobmff_boxes::{MovieBox, SegmentTypeBox};
use isobmff_sample::Sample;

use super::MediaSegmentReader;
use crate::DriverError;

/// Bytes handed over to the reader at a time
const CUT_LENGTH: u64 = 1024 * 1024;

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
///   segment, as [`Structure`](crate::DriverErrorKind::Structure); one
///   ending before where it had already been read to is
///   [`Io`](crate::DriverErrorKind::Io) with
///   [`UnexpectedEof`](io::ErrorKind::UnexpectedEof).
/// * A failure ends the iteration: the samples the reader had completed
///   before it come first, then the failure once, then `None` for good. The
///   end of the segment is the same without the failure.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff::{MediaSegmentDemuxer, MediaSegmentMuxer, Sample, TrackExtendsBox};
/// # use isobmff_test_support::{fragmented_movie, segment_type};
/// // A segment of one fragment carrying two samples of track 1
/// let mut segment = Vec::new();
/// let mut muxer = MediaSegmentMuxer::new(&mut segment);
/// muxer.handle_segment_type(segment_type())?;
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The samples are read off the segment as they were laid out
/// let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0));
/// let mut demuxer = MediaSegmentDemuxer::new(Cursor::new(segment), movie)?;
/// let mut read_back = Vec::new();
/// for sample in &mut demuxer {
///     read_back.push(sample?.into_data());
/// }
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
///
/// // The brands the segment declared are there to read
/// assert_eq!(demuxer.segment_type().map(|styp| styp.major_brand()), Some(segment_type().major_brand()));
/// # Ok::<(), isobmff::DriverError>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentDemuxer<S> {
    source: S,
    reader: MediaSegmentReader,
    cut: Vec<u8>,
    origin: u64,
    handed: u64,
    state: State,
}

/// Where the demuxer stands between samples
#[derive(Debug)]
enum State {
    /// Reading the segment off the source
    Reading,
    /// Over, holding the failure still to report if it ended in one
    Over(Option<DriverError>),
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
    /// * [`Io`](crate::DriverErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn new(source: S, movie: MovieBox) -> Result<Self, DriverError> {
        Self::with_reader(source, MediaSegmentReader::new(movie))
    }

    /// Creates a demuxer over `source` driving `reader`, the segment beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::DriverErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn with_reader(mut source: S, reader: MediaSegmentReader) -> Result<Self, DriverError> {
        let origin = source.stream_position()?;

        Ok(Self {
            source,
            reader,
            cut: Vec::new(),
            origin,
            handed: 0,
            state: State::Reading,
        })
    }

    /// Returns the brands the segment declares itself readable as, once they have come
    #[must_use]
    pub const fn segment_type(&self) -> Option<&SegmentTypeBox> {
        self.reader.segment_type()
    }

    /// Returns the movie the fragments of the segment continue
    #[must_use]
    pub const fn movie(&self) -> &MovieBox {
        self.reader.movie()
    }

    /// Reads on: fetches what the reader lacks if the segment passed it by, else hands over the next cut
    fn read_on(&mut self) -> Result<(), DriverError> {
        let passed_by = self
            .reader
            .wanted_extent()
            .filter(|wanted| wanted.start < self.handed);
        if let Some(wanted) = passed_by {
            // Why not checked_add: the want lies before `handed`, a position
            // the source already stood at, so neither sum can run past what
            // 64 bits carry.
            self.source
                .seek(SeekFrom::Start(self.origin.saturating_add(wanted.start)))?;
            // Why not the want alone: a fragment holds every extent it
            // addresses at once, and a cut read from the first fills the ones
            // behind it too, where fetching them one at a time costs a seek
            // and a sweep of the extents held per sample.
            let length = wanted.end.saturating_sub(wanted.start).max(CUT_LENGTH);
            if self.read_cut(length)? == 0 {
                // Why not carrying on: the want lies before what was handed
                // over in order, so a source holding nothing there has shrunk
                // since, and reading on would ask for the same bytes without end.
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            self.reader.handle_data(wanted.start, &self.cut)?;
            self.source
                .seek(SeekFrom::Start(self.origin.saturating_add(self.handed)))?;

            return Ok(());
        }

        let read = self.read_cut(CUT_LENGTH)?;
        if read == 0 {
            self.reader.finish()?;
            self.state = State::Over(None);
        } else {
            self.reader.handle_input(&self.cut)?;
            self.handed = self.handed.saturating_add(read);
        }

        Ok(())
    }

    /// Reads up to `length` bytes off the source into the cut, and returns how many came
    fn read_cut(&mut self, length: u64) -> io::Result<u64> {
        self.cut.clear();
        let read = self
            .source
            .by_ref()
            .take(length)
            .read_to_end(&mut self.cut)?;

        Ok(read as u64)
    }
}

impl<S: Read + Seek> Iterator for MediaSegmentDemuxer<S> {
    type Item = Result<Sample, DriverError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.reader.poll_sample() {
                return Some(Ok(sample));
            }
            if let State::Over(failure) = &mut self.state {
                return failure.take().map(Err);
            }
            if let Err(failure) = self.read_on() {
                self.state = State::Over(Some(failure));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use std::io::{self, Read, Seek, SeekFrom};

    use isobmff_boxes::{
        MovieBox, MovieFragmentBox, MovieFragmentHeaderBox, TrackExtendsBox, TrackFragmentBox,
        TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
    };
    use isobmff_test_support::{fragmented_movie, written};

    use super::super::tests::{sample, segment_of_one_sample};
    use super::MediaSegmentDemuxer;
    use crate::{DriverErrorKind, Sample, StructureErrorKind};

    /// Source holding nothing past any position it is sought back to
    struct Shrinking(io::Cursor<Vec<u8>>);

    impl Read for Shrinking {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            self.0.read(into)
        }
    }

    impl Seek for Shrinking {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if let SeekFrom::Start(position) = from {
                if position < self.0.position() {
                    self.0
                        .get_mut()
                        .truncate(usize::try_from(position).unwrap());
                }
            }

            self.0.seek(from)
        }
    }

    /// Movie of one track continued in fragments, whose defaults a `trex` states
    fn movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0))
    }

    /// The segment of one sample, followed by a fragment addressing `size` bytes at that sample's start
    fn segment_addressing_back(size: u32) -> Vec<u8> {
        let mut segment = segment_of_one_sample();
        let media_data_start = segment.len().saturating_sub(4) as u64;
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::ZERO,
                1,
                Some(media_data_start),
                None,
                None,
                Some(size),
                None,
            ),
            None,
            vec![
                TrackRunBox::new(
                    Some(0),
                    None,
                    vec![TrackRunSample::new(None, None, None, None)],
                )
                .unwrap(),
            ],
        );
        let fragment = MovieFragmentBox::new(MovieFragmentHeaderBox::new(2), vec![track_fragment]);
        segment.extend_from_slice(&written(&fragment));

        segment
    }

    /// What `demuxer` yields, kind for kind, until it is over
    fn yielded(
        demuxer: &mut MediaSegmentDemuxer<impl Read + Seek>,
    ) -> Vec<Result<Sample, DriverErrorKind>> {
        demuxer
            .map(|sample| sample.map_err(|failure| failure.kind()))
            .collect()
    }

    #[test]
    fn a_fragment_addressing_media_data_before_it_has_the_bytes_fetched() {
        let mut demuxer =
            MediaSegmentDemuxer::new(io::Cursor::new(segment_addressing_back(4)), movie()).unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(sample()),
                Ok(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"SAMP".to_vec()))
            ]
        );
    }

    #[test]
    fn the_segment_begins_where_the_source_stands() {
        let mut source =
            io::Cursor::new([b"junk".as_slice(), &segment_addressing_back(4)].concat());
        source.set_position(4);
        let mut demuxer = MediaSegmentDemuxer::new(source, movie()).unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(sample()),
                Ok(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"SAMP".to_vec()))
            ]
        );
    }

    #[test]
    fn a_source_ending_before_the_bytes_the_reader_lacks_is_the_readers_to_report() {
        let mut demuxer =
            MediaSegmentDemuxer::new(io::Cursor::new(segment_addressing_back(4_096)), movie())
                .unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(sample()),
                Err(DriverErrorKind::Structure(StructureErrorKind::Sample(
                    isobmff_sample::SampleErrorKind::UnfinishedSample
                )))
            ]
        );
    }

    #[test]
    fn a_source_shrunk_below_what_was_read_is_reported_as_ending() {
        let mut demuxer = MediaSegmentDemuxer::new(
            Shrinking(io::Cursor::new(segment_addressing_back(4))),
            movie(),
        )
        .unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(sample()),
                Err(DriverErrorKind::Io(io::ErrorKind::UnexpectedEof))
            ]
        );
    }

    #[test]
    fn the_samples_completed_before_a_failure_come_first_and_the_failure_once() {
        let mut segment = segment_of_one_sample();
        segment.extend_from_slice(b"\0\0\0\x04free");
        let mut demuxer = MediaSegmentDemuxer::new(io::Cursor::new(segment), movie()).unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(sample()),
                Err(DriverErrorKind::Structure(StructureErrorKind::Sequence(
                    isobmff_sequence::ErrorKind::Box(isobmff_core::ErrorKind::SizeBelowHeader)
                )))
            ]
        );
        assert!(demuxer.next().is_none());
    }
}
