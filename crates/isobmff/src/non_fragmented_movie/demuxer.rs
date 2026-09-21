//! [`NonFragmentedDemuxer`], a non-fragmented movie file read off a source that seeks

use alloc::vec::Vec;
use std::io::{self, Read, Seek, SeekFrom};

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;

use super::NonFragmentedReader;
use crate::DriverError;

/// Bytes handed over to the reader at a time
const CUT_LENGTH: u64 = 1024 * 1024;

/// Reads the samples a non-fragmented movie file carries off a source that seeks
///
/// The driver of [`NonFragmentedReader`] over `std::io`: it reads the file off
/// the source a cut at a time and hands each over, fetches the bytes the
/// reader names as lacking wherever the file passed them by — the media data
/// of a movie lying after it — by seeking to them, and yields the samples as
/// they come whole. A caller takes [`Sample`]s and nothing else moves.
///
/// # Contract
///
/// * The file begins where the source stands when the demuxer is created,
///   and every seek is made from there: a file lying at some position in a
///   larger resource is read by seeking the source to it first.
/// * The samples come as `Iterator` items, in the order the reader completes
///   them — a movie lying before its media data has them come as the file
///   lays them down; one lying after it has them come in the order the
///   reader holds their extents, as the bytes fetched for the extent held
///   longest complete them. The boxes the reader read into values are there
///   to read once they have come: [`file_type`](Self::file_type) and
///   [`movie`](Self::movie).
/// * The bytes fetched for a want come off the source a cut at a time, as
///   the file does, so every extent the cut reaches is filled by it. A
///   source ending before bytes the reader lacks is the reader's to report
///   at the end of the file, as
///   [`Structure`](crate::DriverErrorKind::Structure); one ending before
///   where it had already been read to is [`Io`](crate::DriverErrorKind::Io)
///   with [`UnexpectedEof`](io::ErrorKind::UnexpectedEof).
/// * A failure ends the iteration: the samples the reader had completed
///   before it come first, then the failure once, then `None` for good. The
///   end of the file is the same without the failure.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff::NonFragmentedDemuxer;
/// # use isobmff_test_support::non_fragmented_file;
/// // A file of two chunks of one track, its movie lying after its media data
/// let file = non_fragmented_file(&[&[b"SAMP", b"DATA"], &[b"LAST"]], false);
///
/// // The samples are read off the file, the bytes each lacks fetched by seeking
/// let mut demuxer = NonFragmentedDemuxer::new(Cursor::new(file))?;
/// let mut read_back = Vec::new();
/// for sample in &mut demuxer {
///     read_back.push(sample?.into_data());
/// }
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec(), b"LAST".to_vec()]);
///
/// // The movie the file declared is there to read
/// assert_eq!(demuxer.movie().map(|moov| moov.trak().len()), Some(1));
/// # Ok::<(), isobmff::DriverError>(())
/// ```
#[derive(Debug)]
pub struct NonFragmentedDemuxer<S> {
    source: S,
    reader: NonFragmentedReader,
    cut: Vec<u8>,
    origin: u64,
    handed: u64,
    state: State,
}

/// Where the demuxer stands between samples
#[derive(Debug)]
enum State {
    /// Reading the file off the source
    Reading,
    /// Over, holding the failure still to report if it ended in one
    Over(Option<DriverError>),
}

impl<S: Read + Seek> NonFragmentedDemuxer<S> {
    /// Creates a demuxer over `source`, the file beginning where it stands
    ///
    /// The reader beneath is [`NonFragmentedReader::new`]; one holding the
    /// file to other limits is driven through
    /// [`with_reader`](Self::with_reader).
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::DriverErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn new(source: S) -> Result<Self, DriverError> {
        Self::with_reader(source, NonFragmentedReader::new())
    }

    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::DriverErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn with_reader(mut source: S, reader: NonFragmentedReader) -> Result<Self, DriverError> {
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

    /// Returns the brands the file declares itself readable as, once they have come
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.reader.file_type()
    }

    /// Returns the movie whose sample tables declare the samples of the file, once it has come
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.reader.movie()
    }

    /// Reads on: fetches what the reader lacks if the file passed it by, else hands over the next cut
    fn read_on(&mut self) -> Result<(), DriverError> {
        let passed_by = self
            .reader
            .wanted_extent()
            .filter(|wanted| wanted.start < self.handed);
        if let Some(wanted) = passed_by {
            // Why not checked_add: the want names bytes of a file the reader
            // is reading off a finite source, and a want past what the source
            // holds is the reader's to report once the file is over.
            self.source
                .seek(SeekFrom::Start(self.origin.saturating_add(wanted.start)))?;
            // Why not the want alone: a movie lying after its media data
            // holds every extent at once, and a cut read from the first fills
            // the ones behind it too, where fetching them one at a time
            // costs a seek and a sweep of the extents held per sample.
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

impl<S: Read + Seek> Iterator for NonFragmentedDemuxer<S> {
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
        ChunkOffsetBox, MediaDataBox, MovieBox, MovieHeaderBox, SampleSizeBox, SampleToChunkBox,
        TimeToSampleBox,
    };
    use isobmff_core::Mp4EpochSeconds;
    use isobmff_test_support::{
        SAMPLE_DURATION, non_fragmented_file, sample_table, self_contained_data_reference,
        track_laid_out, written,
    };

    use super::NonFragmentedDemuxer;
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

    /// A file of one `mdat` of `SAMP`, its movie after it declaring one sample of `size` bytes there
    fn file_declaring_a_sample_of(size: u32) -> Vec<u8> {
        let media_data = written(&MediaDataBox::new(b"SAMP".to_vec()));
        let stbl = sample_table(
            TimeToSampleBox::from_deltas([SAMPLE_DURATION]),
            SampleToChunkBox::from_chunks([(1, 1)]).unwrap(),
            SampleSizeBox::from_sizes([size]),
            ChunkOffsetBox::from_offsets([8]).unwrap(),
        );
        let epoch = Mp4EpochSeconds::from_seconds(0);
        let movie = MovieBox::new(
            MovieHeaderBox::new(epoch, epoch, 90_000, 0, 2),
            vec![track_laid_out(1, self_contained_data_reference(), stbl)],
            None,
        )
        .unwrap();

        [media_data, written(&movie)].concat()
    }

    /// What `demuxer` yields, kind for kind, until it is over
    fn yielded(
        demuxer: &mut NonFragmentedDemuxer<impl Read + Seek>,
    ) -> Vec<Result<Vec<u8>, DriverErrorKind>> {
        demuxer
            .map(|sample| {
                sample
                    .map(Sample::into_data)
                    .map_err(|failure| failure.kind())
            })
            .collect()
    }

    #[test]
    fn the_file_begins_where_the_source_stands() {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);
        let mut source = io::Cursor::new([b"junk".as_slice(), &file].concat());
        source.set_position(4);

        let read_back: Vec<Sample> = NonFragmentedDemuxer::new(source)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

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
    fn a_source_ending_before_the_bytes_the_reader_lacks_is_the_readers_to_report() {
        let file = file_declaring_a_sample_of(4_096);
        let mut demuxer = NonFragmentedDemuxer::new(io::Cursor::new(file)).unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [Err(DriverErrorKind::Structure(StructureErrorKind::Sample(
                isobmff_sample::SampleErrorKind::UnfinishedSample
            )))]
        );
    }

    #[test]
    fn a_source_shrunk_below_what_was_read_is_reported_as_ending() {
        let file = file_declaring_a_sample_of(4);
        let mut demuxer = NonFragmentedDemuxer::new(Shrinking(io::Cursor::new(file))).unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [Err(DriverErrorKind::Io(io::ErrorKind::UnexpectedEof))]
        );
    }

    #[test]
    fn the_samples_completed_before_a_failure_come_first_and_the_failure_once() {
        let mut file = non_fragmented_file(&[&[b"SAMP"]], true);
        file.extend_from_slice(b"\0\0\0\x04free");
        let mut demuxer = NonFragmentedDemuxer::new(io::Cursor::new(file)).unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(b"SAMP".to_vec()),
                Err(DriverErrorKind::Structure(StructureErrorKind::Sequence(
                    isobmff_sequence::ErrorKind::Box(isobmff_core::ErrorKind::SizeBelowHeader)
                )))
            ]
        );
        assert!(demuxer.next().is_none());
    }

    #[test]
    fn a_source_ending_inside_the_media_data_is_the_readers_to_report() {
        let mut file = non_fragmented_file(&[&[b"SAMP"]], true);
        file.truncate(file.len().saturating_sub(2));
        let mut demuxer = NonFragmentedDemuxer::new(io::Cursor::new(file)).unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [Err(DriverErrorKind::Structure(
                StructureErrorKind::Sequence(isobmff_sequence::ErrorKind::UnfinishedBox)
            ))]
        );
    }
}
