//! [`ReadSamples`] and [`PollOutput`], what a driver asks of the stack beneath it

use core::ops::Range;

use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;

/// Bytes handed over to the reader at a time
pub(crate) const CUT_LENGTH: usize = 1024 * 1024;

/// The five verbs of a reader a demuxer drives
///
/// Each is the reader's own of the same name, with its contract.
pub(crate) trait ReadSamples {
    /// Takes the next cut of the file and reads the samples it completes
    fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error>;

    /// Takes bytes of the file fetched for what [`wanted_extent`](Self::wanted_extent) named, and reads the samples they complete
    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), isobmff_structure::Error>;

    /// Takes the next sample the file handed over so far completed
    fn poll_sample(&mut self) -> Option<Sample>;

    /// Returns the bytes the extent at the front of those held still lacks, if any is held
    fn wanted_extent(&self) -> Option<Range<u64>>;

    /// Declares the file over
    fn finish(&mut self) -> Result<(), isobmff_structure::Error>;
}

/// The one verb of a writer a muxer takes its bytes by
///
/// It is the writer's own of the same name, with its contract.
pub(crate) trait PollOutput {
    /// Hands over the bytes the file has been laid down as so far
    fn poll_output(&mut self) -> Option<EventBytes>;
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::collections::VecDeque;
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_core::{BoxHeader, BoxType};
    use isobmff_sample::Sample;
    use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

    use super::{PollOutput, ReadSamples};

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
    }

    impl ReadSamples for Scripted {
        fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error> {
            self.inputs.push(input.to_vec());
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

        fn finish(&mut self) -> Result<(), isobmff_structure::Error> {
            self.finished = true;

            self.finish.take().map_or(Ok(()), Err)
        }
    }

    /// Writer handing over what it was scripted to, step by step
    #[derive(Default)]
    pub(crate) struct Queued {
        pub(crate) output: VecDeque<EventBytes>,
    }

    impl PollOutput for Queued {
        fn poll_output(&mut self) -> Option<EventBytes> {
            self.output.pop_front()
        }
    }

    /// A sample of track 1 carrying `data`
    pub(crate) fn sample(data: &[u8]) -> Sample {
        Sample::new(1, 0, 1, 0, 0, 1, data.to_vec())
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
