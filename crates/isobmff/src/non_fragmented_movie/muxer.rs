//! [`NonFragmentedMuxer`], a non-fragmented movie file written to a sink as the samples come

use std::io::Write;

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;
use isobmff_structure::NonFragmentedWriter;

use crate::{DriverError, Muxer, PollOutput};

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
///   [`Structure`](crate::DriverErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::DriverErrorKind::Io), unless the writer refused the call
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
/// use isobmff::{NonFragmentedDemuxer, NonFragmentedMuxer, Sample};
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
/// # Ok::<(), isobmff::DriverError>(())
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
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_file_type`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), DriverError> {
        self.muxer
            .drive(|writer| writer.handle_file_type(file_type))
    }

    /// Takes the movie the file is laid down against, to be written last
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_movie`] makes of the call.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), DriverError> {
        self.muxer.drive(|writer| writer.handle_movie(movie))
    }

    /// Opens a chunk, which the samples handed over next are laid out in, writing the chunk open before it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`NonFragmentedWriter::begin_chunk`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn begin_chunk(&mut self) -> Result<(), DriverError> {
        self.muxer.drive(NonFragmentedWriter::begin_chunk)
    }

    /// Takes a sample, and places it at the end of the chunk that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`NonFragmentedWriter::handle_sample`] makes of the call.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), DriverError> {
        self.muxer.drive(|writer| writer.handle_sample(sample))
    }

    /// Declares the file over, writing the chunk that is open and then the movie, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`NonFragmentedWriter::finish`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub fn finish(&mut self) -> Result<(), DriverError> {
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

    use isobmff_test_support::{file_type, written};

    use super::NonFragmentedMuxer;

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = NonFragmentedMuxer::new(&mut file);

        muxer.handle_file_type(file_type()).unwrap();

        assert_eq!(file, written(&file_type()));
    }
}
