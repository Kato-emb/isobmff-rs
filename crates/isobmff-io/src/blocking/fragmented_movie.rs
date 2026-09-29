//! [`FragmentedMuxer`], a fragmented movie file written to a sink, ISO/IEC 14496-12 Annex A.8

use std::io::Write;

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_sample::Sample;
use isobmff_structure::FragmentedMuxFsm;

use super::driver::Muxer;
use crate::Error;

/// Lays a fragmented movie file down on a sink, taking the samples as they come
///
/// The driver of [`FragmentedMuxFsm`] over `std::io`: it takes the boxes and
/// the samples as the writer does, and writes every byte the writer makes of
/// them to the sink before the call returns. A caller hands over boxes and
/// samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`FragmentedMuxFsm`]'s contract, carried through as
///   [`Structure`](crate::ErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::ErrorKind::Io), unless the writer refused the call
///   too, whose failure is the one reported. The sink is written to a box
///   header or a box payload at a time, as the writer hands them over — the
///   media data of a fragment whole; one that is costly to write to in small
///   pieces is the caller's to wrap in a `BufWriter`.
/// * [`finish`](Self::finish) declares the file over and flushes the sink.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::blocking::FragmentedMuxer;
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file opening with its brands and the movie its fragments continue
/// let mut file = Vec::new();
/// let mut muxer = FragmentedMuxer::new(&mut file);
/// muxer.handle_file_type(file_type())?;
/// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
///
/// // One fragment of two samples of track 1, written to the file as it is closed
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The file opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff_io::Error>(())
/// ```
#[derive(Debug)]
pub struct FragmentedMuxer<W> {
    muxer: Muxer<W, FragmentedMuxFsm>,
}

impl<W: Write> FragmentedMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a fragmented movie file
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, FragmentedMuxFsm::new()),
        }
    }

    /// Takes the brands the file declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::handle_file_type`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_file_type(file_type))
    }

    /// Takes the movie the fragments continue, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::handle_movie`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_movie(movie))
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::begin_fragment`] makes of the call.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment(sequence_number))
    }

    /// Opens a fragment in which every track continues where the samples written for it reach
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::begin_fragment_continuing`] makes of the call.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment_continuing(sequence_number))
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::handle_sample`] makes of the call.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_sample(sample))
    }

    /// Closes the fragment that is open, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::finish_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.muxer.drive(FragmentedMuxFsm::finish_fragment)
    }

    /// Declares the file over, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedMuxFsm::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink does not flush.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(FragmentedMuxFsm::finish)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_test_support::{file_type, written};

    use super::FragmentedMuxer;

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = FragmentedMuxer::new(&mut file);

        muxer.handle_file_type(file_type()).unwrap();

        assert_eq!(file, written(&file_type()));
    }
}
