//! [`SampleReader`], which gathers the bytes of one sample of a subscribed track from the input

use alloc::vec::Vec;

use super::error::Error;
use super::event::{Event, Sample};
use crate::{RequiredInput, SampleLocation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No sample is handed over
    Waiting,
    /// The bytes of the sample handed over are wanted
    Gathering(SampleLocation),
}

/// What the reader holds, in its state, that decides its next step
enum Observation {
    /// No sample is handed over
    NoSample,
    /// The sample handed over is larger than the limit
    OverLimit(SampleLocation),
    /// Every byte of the sample is in hand, as for a sample of no bytes
    InHand(SampleLocation),
    /// The input ended inside the sample
    Truncated(SampleLocation),
    /// The input ended before a byte of the sample was taken
    Abandoned,
    /// Nothing without more input
    NeedMoreData,
}

/// Output of one transition, held until [`SampleReader::poll_event`] lends it
#[derive(Debug, Clone, Copy)]
enum Output {
    /// A sample whose bytes are in the reader's buffer
    Sample(SampleLocation),
    Error(Error),
}

/// Reader of the bytes of the one sample the demuxer hands over at a time, from input cut anywhere
#[derive(Debug, Clone)]
pub(super) struct SampleReader {
    sample_size_limit: u64,
    state: State,
    /// The bytes of the sample being gathered, or of the one last lent
    bytes: Vec<u8>,
    /// The output of the last transition, which `poll_event` lends
    pending: Option<Output>,
    finished: bool,
}

impl SampleReader {
    /// Creates a reader of samples of at most `sample_size_limit` bytes
    pub(super) const fn new(sample_size_limit: u64) -> Self {
        Self {
            sample_size_limit,
            state: State::Waiting,
            bytes: Vec::new(),
            pending: None,
            finished: false,
        }
    }

    /// Takes the location of the sample to gather next
    pub(super) fn handle_location(&mut self, location: SampleLocation) {
        self.state = State::Gathering(location);
        self.bytes.clear();
        self.step();
    }

    /// Drops the sample being gathered if it is one of track `track_id`, keeping an event that waits
    pub(super) fn drop_track(&mut self, track_id: u32) {
        // Why not a new reader in its place: it would drop the event that
        // waits too, and the buffer with its capacity.
        if let State::Gathering(location) = self.state
            && location.properties.track_id == track_id
        {
            self.state = State::Waiting;
        }
    }

    /// Whether a sample is being gathered
    pub(super) const fn has_sample(&self) -> bool {
        matches!(self.state, State::Gathering(_))
    }

    /// Whether an event waits for `poll_event`
    pub(super) const fn has_event(&self) -> bool {
        self.pending.is_some()
    }

    /// The file offset of the next byte of the sample being gathered, and the length of the bytes still wanted
    pub(super) fn required_input(&self) -> Option<RequiredInput> {
        let State::Gathering(location) = self.state else {
            return None;
        };
        let gathered = length_of(self.bytes.len());
        Some(RequiredInput {
            position: location.offset.saturating_add(gathered),
            length: Some(u64::from(location.size).saturating_sub(gathered)),
        })
    }

    /// Takes the bytes of the sample being gathered from input that starts at file offset `position`, at or before the one `required_input` names
    pub(super) fn handle_input(&mut self, position: u64, bytes: &[u8]) {
        let State::Gathering(location) = self.state else {
            return;
        };
        let gathered = self.bytes.len();
        let ignored = usize::try_from(
            location
                .offset
                .saturating_add(length_of(gathered))
                .saturating_sub(position),
        )
        .unwrap_or(usize::MAX);
        let bytes = bytes.get(ignored..).unwrap_or_default();
        let wanted = usize::try_from(location.size)
            .unwrap_or(usize::MAX)
            .saturating_sub(gathered);
        self.bytes
            .extend_from_slice(bytes.get(..wanted).unwrap_or(bytes));
        self.step();
    }

    /// The sample gathered, or the error of one that cannot be, once
    pub(super) fn poll_event(&mut self) -> Option<Event<'_>> {
        Some(match self.pending.take()? {
            Output::Sample(location) => Event::Sample(Sample {
                properties: location.properties,
                offset: location.offset,
                bytes: &self.bytes,
            }),
            Output::Error(error) => Event::Error(error),
        })
    }

    /// Ends the input at the bytes handed over so far
    pub(super) fn finish(&mut self) {
        self.finished = true;
        if self.pending.is_none() {
            self.step();
        }
    }

    /// What the reader holds, in its state, that decides its next step
    fn observe(&self) -> Observation {
        let State::Gathering(location) = self.state else {
            return Observation::NoSample;
        };
        let size = u64::from(location.size);
        let gathered = length_of(self.bytes.len());
        if size > self.sample_size_limit {
            Observation::OverLimit(location)
        } else if gathered == size {
            Observation::InHand(location)
        } else if self.finished && gathered == 0 {
            Observation::Abandoned
        } else if self.finished {
            Observation::Truncated(location)
        } else {
            Observation::NeedMoreData
        }
    }

    /// Moves to the next state, holding the output of the transition for `poll_event`
    fn step(&mut self) {
        let (next_state, output) = match self.observe() {
            Observation::OverLimit(location) => (
                State::Waiting,
                Some(Output::Error(Error::SampleOverLimit {
                    location,
                    limit: self.sample_size_limit,
                })),
            ),
            Observation::InHand(location) => (State::Waiting, Some(Output::Sample(location))),
            Observation::Truncated(location) => (
                State::Waiting,
                Some(Output::Error(Error::TruncatedSample { location })),
            ),
            // Why drop a sample none of whose bytes came without an error: a
            // caller that cannot seek ends the input where the demuxer wants
            // bytes before those it has read.
            Observation::Abandoned => (State::Waiting, None),
            Observation::NoSample | Observation::NeedMoreData => (self.state, None),
        };
        self.state = next_state;
        self.pending = output;
    }
}

fn length_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
