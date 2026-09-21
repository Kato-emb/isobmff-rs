//! [`MediaSegmentMuxer`], a media segment written to a sink as the samples come

use std::io::Write;

use isobmff_boxes::SegmentTypeBox;
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;

use super::MediaSegmentWriter;
use crate::{DriverError, PollOutput, drive};

/// Lays a media segment down on a sink, taking the samples as they come
///
/// The driver of [`MediaSegmentWriter`] over `std::io`: it takes the brands
/// and the samples as the writer does, and writes every byte the writer makes
/// of them to the sink before the call returns. A caller hands over brands
/// and samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`MediaSegmentWriter`]'s contract, carried through as
///   [`Structure`](crate::DriverErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::DriverErrorKind::Io), unless the writer refused the call
///   too, whose failure is the one reported. The sink is written to a box
///   header or a box payload at a time, as the writer hands them over — the
///   media data of a fragment whole; one that is costly to write to in small
///   pieces is the caller's to wrap in a `BufWriter`.
/// * [`finish`](Self::finish) declares the segment over and flushes the sink.
///
/// # Examples
///
/// ```
/// use isobmff::{MediaSegmentMuxer, Sample};
/// # use isobmff_test_support::segment_type;
/// // A segment opening with its brands
/// let mut segment = Vec::new();
/// let mut muxer = MediaSegmentMuxer::new(&mut segment);
/// muxer.handle_segment_type(segment_type())?;
///
/// // One fragment of two samples of track 1, written to the segment as it is closed
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The segment opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&segment[4..8], b"styp");
/// assert!(segment.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff::DriverError>(())
/// ```
#[derive(Debug)]
pub struct MediaSegmentMuxer<W> {
    sink: W,
    writer: MediaSegmentWriter,
}

impl<W: Write> MediaSegmentMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a media segment
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            sink,
            writer: MediaSegmentWriter::new(),
        }
    }

    /// Takes the brands the segment declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`MediaSegmentWriter::handle_segment_type`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn handle_segment_type(&mut self, segment_type: SegmentTypeBox) -> Result<(), DriverError> {
        drive(&mut self.writer, &mut self.sink, |writer| {
            writer.handle_segment_type(segment_type)
        })
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`MediaSegmentWriter::begin_fragment`] makes of the call.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), DriverError> {
        drive(&mut self.writer, &mut self.sink, |writer| {
            writer.begin_fragment(sequence_number)
        })
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`MediaSegmentWriter::handle_sample`] makes of the call.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), DriverError> {
        drive(&mut self.writer, &mut self.sink, |writer| {
            writer.handle_sample(sample)
        })
    }

    /// Closes the fragment that is open, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`MediaSegmentWriter::finish_fragment`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub fn finish_fragment(&mut self) -> Result<(), DriverError> {
        drive(
            &mut self.writer,
            &mut self.sink,
            MediaSegmentWriter::finish_fragment,
        )
    }

    /// Declares the segment over, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::DriverErrorKind::Structure): what
    ///   [`MediaSegmentWriter::finish`] makes of the call.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink does not flush.
    pub fn finish(&mut self) -> Result<(), DriverError> {
        drive(&mut self.writer, &mut self.sink, MediaSegmentWriter::finish)?;
        self.sink.flush()?;

        Ok(())
    }
}

impl PollOutput for MediaSegmentWriter {
    fn poll_output(&mut self) -> Option<EventBytes> {
        MediaSegmentWriter::poll_output(self)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use std::io;

    use isobmff_boxes::MovieFragmentBox;
    use isobmff_core::BoxDefinition;
    use isobmff_test_support::{segment_type, written};

    use super::MediaSegmentMuxer;
    use crate::{DriverErrorKind, StructureError};

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut segment = Vec::new();
        let mut muxer = MediaSegmentMuxer::new(&mut segment);

        muxer.handle_segment_type(segment_type()).unwrap();

        assert_eq!(segment, written(&segment_type()));
    }

    #[test]
    fn a_refusal_of_the_writer_is_reported_and_leaves_what_was_written() {
        let mut segment = Vec::new();
        let mut muxer = MediaSegmentMuxer::new(&mut segment);
        muxer.handle_segment_type(segment_type()).unwrap();

        let refused = muxer.finish();

        assert_eq!(
            refused.map_err(|failure| failure.structure_error()),
            Err(Some(StructureError::missing_mandatory_box(
                MovieFragmentBox::BOX_TYPE
            )))
        );
        assert_eq!(segment, written(&segment_type()));
    }

    #[test]
    fn a_sink_taking_no_byte_is_reported_as_the_sink_failing() {
        let mut muxer = MediaSegmentMuxer::new(&mut [][..]);

        assert_eq!(
            muxer
                .handle_segment_type(segment_type())
                .map_err(|failure| failure.kind()),
            Err(DriverErrorKind::Io(io::ErrorKind::WriteZero))
        );
    }
}
