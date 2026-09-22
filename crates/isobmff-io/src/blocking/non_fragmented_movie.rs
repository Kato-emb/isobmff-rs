//! [`NonFragmentedDemuxer`] and [`NonFragmentedMuxer`], a non-fragmented movie file read off a source that seeks and written to a sink, ISO/IEC 14496-12 §8.2.1 and §8.7

use core::ops::Range;
use std::io::{Read, Seek, Write};

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;
use isobmff_structure::{NonFragmentedReader, NonFragmentedWriter, StructureError};

use super::driver::{Demuxer, Muxer, PollOutput, ReadSamples};
use crate::Error;

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
///   [`Structure`](crate::ErrorKind::Structure); one ending before
///   where it had already been read to is [`Io`](crate::ErrorKind::Io)
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
/// use isobmff_io::blocking::NonFragmentedDemuxer;
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
/// # Ok::<(), isobmff_io::Error>(())
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
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn new(source: S) -> Result<Self, Error> {
        Self::with_reader(source, NonFragmentedReader::new())
    }

    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn with_reader(source: S, reader: NonFragmentedReader) -> Result<Self, Error> {
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
    type Item = Result<Sample, Error>;

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

/// Lays a non-fragmented movie file down on a sink, taking the samples as they come
///
/// The driver of [`NonFragmentedWriter`] over `std::io`: it takes the boxes
/// and the samples as the writer does, and writes every byte the writer makes
/// of them to the sink before the call returns. A caller hands over boxes and
/// samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`NonFragmentedWriter`]'s contract, carried through as
///   [`Structure`](crate::ErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::ErrorKind::Io), unless the writer refused the call
///   too, whose failure is the one reported. The sink is written to a box
///   header, a box payload, or one sample of a chunk at a time, as the
///   writer hands them over; one that is costly to write to in small pieces
///   is the caller's to wrap in a `BufWriter`.
/// * [`finish`](Self::finish) declares the file over, writes the movie the
///   writer lays down last, and flushes the sink.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff_io::blocking::{NonFragmentedDemuxer, NonFragmentedMuxer};
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{file_type, unfragmented_movie};
/// // A file opening with its brands, whose movie declares one track and no sample yet
/// let mut file = Vec::new();
/// let mut muxer = NonFragmentedMuxer::new(&mut file);
/// muxer.handle_file_type(file_type())?;
/// muxer.handle_movie(unfragmented_movie())?;
///
/// // Two chunks of track 1, written to the file as they come
/// muxer.begin_chunk()?;
/// muxer.handle_sample(Sample::new(1, 0, 3_000, 0, 0, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 3_000, 3_000, 0, 0, 1, b"DATA".to_vec()))?;
/// muxer.begin_chunk()?;
/// muxer.handle_sample(Sample::new(1, 6_000, 3_000, 0, 0, 1, b"LAST".to_vec()))?;
/// muxer.finish()?;
///
/// // Read back, the samples come out as they were laid down
/// let read_back: Vec<Vec<u8>> = NonFragmentedDemuxer::new(Cursor::new(file))?
///     .map(|sample| sample.map(Sample::into_data))
///     .collect::<Result<_, _>>()?;
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec(), b"LAST".to_vec()]);
/// # Ok::<(), isobmff_io::Error>(())
/// ```
#[derive(Debug)]
pub struct NonFragmentedMuxer<W> {
    muxer: Muxer<W, NonFragmentedWriter>,
}

impl<W: Write> NonFragmentedMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a non-fragmented movie file
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, NonFragmentedWriter::new()),
        }
    }

    /// Takes the brands the file declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_file_type`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_file_type(file_type))
    }

    /// Takes the movie the file is laid down against, to be written last
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_movie`] makes of the call.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_movie(movie))
    }

    /// Opens a chunk, which the samples handed over next are laid out in, writing the chunk open before it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::begin_chunk`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn begin_chunk(&mut self) -> Result<(), Error> {
        self.muxer.drive(NonFragmentedWriter::begin_chunk)
    }

    /// Takes a sample, and places it at the end of the chunk that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_sample`] makes of the call.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_sample(sample))
    }

    /// Declares the file over, writing the chunk that is open and then the movie, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`NonFragmentedWriter::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(NonFragmentedWriter::finish)
    }
}

impl PollOutput for NonFragmentedWriter {
    fn poll_output(&mut self) -> Option<EventBytes> {
        NonFragmentedWriter::poll_output(self)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use std::io;

    use isobmff_sample::Sample;
    use isobmff_test_support::{SAMPLE_DURATION, file_type, non_fragmented_file, written};

    use super::{NonFragmentedDemuxer, NonFragmentedMuxer};

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

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = NonFragmentedMuxer::new(&mut file);

        muxer.handle_file_type(file_type()).unwrap();

        assert_eq!(file, written(&file_type()));
    }
}
