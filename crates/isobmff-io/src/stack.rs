//! [`Demux`], [`ResumeSamples`] and [`Mux`], what a driver asks of the stack beneath it

use core::ops::Range;

use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;

/// Most bytes read off the source at a time
pub(crate) const CUT_LENGTH: usize = 1024 * 1024;

/// The verbs and queries of a demux FSM that a demux driver drives it by
///
/// Each is the demux FSM's own of the same name, with its contract. The
/// trait is sealed: the demux FSMs of [`isobmff_structure`] implement it,
/// and no other type can.
pub trait Demux: sealed::Sealed {
    /// Takes the next bytes of the file in order, and reads the samples they complete
    ///
    /// # Errors
    ///
    /// * What the demux FSM's `handle_input` makes of the bytes.
    fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error>;

    /// Takes bytes of the file read for what [`wanted_extent`](Self::wanted_extent) named, and reads the samples they complete
    ///
    /// # Errors
    ///
    /// * What the demux FSM's `handle_data` makes of the bytes.
    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), isobmff_structure::Error>;

    /// Takes the next sample the file handed over so far completed
    fn poll_sample(&mut self) -> Option<Sample>;

    /// Returns the bytes the extent at the front of those held still lacks, once the input has passed its start
    fn wanted_extent(&self) -> Option<Range<u64>>;

    /// Returns the file offset the next byte handed to [`handle_input`](Self::handle_input) lies at
    fn input_offset(&self) -> u64;

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * What the demux FSM's `finish` makes of the end of the file.
    fn finish(&mut self) -> Result<(), isobmff_structure::Error>;
}

/// The verb of a reader a demuxer restarts at a resume point an index names
///
/// It is the reader's own of the same name, with its contract: the readers of
/// the structures an index points into have it, and the reader of a
/// non-fragmented movie does not.
pub(crate) trait ResumeSamples: Demux {
    /// Restarts the reading at `offset`, the file offset the input handed over next starts at
    fn resume_at(&mut self, offset: u64) -> Result<(), isobmff_structure::Error>;
}

/// The verb of a mux FSM by which a mux driver takes the bytes the FSM made
///
/// It is the mux FSM's own of the same name, with its contract. The trait is
/// sealed: the mux FSMs of [`isobmff_structure`] implement it, and no other
/// type can.
pub trait Mux: sealed::Sealed {
    /// Hands over the bytes the file has been laid down as so far
    fn poll_output(&mut self) -> Option<EventBytes>;
}

mod sealed {
    /// The bound no type outside this crate can meet, which closes [`Demux`](super::Demux) and [`Mux`](super::Mux)
    #[allow(
        unnameable_types,
        reason = "a supertrait no other crate can name is what seals the trait"
    )]
    pub trait Sealed {}

    impl Sealed for isobmff_structure::FragmentedDemuxFsm {}
    impl Sealed for isobmff_structure::MediaSegmentDemuxFsm {}
    impl Sealed for isobmff_structure::NonFragmentedDemuxFsm {}
    impl Sealed for isobmff_structure::FragmentedMuxFsm {}
    impl Sealed for isobmff_structure::MediaSegmentMuxFsm {}
    impl Sealed for isobmff_structure::NonFragmentedMuxFsm {}
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::collections::VecDeque;
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::SampleFlags;
    use isobmff_core::{BoxHeader, BoxType};
    use isobmff_sample::Sample;
    use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

    use super::{Demux, Mux, ResumeSamples, sealed};

    /// Reader answering as scripted, and recording what it was handed
    #[derive(Default)]
    pub(crate) struct Scripted {
        pub(crate) wanted: Option<Range<u64>>,
        pub(crate) completed_by_input: Vec<Sample>,
        pub(crate) finish: Option<isobmff_structure::Error>,
        pub(crate) inputs: Vec<Vec<u8>>,
        pub(crate) data: Vec<(u64, Vec<u8>)>,
        pub(crate) samples: VecDeque<Sample>,
        pub(crate) finished: bool,
        pub(crate) resumed_at: Vec<u64>,
        pub(crate) input_offset: u64,
    }

    impl sealed::Sealed for Scripted {}

    impl Demux for Scripted {
        fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error> {
            self.inputs.push(input.to_vec());
            self.input_offset = self
                .input_offset
                .saturating_add(u64::try_from(input.len()).unwrap());
            self.samples.extend(self.completed_by_input.drain(..));

            Ok(())
        }

        fn handle_data(
            &mut self,
            offset: u64,
            data: &[u8],
        ) -> Result<(), isobmff_structure::Error> {
            self.data.push((offset, data.to_vec()));
            self.wanted = None;

            Ok(())
        }

        fn poll_sample(&mut self) -> Option<Sample> {
            self.samples.pop_front()
        }

        fn wanted_extent(&self) -> Option<Range<u64>> {
            self.wanted.clone()
        }

        fn input_offset(&self) -> u64 {
            self.input_offset
        }

        fn finish(&mut self) -> Result<(), isobmff_structure::Error> {
            if self.finished {
                return Err(isobmff_structure::Error::already_finished());
            }
            self.finished = true;

            self.finish.take().map_or(Ok(()), Err)
        }
    }

    impl ResumeSamples for Scripted {
        fn resume_at(&mut self, offset: u64) -> Result<(), isobmff_structure::Error> {
            self.resumed_at.push(offset);
            self.input_offset = offset;
            self.samples.clear();
            self.wanted = None;
            self.finished = false;

            Ok(())
        }
    }

    /// Writer handing over what it was scripted to, step by step
    #[derive(Default)]
    pub(crate) struct Queued {
        pub(crate) output: VecDeque<EventBytes>,
    }

    impl sealed::Sealed for Queued {}

    impl Mux for Queued {
        fn poll_output(&mut self) -> Option<EventBytes> {
            self.output.pop_front()
        }
    }

    /// A sample of track 1 carrying `data`
    pub(crate) fn sample(data: &[u8]) -> Sample {
        Sample::new(1, 0, 1, 0, SampleFlags::ZERO, 1, data.to_vec())
    }

    /// The bytes a `free` box of `payload` is framed as, one `EventBytes` a step
    pub(crate) fn framed(payload: &[u8]) -> Vec<EventBytes> {
        let mut boxes = BoxWriter::new();
        let header =
            BoxHeader::with_payload_len(BoxType::compact(*b"free"), payload.len() as u64).unwrap();

        boxes.handle_event(BoxEvent::Header(header)).unwrap();
        boxes
            .handle_event(BoxEvent::Payload(payload.to_vec()))
            .unwrap();
        boxes.handle_event(BoxEvent::End).unwrap();

        core::iter::from_fn(|| boxes.poll_output()).collect()
    }
}
