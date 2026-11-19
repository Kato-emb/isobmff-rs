//! [`MediaDataReader`], the state machine that assembles samples from the bytes of their media data

use alloc::collections::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::{Ordering, Reverse};

use super::error::Error;
use super::event::{Event, Sample};
use crate::{RequiredInput, SampleLocation};

/// A sample the reader holds, with the order it was handed over in
#[derive(Debug, Clone, Copy)]
struct Held {
    location: SampleLocation,
    order: u64,
}

impl Held {
    fn key(&self) -> (u64, u64) {
        (self.location.offset, self.order)
    }
}

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Held {}

impl PartialOrd for Held {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Held {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No bytes of the first sample held are being gathered
    Waiting,
    /// The bytes of the first sample held are being gathered
    Gathering,
}

/// What the reader holds, in its state, that decides its next step
enum Observation {
    /// No sample is held
    NoSample,
    /// The first sample held has no bytes
    Empty(SampleLocation),
    /// The first sample held is larger than the limit
    OverLimit(SampleLocation),
    /// The first sample held wants its bytes
    Wanted,
    /// Every byte of the sample being gathered is in hand
    InHand(SampleLocation),
    /// The input ended inside the sample being gathered
    Truncated(SampleLocation),
    /// The input ended before a byte of the first sample held was taken
    Abandoned,
    /// Nothing without more input
    NeedMoreData,
}

/// Output of one transition, held until [`MediaDataReader::poll_event`] lends it
#[derive(Debug, Clone, Copy)]
enum Output {
    /// A sample whose bytes are in the reader's buffer
    Sample(SampleLocation),
    Error(Error),
}

/// What one transition leaves [`MediaDataReader::step`] to do
enum Step {
    Continue,
    Output(Output),
    NeedMoreData,
}

/// Reader of the samples at the locations it is given, from the bytes of their media data that arrive in pieces
///
/// The media data of a file, ISO/IEC 14496-12 §8.1.1, or of a file a data
/// reference names, §8.7.2, at the offsets the structure-data gives its
/// samples, §8.7.5.1.
///
/// The reader takes the location of each sample to assemble with
/// [`handle_location`](Self::handle_location), names the file offset it
/// wants next with [`required_input`](Self::required_input), takes bytes from
/// any offset up to that one with [`handle_input`](Self::handle_input), and
/// lends each sample from [`poll_event`](Self::poll_event) once its bytes are
/// in hand. It requests the samples it holds in the order of their file
/// offsets, those at the same offset in the order they were handed over, and
/// holds the bytes of one sample at a time. Handing input over never fails: a
/// sample larger than the limit, or one the input ends inside, is reported as
/// [`Event::Error`]. The reader does no I/O, and does not tell which file the
/// locations lie in: that is the caller's, from the data reference of each
/// sample.
///
/// Two samples, assembled from media data handed over in pieces:
///
/// ```
/// use isobmff::boxes::SampleFlags;
/// use isobmff::media_data::{Event, MediaDataReader};
/// use isobmff::{SampleLocation, SampleProperties};
///
/// // Where the structure-data says the samples lie, out of file order
/// let at = |offset, size| SampleLocation {
///     properties: SampleProperties {
///         track_id: 1,
///         decoding_time: offset,
///         sample_duration: 1,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::default(),
///         sample_description_index: 1,
///     },
///     offset,
///     size,
/// };
/// let mut reader = MediaDataReader::new(16, 1024);
/// reader.handle_location(at(6, 3))?;
/// reader.handle_location(at(1, 4))?;
///
/// // Hand the media data over in pieces of 2 bytes, each again until the
/// // reader wants what lies past it
/// let media_data = b"_abcd_xyz";
/// let mut samples = Vec::new();
/// for (index, piece) in media_data.chunks(2).enumerate() {
///     let position = u64::try_from(index * 2)?;
///     let end = position + u64::try_from(piece.len())?;
///     while let Some(required) = reader.required_input()
///         && required.position < end
///     {
///         reader.handle_input(position, piece);
///         while let Some(event) = reader.poll_event() {
///             if let Event::Sample(sample) = event {
///                 samples.push(sample.bytes.to_vec());
///             }
///         }
///     }
/// }
/// assert_eq!(samples, [b"abcd".to_vec(), b"xyz".to_vec()]);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct MediaDataReader {
    sample_count_limit: u64,
    sample_size_limit: u64,
    held: BinaryHeap<Reverse<Held>>,
    // Why count the samples handed over rather than order the held ones by
    // offset alone: samples at the same offset keep the order they were
    // handed over in, which the heap does not keep.
    handed: u64,
    state: State,
    /// The bytes of the sample being gathered, or of the one last lent
    bytes: Vec<u8>,
    /// The output of the last step, which `poll_event` lends
    pending: Option<Output>,
    finished: bool,
}

impl MediaDataReader {
    /// Creates a reader that holds at most `sample_count_limit` samples, each of at most `sample_size_limit` bytes
    ///
    /// The number and the size of the samples are read from the input, so
    /// the reader holds them against these limits before it allocates room
    /// for them.
    #[must_use]
    pub const fn new(sample_count_limit: u64, sample_size_limit: u64) -> Self {
        Self {
            sample_count_limit,
            sample_size_limit,
            held: BinaryHeap::new(),
            handed: 0,
            state: State::Waiting,
            bytes: Vec::new(),
            pending: None,
            finished: false,
        }
    }

    /// Takes the location of a sample to assemble
    ///
    /// # Errors
    /// * [`Error::SampleCountOverLimit`]: the reader holds as many samples as
    ///   the limit; the sample is not taken, and nothing is allocated for it.
    ///
    /// # Panics
    /// Panics if called after [`finish`](Self::finish): a bug of the caller.
    #[expect(
        clippy::panic_in_result_fn,
        reason = "a location handed over after finish is a bug of the caller, not a sample the reader refuses"
    )]
    pub fn handle_location(&mut self, location: SampleLocation) -> Result<(), Error> {
        assert!(!self.finished, "a location handed over after finish");
        let sample_count = u64::try_from(self.held.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        if sample_count > self.sample_count_limit {
            return Err(Error::SampleCountOverLimit {
                sample_count,
                limit: self.sample_count_limit,
            });
        }
        self.held.push(Reverse(Held {
            location,
            order: self.handed,
        }));
        self.handed = self.handed.wrapping_add(1);
        if self.pending.is_none() {
            self.step();
        }
        Ok(())
    }

    /// Returns whether the reader holds a sample it has not lent or reported
    #[must_use]
    pub fn has_samples(&self) -> bool {
        !self.held.is_empty()
    }

    /// Returns whether an event waits for [`poll_event`](Self::poll_event)
    #[must_use]
    pub const fn has_event(&self) -> bool {
        self.pending.is_some()
    }

    /// Returns where the reader wants input next, or `None` once it wants none
    ///
    /// The position is the file offset of the next byte of the first sample
    /// held, in the order of their file offsets, and the length the bytes of
    /// it still wanted. The reader wants none while it holds no sample, and
    /// after [`finish`](Self::finish).
    #[must_use]
    pub fn required_input(&self) -> Option<RequiredInput> {
        if self.finished {
            return None;
        }
        let Reverse(head) = self.held.peek()?;
        let gathered = match self.state {
            State::Gathering => length_of(self.bytes.len()),
            State::Waiting => 0,
        };
        Some(RequiredInput {
            position: head.location.offset.saturating_add(gathered),
            length: Some(u64::from(head.location.size).saturating_sub(gathered)),
        })
    }

    /// Takes bytes of the media data that start at the file offset `position`
    ///
    /// Bytes before the position [`required_input`](Self::required_input)
    /// names are ignored, so input may start earlier and run on past what is
    /// wanted. The reader copies the bytes of the sample it gathers, and none
    /// past it; it takes none while an event waits for
    /// [`poll_event`](Self::poll_event), and none while it holds no sample,
    /// when it wants no input.
    ///
    /// # Panics
    /// Panics if `position` is after the position `required_input` names, or
    /// if called after [`finish`](Self::finish): either is a bug of the caller.
    pub fn handle_input(&mut self, position: u64, bytes: &[u8]) {
        assert!(!self.finished, "input handed over after finish");
        if self.pending.is_none() {
            self.step();
        }
        if self.pending.is_some() || self.state != State::Gathering {
            return;
        }
        let Some(required) = self.required_input() else {
            return;
        };
        assert!(
            position <= required.position,
            "input starts at offset {position}, after the offset {} the reader requires",
            required.position
        );
        let ignored =
            usize::try_from(required.position.saturating_sub(position)).unwrap_or(usize::MAX);
        let bytes = bytes.get(ignored..).unwrap_or_default();
        let wanted = required
            .length
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(usize::MAX);
        self.bytes
            .extend_from_slice(bytes.get(..wanted).unwrap_or(bytes));
        self.step();
    }

    /// Returns the next sample assembled, or error, one at a time, or `None` once the reader needs more input
    pub fn poll_event(&mut self) -> Option<Event<'_>> {
        if self.pending.is_none() {
            self.step();
        }
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
    ///
    /// The events that follow report a sample the input ends inside as
    /// [`Error::TruncatedSample`]. A sample none of whose bytes was handed
    /// over is dropped and not reported.
    pub fn finish(&mut self) {
        self.finished = true;
        if self.pending.is_none() {
            self.step();
        }
    }

    /// What the reader holds, in its state, that decides its next step
    fn observe(&self) -> Observation {
        let Some(Reverse(head)) = self.held.peek() else {
            return Observation::NoSample;
        };
        let location = head.location;
        let size = u64::from(location.size);
        let gathered = match self.state {
            State::Gathering => length_of(self.bytes.len()),
            State::Waiting => 0,
        };
        match self.state {
            State::Waiting if size > self.sample_size_limit => Observation::OverLimit(location),
            State::Waiting if size == 0 => Observation::Empty(location),
            State::Waiting | State::Gathering if self.finished && gathered == 0 => {
                Observation::Abandoned
            }
            State::Waiting => Observation::Wanted,
            State::Gathering if gathered == size => Observation::InHand(location),
            State::Gathering if self.finished => Observation::Truncated(location),
            State::Gathering => Observation::NeedMoreData,
        }
    }

    /// Steps until an output is held for `poll_event`, or the reader waits for input
    fn step(&mut self) {
        loop {
            let (next_state, step) = match (self.state, self.observe()) {
                (_, Observation::NoSample) => (State::Waiting, Step::NeedMoreData),
                (State::Waiting, Observation::OverLimit(location)) => {
                    self.held.pop();
                    (
                        State::Waiting,
                        Step::Output(Output::Error(Error::SampleOverLimit {
                            location,
                            limit: self.sample_size_limit,
                        })),
                    )
                }
                (State::Waiting, Observation::Empty(location)) => {
                    self.held.pop();
                    self.bytes.clear();
                    (State::Waiting, Step::Output(Output::Sample(location)))
                }
                // Why drop a sample none of whose bytes came without an
                // error: a caller that cannot seek ends the input where the
                // reader wants bytes before those it has read.
                (_, Observation::Abandoned) => {
                    self.held.pop();
                    (State::Waiting, Step::Continue)
                }
                (State::Waiting, Observation::Wanted) => {
                    self.bytes.clear();
                    (State::Gathering, Step::NeedMoreData)
                }
                (State::Gathering, Observation::InHand(location)) => {
                    self.held.pop();
                    (State::Waiting, Step::Output(Output::Sample(location)))
                }
                (State::Gathering, Observation::Truncated(location)) => {
                    self.held.pop();
                    (
                        State::Waiting,
                        Step::Output(Output::Error(Error::TruncatedSample { location })),
                    )
                }
                (
                    State::Gathering,
                    Observation::NeedMoreData
                    | Observation::OverLimit(_)
                    | Observation::Empty(_)
                    | Observation::Wanted,
                )
                | (
                    State::Waiting,
                    Observation::NeedMoreData | Observation::InHand(_) | Observation::Truncated(_),
                ) => (self.state, Step::NeedMoreData),
            };
            self.state = next_state;
            match step {
                Step::Continue => {}
                Step::Output(output) => {
                    self.pending = Some(output);
                    return;
                }
                Step::NeedMoreData => return,
            }
        }
    }
}

fn length_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
