//! [`FragmentedDemuxer`], a fragmented movie file read off a source that seeks

use core::ops::Range;
use std::io::{Read, Seek};

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;

use super::FragmentedReader;
use crate::{Demuxing, DriverError, ReadSamples, StructureError};

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
/// * The bytes fetched for a want come off the source a cut at a time, as
///   the file does — a cut reaching at least to the end of the want — so
///   every extent the cut reaches is filled by it. A
///   source ending before bytes the reader lacks is the reader's to report
///   at the end of the file, as
///   [`Structure`](crate::DriverErrorKind::Structure); one ending before
///   where it had already been read to is [`Io`](crate::DriverErrorKind::Io)
///   with [`UnexpectedEof`](std::io::ErrorKind::UnexpectedEof).
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
    demuxing: Demuxing<S, FragmentedReader>,
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
    pub fn with_reader(source: S, reader: FragmentedReader) -> Result<Self, DriverError> {
        Ok(Self {
            demuxing: Demuxing::new(source, reader)?,
        })
    }

    /// Returns the brands the file declares itself readable as, once they have come
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.demuxing.reader().file_type()
    }

    /// Returns the movie the fragments of the file continue, once it has come
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.demuxing.reader().movie()
    }
}

impl<S: Read + Seek> Iterator for FragmentedDemuxer<S> {
    type Item = Result<Sample, DriverError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.demuxing.next()
    }
}

impl ReadSamples for FragmentedReader {
    fn handle_input(&mut self, input: &[u8]) -> Result<(), StructureError> {
        FragmentedReader::handle_input(self, input)
    }

    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), StructureError> {
        FragmentedReader::handle_data(self, offset, data)
    }

    fn poll_sample(&mut self) -> Option<Sample> {
        FragmentedReader::poll_sample(self)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        FragmentedReader::wanted_extent(self)
    }

    fn finish(&mut self) -> Result<(), StructureError> {
        FragmentedReader::finish(self)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use std::io;

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBox, TrackFragmentHeaderBox,
        TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
    };
    use isobmff_test_support::written;

    use super::super::tests::{file_of_one_sample, sample};
    use super::FragmentedDemuxer;
    use crate::Sample;

    /// The file of one sample, followed by a fragment addressing that sample's bytes again
    fn file_addressing_back() -> Vec<u8> {
        let mut file = file_of_one_sample();
        let media_data_start = file.len().saturating_sub(4) as u64;
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::ZERO,
                1,
                Some(media_data_start),
                None,
                None,
                Some(4),
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
        file.extend_from_slice(&written(&fragment));

        file
    }

    #[test]
    fn a_fragment_addressing_media_data_before_it_has_the_bytes_fetched() {
        let read_back: Vec<Sample> =
            FragmentedDemuxer::new(io::Cursor::new(file_addressing_back()))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        assert_eq!(
            read_back,
            [
                sample(),
                Sample::new(1, 1_024, 1_024, 0, 0, 1, b"SAMP".to_vec())
            ]
        );
    }
}
