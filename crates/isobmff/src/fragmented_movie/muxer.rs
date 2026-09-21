//! [`FragmentedMuxer`], a fragmented movie file written to a sink as the samples come

use std::io::Write;

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;

use super::FragmentedWriter;
use crate::{DriverError, StructureError};

/// Lays a fragmented movie file down on a sink, taking the samples as they come
///
/// The driver of [`FragmentedWriter`] over `std::io`: it takes the boxes and
/// the samples as the writer does, and writes every byte the writer makes of
/// them to the sink before the call returns. A caller hands over boxes and
/// samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`FragmentedWriter`]'s contract, carried through as
///   [`Structure`](crate::DriverErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::DriverErrorKind::Io), reported after the writer's own
///   failure if both fail.
/// * [`finish`](Self::finish) declares the file over and flushes the sink.
///
/// # Examples
///
/// ```
/// use isobmff::{FragmentedMuxer, Sample, TrackExtendsBox};
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file opening with its brands and the movie its fragments continue
/// let mut file = Vec::new();
/// let mut muxer = FragmentedMuxer::new(&mut file);
/// muxer.handle_file_type(file_type())?;
/// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0)))?;
///
/// // One fragment of two samples of track 1, written to the file as it is closed
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The file opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff::DriverError>(())
/// ```
#[derive(Debug)]
pub struct FragmentedMuxer<W> {
    sink: W,
    writer: FragmentedWriter,
}

impl<W: Write> FragmentedMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a fragmented movie file
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            sink,
            writer: FragmentedWriter::new(),
        }
    }

    /// Takes the brands the file declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`FragmentedWriter::handle_file_type`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), DriverError> {
        self.drive(|writer| writer.handle_file_type(file_type))
    }

    /// Takes the movie the fragments continue, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`FragmentedWriter::handle_movie`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), DriverError> {
        self.drive(|writer| writer.handle_movie(movie))
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`FragmentedWriter::begin_fragment`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), DriverError> {
        self.drive(|writer| writer.begin_fragment(sequence_number))
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`FragmentedWriter::handle_sample`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), DriverError> {
        self.drive(|writer| writer.handle_sample(sample))
    }

    /// Closes the fragment that is open, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`FragmentedWriter::finish_fragment`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn finish_fragment(&mut self) -> Result<(), DriverError> {
        self.drive(FragmentedWriter::finish_fragment)
    }

    /// Declares the file over, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`FragmentedWriter::finish`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub fn finish(&mut self) -> Result<(), DriverError> {
        self.drive(FragmentedWriter::finish)?;
        self.sink.flush()?;

        Ok(())
    }

    /// Makes `step` of the writer, and writes what the writer made of it whether it failed or not
    fn drive(
        &mut self,
        step: impl FnOnce(&mut FragmentedWriter) -> Result<(), StructureError>,
    ) -> Result<(), DriverError> {
        let stepped = step(&mut self.writer);
        let mut written = Ok(());
        while let Some(bytes) = self.writer.poll_output() {
            written = written.and_then(|()| self.sink.write_all(&bytes));
        }
        stepped?;
        written?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use std::io::{self, Write};

    use isobmff_boxes::{FileTypeBox, TrackExtendsBox};
    use isobmff_core::BoxDefinition;
    use isobmff_test_support::{file_type, fragmented_movie, written};

    use super::FragmentedMuxer;
    use crate::{DriverErrorKind, StructureError, StructureErrorKind};

    /// Sink refusing every byte
    struct Refusing;

    impl Write for Refusing {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = FragmentedMuxer::new(&mut file);

        muxer.handle_file_type(file_type()).unwrap();

        assert_eq!(file, written(&file_type()));
    }

    #[test]
    fn the_bytes_made_before_a_refusal_are_written_and_the_refusal_reported() {
        let mut file = Vec::new();
        let mut muxer = FragmentedMuxer::new(&mut file);
        let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0));
        muxer.handle_movie(movie.clone()).unwrap();

        let refused = muxer.handle_file_type(file_type());

        assert_eq!(
            refused.map_err(|failure| failure.structure_error()),
            Err(Some(StructureError::box_out_of_order(
                FileTypeBox::BOX_TYPE
            )))
        );
        assert_eq!(file, written(&movie));
    }

    #[test]
    fn a_sink_refusing_the_bytes_is_reported_as_such() {
        let mut muxer = FragmentedMuxer::new(Refusing);

        assert_eq!(
            muxer
                .handle_file_type(file_type())
                .map_err(|failure| failure.kind()),
            Err(DriverErrorKind::Io(io::ErrorKind::BrokenPipe))
        );
    }

    #[test]
    fn the_writers_own_failure_is_reported_ahead_of_the_sinks() {
        let mut muxer = FragmentedMuxer::new(Refusing);

        assert_eq!(
            muxer.finish().map_err(|failure| failure.kind()),
            Err(DriverErrorKind::Structure(
                StructureErrorKind::MissingMandatoryBox
            ))
        );
    }
}
