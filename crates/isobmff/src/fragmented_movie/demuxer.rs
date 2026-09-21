//! [`FragmentedDemuxer`], a fragmented movie file read off a source that seeks

use alloc::vec::Vec;
use std::io::{self, Read, Seek, SeekFrom};

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;

use super::FragmentedReader;
use crate::DriverError;

/// Bytes handed over to the reader at a time
const CUT_LENGTH: u64 = 1024 * 1024;

/// Reads the samples a fragmented movie file carries off a source that seeks
///
/// The driver of [`FragmentedReader`] over `std::io`: it reads the file off
/// the source a cut at a time and hands each over, fetches the bytes the
/// reader names as lacking wherever the file passed them by — the media data
/// of a fragment addressing bytes before it — by seeking to them, and yields
/// the samples as they come whole. A caller takes [`Sample`]s and nothing
/// else moves.
///
/// # Contract
///
/// * The file begins where the source stands when the demuxer is created,
///   and every seek is made from there: a file lying at some position in a
///   larger resource is read by seeking the source to it first.
/// * The samples come as `Iterator` items, in the order the reader completes
///   them — for a fragment followed by the media data it addresses, as the
///   file lays them down. The boxes the reader read into values are there to
///   read once they have come: [`file_type`](Self::file_type) and
///   [`movie`](Self::movie).
/// * The bytes fetched for a want are read off the source whole; a source
///   ending before them is [`Io`](crate::DriverErrorKind::Io) with
///   [`UnexpectedEof`](io::ErrorKind::UnexpectedEof). A source ending before
///   a want lying ahead is the reader's to report at the end of the file, as
///   [`Structure`](crate::DriverErrorKind::Structure).
/// * A failure ends the iteration: the samples the reader had completed
///   before it come first, then the failure once, then `None` for good. The
///   end of the file is the same without the failure.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff::{FragmentedDemuxer, FragmentedMuxer, Sample, TrackExtendsBox};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying two samples of track 1
/// let mut file = Vec::new();
/// let mut muxer = FragmentedMuxer::new(&mut file);
/// muxer.handle_file_type(file_type())?;
/// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0)))?;
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The samples are read off the file as they were laid out
/// let mut demuxer = FragmentedDemuxer::new(Cursor::new(file))?;
/// let mut read_back = Vec::new();
/// for sample in &mut demuxer {
///     read_back.push(sample?.into_data());
/// }
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
///
/// // The brands and the movie the file declared are there to read
/// assert_eq!(demuxer.file_type().map(|ftyp| ftyp.major_brand()), Some(file_type().major_brand()));
/// assert_eq!(demuxer.movie().map(|moov| moov.trak().len()), Some(1));
/// # Ok::<(), isobmff::DriverError>(())
/// ```
#[derive(Debug)]
pub struct FragmentedDemuxer<S> {
    source: S,
    reader: FragmentedReader,
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

impl<S: Read + Seek> FragmentedDemuxer<S> {
    /// Creates a demuxer over `source`, the file beginning where it stands
    ///
    /// The reader beneath is [`FragmentedReader::new`]; one holding the file
    /// to other limits is driven through [`with_reader`](Self::with_reader).
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::DriverErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn new(source: S) -> Result<Self, DriverError> {
        Self::with_reader(source, FragmentedReader::new())
    }

    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::DriverErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn with_reader(mut source: S, reader: FragmentedReader) -> Result<Self, DriverError> {
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

    /// Returns the movie the fragments of the file continue, once it has come
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
            // holds is reported as the source ending, not as arithmetic.
            self.source
                .seek(SeekFrom::Start(self.origin.saturating_add(wanted.start)))?;
            let length = wanted.end.saturating_sub(wanted.start);
            if self.read_cut(length)? < length {
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

impl<S: Read + Seek> Iterator for FragmentedDemuxer<S> {
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
    use std::io;

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, TrackExtendsBox, TrackFragmentBox,
        TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
    };
    use isobmff_test_support::{fragmented_movie, written};

    use super::FragmentedDemuxer;
    use crate::{DriverErrorKind, FragmentedWriter, Sample, StructureErrorKind};

    /// A file of one fragment carrying `sample`, with no brands
    fn file_of(sample: Sample) -> Vec<u8> {
        let mut writer = FragmentedWriter::new();
        let mut file = Vec::new();

        writer
            .handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0)))
            .unwrap();
        writer.begin_fragment(1).unwrap();
        writer.handle_sample(sample).unwrap();
        writer.finish_fragment().unwrap();
        writer.finish().unwrap();
        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    /// A fragment of one sample of `size` bytes, lying at `base_data_offset` in the file
    fn fragment_addressing(base_data_offset: u64, size: u32) -> MovieFragmentBox {
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::ZERO,
                1,
                Some(base_data_offset),
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

        MovieFragmentBox::new(MovieFragmentHeaderBox::new(2), vec![track_fragment])
    }

    /// What the demuxer yields off `file`, kind for kind, until it is over
    fn yielded(file: Vec<u8>) -> Vec<Result<Sample, DriverErrorKind>> {
        FragmentedDemuxer::new(io::Cursor::new(file))
            .unwrap()
            .map(|sample| sample.map_err(|failure| failure.kind()))
            .collect()
    }

    #[test]
    fn a_fragment_addressing_media_data_before_it_has_the_bytes_fetched() {
        let first = Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec());
        let mut file = file_of(first.clone());
        let media_data_start = file.len().saturating_sub(4) as u64;
        file.extend_from_slice(&written(&fragment_addressing(media_data_start, 4)));

        assert_eq!(
            yielded(file),
            [
                Ok(first),
                Ok(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"SAMP".to_vec()))
            ]
        );
    }

    #[test]
    fn the_file_begins_where_the_source_stands() {
        let sample = Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec());
        let mut file = file_of(sample.clone());
        let media_data_start = file.len().saturating_sub(4) as u64;
        file.extend_from_slice(&written(&fragment_addressing(media_data_start, 4)));
        let mut source = io::Cursor::new([b"junk".as_slice(), &file].concat());
        source.set_position(4);

        let read_back: Vec<Sample> = FragmentedDemuxer::new(source)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(
            read_back,
            [
                sample,
                Sample::new(1, 1_024, 1_024, 0, 0, 1, b"SAMP".to_vec())
            ]
        );
    }

    #[test]
    fn a_source_ending_before_the_bytes_the_reader_lacks_is_reported_as_such() {
        let first = Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec());
        let mut file = file_of(first.clone());
        let media_data_start = file.len().saturating_sub(4) as u64;
        file.extend_from_slice(&written(&fragment_addressing(media_data_start, 4_096)));

        assert_eq!(
            yielded(file),
            [
                Ok(first),
                Err(DriverErrorKind::Io(io::ErrorKind::UnexpectedEof))
            ]
        );
    }

    #[test]
    fn the_samples_completed_before_a_failure_come_first_and_the_failure_once() {
        let sample = Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec());
        let mut file = file_of(sample.clone());
        file.extend_from_slice(b"\0\0\0\x04free");
        let mut demuxer = FragmentedDemuxer::new(io::Cursor::new(file)).unwrap();

        let until_over: Vec<_> = demuxer
            .by_ref()
            .map(|sample| sample.map_err(|failure| failure.kind()))
            .collect();

        assert_eq!(
            until_over,
            [
                Ok(sample),
                Err(DriverErrorKind::Structure(StructureErrorKind::Sequence(
                    isobmff_sequence::ErrorKind::Box(isobmff_core::ErrorKind::SizeBelowHeader)
                )))
            ]
        );
        assert!(demuxer.next().is_none());
    }

    #[test]
    fn a_source_ending_inside_the_media_data_is_the_readers_to_report() {
        let mut file = file_of(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()));
        file.truncate(file.len().saturating_sub(2));

        assert_eq!(
            yielded(file),
            [Err(DriverErrorKind::Structure(
                StructureErrorKind::Sequence(isobmff_sequence::ErrorKind::UnfinishedBox)
            ))]
        );
    }
}
