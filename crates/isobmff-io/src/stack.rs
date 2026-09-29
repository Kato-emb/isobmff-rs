//! [`Demux`] and [`Mux`], what a driver asks of the stack beneath it, what a demux driver reads for a demux FSM — [`Request`], and [`ClosingMovieFragmentRandomAccessOffset`] where it looks for an `mfra` — and [`InFlight`], what a mux driver is writing

use core::ops::Range;
use std::io;

use isobmff_boxes::MovieFragmentRandomAccessOffsetBox;
use isobmff_core::BoxDecode;
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;
use isobmff_structure::{
    FragmentedDemuxFsm, FragmentedMuxFsm, MediaSegmentDemuxFsm, MediaSegmentMuxFsm,
    NonFragmentedDemuxFsm, NonFragmentedMuxFsm,
};

use crate::Error;

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

/// The verb of a mux FSM by which a mux driver takes the bytes the FSM made
///
/// It is the mux FSM's own of the same name, with its contract. The trait is
/// sealed: the mux FSMs of [`isobmff_structure`] implement it, and no other
/// type can.
pub trait Mux: sealed::Sealed {
    /// Hands over the bytes the file has been laid down as so far
    fn poll_output(&mut self) -> Option<EventBytes>;
}

/// What a demux driver reads next for a demux FSM: the bytes it wants, or else the file on in order
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    /// The file offset the read starts at
    pub(crate) offset: u64,
    /// The most bytes the read takes
    pub(crate) length: usize,
    wanted: bool,
}

impl Request {
    /// Returns the read `fsm` asks for next: the extent it wants, a buffer at most, or else a buffer on from its input offset
    pub(crate) fn of<D: Demux>(fsm: &D) -> Self {
        fsm.wanted_extent().map_or(
            Self {
                offset: fsm.input_offset(),
                length: CUT_LENGTH,
                wanted: false,
            },
            |wanted| Self {
                offset: wanted.start,
                length: usize::try_from(wanted.end.saturating_sub(wanted.start))
                    .map_or(CUT_LENGTH, |length| length.min(CUT_LENGTH)),
                wanted: true,
            },
        )
    }

    /// Hands `fsm` the bytes read for the request, and returns `false` once the file was declared over before
    ///
    /// No byte read in order declares the file over.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io) with
    ///   [`UnexpectedEof`](io::ErrorKind::UnexpectedEof): no byte was read
    ///   for a want, the source ending before bytes it held.
    /// * [`Structure`](crate::ErrorKind::Structure): what `fsm` makes of the
    ///   bytes, or of the end of the file.
    pub(crate) fn hand_over<D: Demux>(self, fsm: &mut D, read: &[u8]) -> Result<bool, Error> {
        let handed = match (self.wanted, read.is_empty()) {
            (true, true) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            (true, false) => fsm.handle_data(self.offset, read),
            (false, false) => fsm.handle_input(read),
            (false, true) => match fsm.finish() {
                Err(failure) if failure.kind() == isobmff_structure::ErrorKind::AlreadyFinished => {
                    return Ok(false);
                }
                finished => finished,
            },
        };

        handed?;

        Ok(true)
    }
}

impl Demux for FragmentedDemuxFsm {
    fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error> {
        FragmentedDemuxFsm::handle_input(self, input)
    }

    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), isobmff_structure::Error> {
        FragmentedDemuxFsm::handle_data(self, offset, data)
    }

    fn poll_sample(&mut self) -> Option<Sample> {
        FragmentedDemuxFsm::poll_sample(self)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        FragmentedDemuxFsm::wanted_extent(self)
    }

    fn input_offset(&self) -> u64 {
        FragmentedDemuxFsm::input_offset(self)
    }

    fn finish(&mut self) -> Result<(), isobmff_structure::Error> {
        FragmentedDemuxFsm::finish(self)
    }
}

impl Demux for MediaSegmentDemuxFsm {
    fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error> {
        MediaSegmentDemuxFsm::handle_input(self, input)
    }

    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), isobmff_structure::Error> {
        MediaSegmentDemuxFsm::handle_data(self, offset, data)
    }

    fn poll_sample(&mut self) -> Option<Sample> {
        MediaSegmentDemuxFsm::poll_sample(self)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        MediaSegmentDemuxFsm::wanted_extent(self)
    }

    fn input_offset(&self) -> u64 {
        MediaSegmentDemuxFsm::input_offset(self)
    }

    fn finish(&mut self) -> Result<(), isobmff_structure::Error> {
        MediaSegmentDemuxFsm::finish(self)
    }
}

impl Demux for NonFragmentedDemuxFsm {
    fn handle_input(&mut self, input: &[u8]) -> Result<(), isobmff_structure::Error> {
        NonFragmentedDemuxFsm::handle_input(self, input)
    }

    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), isobmff_structure::Error> {
        NonFragmentedDemuxFsm::handle_data(self, offset, data)
    }

    fn poll_sample(&mut self) -> Option<Sample> {
        NonFragmentedDemuxFsm::poll_sample(self)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        NonFragmentedDemuxFsm::wanted_extent(self)
    }

    fn input_offset(&self) -> u64 {
        NonFragmentedDemuxFsm::input_offset(self)
    }

    fn finish(&mut self) -> Result<(), isobmff_structure::Error> {
        NonFragmentedDemuxFsm::finish(self)
    }
}

impl Mux for FragmentedMuxFsm {
    fn poll_output(&mut self) -> Option<EventBytes> {
        FragmentedMuxFsm::poll_output(self)
    }
}

impl Mux for MediaSegmentMuxFsm {
    fn poll_output(&mut self) -> Option<EventBytes> {
        MediaSegmentMuxFsm::poll_output(self)
    }
}

impl Mux for NonFragmentedMuxFsm {
    fn poll_output(&mut self) -> Option<EventBytes> {
        NonFragmentedMuxFsm::poll_output(self)
    }
}

/// Where the `mfro` closing a file lies when it closes with an `mfra`, ISO/IEC 14496-12 §8.8.11
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClosingMovieFragmentRandomAccessOffset {
    file_len: u64,
    start: u64,
}

impl ClosingMovieFragmentRandomAccessOffset {
    /// The bytes an `mfro` occupies
    pub(crate) const LEN: usize = 16;

    /// Returns where the `mfro` closing a file `file_len` long lies, `None` when the file is too short for one
    pub(crate) fn of(file_len: u64) -> Option<Self> {
        file_len
            .checked_sub(Self::LEN as u64)
            .map(|start| Self { file_len, start })
    }

    /// Returns the position of the `mfro` in a resource the file lies at `origin` of
    pub(crate) const fn position(self, origin: u64) -> u64 {
        // Why not checked_add: the file was measured from `origin` to where
        // the resource ends, so the sum lies within what 64 bits carry.
        origin.saturating_add(self.start)
    }

    /// Returns the offset of the `mfra` the bytes read at the `mfro` step back to, if they are an `mfro` stepping back within the file
    pub(crate) fn movie_fragment_random_access_start(self, mfro: &[u8; Self::LEN]) -> Option<u64> {
        MovieFragmentRandomAccessOffsetBox::decode(mfro)
            .ok()
            .and_then(|(mfro, _)| mfro.movie_fragment_random_access_start(self.file_len))
    }
}

/// The chunk a mux driver is writing and how many of its bytes the sink took, if any
#[derive(Debug)]
pub(crate) struct InFlight(Option<(EventBytes, usize)>);

impl InFlight {
    /// Creates one writing no chunk
    pub(crate) const fn new() -> Self {
        Self(None)
    }

    /// Returns the bytes of the chunk the sink is to take next, taking the next chunk `fsm` made once one is written whole
    pub(crate) fn rest<W: Mux>(&mut self, fsm: &mut W) -> Option<&[u8]> {
        while self
            .0
            .as_ref()
            .is_none_or(|(chunk, taken)| *taken >= chunk.len())
        {
            self.0 = Some((fsm.poll_output()?, 0));
        }

        self.0
            .as_ref()
            .and_then(|(chunk, taken)| chunk.get(*taken..))
    }

    /// Counts what the sink made of the bytes [`rest`](Self::rest) returned
    ///
    /// An interrupted write takes none of them, which the next [`rest`](Self::rest) returns again.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the sink refused the bytes, or took
    ///   none of them ([`WriteZero`](io::ErrorKind::WriteZero)).
    pub(crate) fn took(&mut self, written: io::Result<usize>) -> Result<(), Error> {
        match written {
            Err(failure) if failure.kind() == io::ErrorKind::Interrupted => Ok(()),
            Ok(0) => Err(io::Error::from(io::ErrorKind::WriteZero).into()),
            Ok(written) => {
                if let Some((_, taken)) = &mut self.0 {
                    *taken = taken.saturating_add(written);
                }

                Ok(())
            }
            Err(failure) => Err(failure.into()),
        }
    }
}

mod sealed {
    /// The bound no type outside this crate can meet, which closes [`Demux`](super::Demux) and [`Mux`](super::Mux)
    #[allow(
        unnameable_types,
        reason = "a supertrait no other crate can name is what seals the trait"
    )]
    pub trait Sealed {}

    use isobmff_structure::{
        FragmentedDemuxFsm, FragmentedMuxFsm, MediaSegmentDemuxFsm, MediaSegmentMuxFsm,
        NonFragmentedDemuxFsm, NonFragmentedMuxFsm,
    };

    impl Sealed for FragmentedDemuxFsm {}
    impl Sealed for MediaSegmentDemuxFsm {}
    impl Sealed for NonFragmentedDemuxFsm {}
    impl Sealed for FragmentedMuxFsm {}
    impl Sealed for MediaSegmentMuxFsm {}
    impl Sealed for NonFragmentedMuxFsm {}
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

    use super::{Demux, Mux, sealed};

    /// Demux FSM answering as scripted, and recording what it was handed
    #[derive(Default)]
    pub(crate) struct Scripted {
        pub(crate) wanted: Option<Range<u64>>,
        pub(crate) completed_by_input: Vec<Sample>,
        pub(crate) finish: Option<isobmff_structure::Error>,
        pub(crate) inputs: Vec<Vec<u8>>,
        pub(crate) data: Vec<(u64, Vec<u8>)>,
        pub(crate) samples: VecDeque<Sample>,
        pub(crate) finished: bool,
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

    impl Scripted {
        /// Restarts the reading at `offset`, as a demux FSM's `resume_at` does
        pub(crate) fn resume_at(&mut self, offset: u64) -> Result<(), isobmff_structure::Error> {
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
