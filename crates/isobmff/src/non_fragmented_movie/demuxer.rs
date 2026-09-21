//! [`NonFragmentedDemuxer`], a non-fragmented movie file read off a source that seeks

use core::ops::Range;
use std::io::{Read, Seek};

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;

use super::NonFragmentedReader;
use crate::{Demuxer, DriverError, ReadSamples, StructureError};

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
    demuxer: Demuxer<S, NonFragmentedReader>,
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
    pub fn with_reader(source: S, reader: NonFragmentedReader) -> Result<Self, DriverError> {
        Ok(Self {
            demuxer: Demuxer::new(source, reader)?,
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
}

impl<S: Read + Seek> Iterator for NonFragmentedDemuxer<S> {
    type Item = Result<Sample, DriverError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.demuxer.next()
    }
}

impl ReadSamples for NonFragmentedReader {
    fn handle_input(&mut self, input: &[u8]) -> Result<(), StructureError> {
        NonFragmentedReader::handle_input(self, input)
    }

    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), StructureError> {
        NonFragmentedReader::handle_data(self, offset, data)
    }

    fn poll_sample(&mut self) -> Option<Sample> {
        NonFragmentedReader::poll_sample(self)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        NonFragmentedReader::wanted_extent(self)
    }

    fn finish(&mut self) -> Result<(), StructureError> {
        NonFragmentedReader::finish(self)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use std::io;

    use isobmff_test_support::{SAMPLE_DURATION, non_fragmented_file};

    use super::NonFragmentedDemuxer;
    use crate::Sample;

    #[test]
    fn a_movie_lying_after_its_media_data_has_the_bytes_fetched() {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);

        let read_back: Vec<Sample> = NonFragmentedDemuxer::new(io::Cursor::new(file))
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
}
