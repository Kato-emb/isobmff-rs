//! [`MediaSegmentMuxer`], a media segment written to a sink, ISO/IEC 14496-12 §8.16

use std::io::Write;

use isobmff_boxes::SegmentTypeBox;
use isobmff_sample::Sample;
use isobmff_structure::MediaSegmentMuxFsm;

use super::driver::Muxer;
use crate::Error;

/// Lays a media segment down on a sink, taking the samples as they come
///
/// The driver of [`MediaSegmentMuxFsm`] over `std::io`: it takes the brands
/// and the samples as the writer does, and writes every byte the writer makes
/// of them to the sink before the call returns. A caller hands over brands
/// and samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`MediaSegmentMuxFsm`]'s contract, carried through as
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
    muxer: Muxer<W, MediaSegmentMuxFsm>,
}

impl<W: Write> MediaSegmentMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a media segment
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, MediaSegmentMuxFsm::new()),
        }
    }

    /// Takes the brands the segment declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::handle_segment_type`] makes of the call.
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
    ///   [`MediaSegmentMuxFsm::begin_fragment`] makes of the call.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment(sequence_number))
    }

    /// Opens a fragment in which every track continues where the samples written for it reach
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::begin_fragment_continuing`] makes of the call.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment_continuing(sequence_number))
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::handle_sample`] makes of the call.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_sample(sample))
    }

    /// Closes the fragment that is open, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::finish_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.muxer.drive(MediaSegmentMuxFsm::finish_fragment)
    }

    /// Declares the segment over, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`MediaSegmentMuxFsm::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink does not flush.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(MediaSegmentMuxFsm::finish)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_test_support::{segment_type, written};

    use super::MediaSegmentMuxer;

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut segment = Vec::new();
        let mut muxer = MediaSegmentMuxer::new(&mut segment);

        muxer.handle_segment_type(segment_type()).unwrap();

        assert_eq!(segment, written(&segment_type()));
    }
}
