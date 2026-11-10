//! [`Demuxer`], which reads a file into its boxes and the samples of the tracks a caller subscribes to

use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_core::{BoxDecode, BoxDefinition};

use crate::framing::{self, BoxReader, FramerConfig};
use crate::structure_data::StructureDataReader;
use crate::{RequiredInput, SampleLocation};

mod config;
mod event;

pub use config::DemuxerConfig;
pub use event::{Error, Event, Sample};

/// The event [`Demuxer::poll_event`] lends next
#[derive(Debug, Clone)]
enum Pending {
    Ready(Event<'static>),
    /// A sample whose bytes are in [`Samples::bytes`]
    Sample(SampleLocation),
}

/// Where the demuxer is in the file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No `moov` was read
    NoMovie,
    /// The `moov` was read, and no sample is being gathered
    Movie,
    /// The bytes of the sample at `index` of the locations are being gathered
    Gathering { index: usize },
}

/// What the demuxer holds, in its state, that decides its next step
enum Observation {
    /// A location not yet reported
    Unreported(SampleLocation),
    /// The next sample of a subscribed track wanted, at this index of the
    /// locations
    Wanted(usize),
    /// The next sample wanted, larger than the limit
    OverLimit(usize, SampleLocation),
    /// The next sample wanted, of no bytes
    Empty(usize, SampleLocation),
    /// Every byte of the sample being gathered is in hand
    SampleInHand(SampleLocation),
    /// The input ended inside the sample being gathered
    Truncated(usize, SampleLocation),
    /// The input ended before a byte of the sample wanted was taken
    Abandoned,
    /// A box the box reader reported
    Box(BoxRead),
    /// Nothing without more input
    NeedMoreData,
}

/// What a transition leaves [`Demuxer::step`] to do
enum Step {
    /// Step again
    Continue,
    /// Hold the event for [`Demuxer::poll_event`]
    Output(Pending),
    /// Wait for input
    NeedMoreData,
}

/// How much of the input the state takes next
enum Take {
    /// Up to this many bytes, the rest of the sample being gathered
    Sample(u64),
    /// What the box reader takes
    Boxes,
}

/// A box the box reader reported, read into its value where the demuxer reads one
enum BoxRead {
    /// The `moov`, whose locations now fill [`Samples::locations`]
    Movie(alloc::boxed::Box<MovieBox>),
    Event(Event<'static>),
}

/// The samples of the `moov`, and how far they are reported and their bytes gathered
#[derive(Debug, Clone)]
struct Samples {
    /// The location of each sample, in the order of their file offsets
    locations: Vec<SampleLocation>,
    /// How many of `locations` were reported
    reported: usize,
    /// Index of the first location whose bytes may still be requested
    next: usize,
    /// The bytes of the sample gathered, or being gathered
    bytes: Vec<u8>,
    /// `track_ID` of each track whose samples are lent
    subscribed: Vec<u32>,
}

/// Reader of a file into its boxes at file level and the samples of the tracks a caller subscribes to
///
/// The structure of a file, ISO/IEC 14496-12 §4.2.2, §8.2 and §8.5.
///
/// The demuxer is driven as a [`BoxReader`] is: it names where it wants input
/// with [`required_input`](Self::required_input), takes bytes from any offset
/// up to that one with [`handle_input`](Self::handle_input), and reports one
/// event per call of [`poll_event`](Self::poll_event). It reads the `ftyp` and
/// the `moov` into their values, reports where every sample of the `moov`
/// lies, and lends the bytes of each sample of a track
/// [`subscribe_track`](Self::subscribe_track) names: once it has read the
/// `moov`, it requests the ranges of those samples in the order of their file
/// offsets, those before the `moov` among them. Handing input over never
/// fails: what cannot be read is reported as [`Event::Error`]. Which tracks to
/// subscribe to, and whether to read the file where the demuxer asks, are the
/// caller's choice; the demuxer reads no sample of a track not subscribed to.
/// A caller that cannot seek, and finds the position `required_input` names
/// before the bytes it has read, calls [`finish`](Self::finish): the samples
/// there are located and not lent, and no error is reported for them.
///
/// An event is held until it is polled, and one call of `handle_input` takes
/// no more input than the next event needs: a caller that reads a stream in
/// pieces hands each piece over again until the position `required_input`
/// names lies past its end.
///
/// A movie file whose `moov` comes last, read where the demuxer asks:
///
/// ```
/// use isobmff::demuxer::Event;
/// use isobmff::{Demuxer, DemuxerConfig};
///
/// let file: &[u8] = include_bytes!("../tests/fixtures/movie_moov_last.mp4");
/// let mut demuxer = Demuxer::new(DemuxerConfig::default());
/// let mut samples = 0;
/// loop {
///     while let Some(event) = demuxer.poll_event() {
///         match event {
///             // Subscribe to every track once the `moov` is read
///             Event::Movie(movie) => {
///                 for trak in &movie.trak {
///                     demuxer.subscribe_track(trak.tkhd.track_id);
///                 }
///             }
///             // The samples before the `moov` are lent once it is read
///             Event::Sample(_) => samples += 1,
///             _ => {}
///         }
///     }
///     let Some(required) = demuxer.required_input() else { break };
///     match file.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
///         _ => demuxer.finish(),
///     }
/// }
/// assert_eq!(samples, 12 + 58);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Demuxer {
    boxes: BoxReader,
    structure_data: StructureDataReader,
    sample_size_limit: u64,
    state: State,
    samples: Samples,
    pending: Option<Pending>,
    // Why hold that the input ended rather than ask the box reader: it wants
    // no input either once no box follows that it can find, while samples
    // may still be wanted.
    finished: bool,
}

impl Demuxer {
    /// Creates a demuxer of a file from its first byte, subscribed to no track
    #[must_use]
    pub fn new(config: DemuxerConfig) -> Self {
        let mut framer = FramerConfig::new(config.payload_limit);
        framer.gathered_box_types = vec![FileTypeBox::BOX_TYPE, MovieBox::BOX_TYPE];
        Self {
            boxes: BoxReader::new(framer),
            structure_data: StructureDataReader::new(config.sample_count_limit),
            sample_size_limit: config.sample_size_limit,
            state: State::NoMovie,
            samples: Samples {
                locations: Vec::new(),
                reported: 0,
                next: 0,
                bytes: Vec::new(),
                subscribed: Vec::new(),
            },
            pending: None,
            finished: false,
        }
    }

    /// Lends the bytes of the samples of track `track_id` from now on
    pub fn subscribe_track(&mut self, track_id: u32) {
        if !self.samples.subscribed.contains(&track_id) {
            self.samples.subscribed.push(track_id);
        }
    }

    /// Stops lending the bytes of the samples of track `track_id`, dropping those of one being gathered
    pub fn unsubscribe_track(&mut self, track_id: u32) {
        self.samples
            .subscribed
            .retain(|&subscribed| subscribed != track_id);
        if let State::Gathering { index } = self.state
            && self
                .samples
                .locations
                .get(index)
                .is_some_and(|location| location.properties.track_id == track_id)
        {
            self.state = State::Movie;
        }
    }

    /// Returns where the demuxer wants input next, or `None` once it wants none
    ///
    /// The position is the file offset of the first byte wanted: the next
    /// byte of a sample of a subscribed track, or what the box reader wants.
    #[must_use]
    pub fn required_input(&self) -> Option<RequiredInput> {
        if self.finished {
            return None;
        }
        let boxes = self.boxes.required_input();
        let (location, gathered) = match self.state {
            State::Gathering { index } => (
                self.samples.locations.get(index).copied(),
                length_of(self.samples.bytes.len()),
            ),
            State::Movie => (self.target(boxes).map(|(_, location)| location), 0),
            State::NoMovie => (None, 0),
        };
        match location {
            Some(location) => Some(RequiredInput {
                position: location.offset.saturating_add(gathered),
                length: Some(u64::from(location.size).saturating_sub(gathered)),
            }),
            None => boxes,
        }
    }

    /// Takes bytes of the file that start at the file offset `position`
    ///
    /// Bytes before the position [`required_input`](Self::required_input)
    /// names are ignored. The demuxer takes bytes until it has the next event,
    /// and none while an event waits for [`poll_event`](Self::poll_event); it
    /// copies the box headers, the payloads it reads into values, and the
    /// samples it lends.
    ///
    /// # Panics
    /// Panics if `position` is after the position `required_input` names, or
    /// if called after [`finish`](Self::finish): either is a bug of the caller.
    pub fn handle_input(&mut self, position: u64, bytes: &[u8]) {
        assert!(!self.finished, "input handed over after finish");
        if self.pending.is_none() {
            self.step();
        }
        if self.pending.is_some() {
            return;
        }
        let Some(required) = self.required_input() else {
            return;
        };
        assert!(
            position <= required.position,
            "input starts at offset {position}, after the offset {} the demuxer requires",
            required.position
        );
        match self.take() {
            Take::Sample(remaining) => {
                let ignored = usize::try_from(required.position.saturating_sub(position))
                    .unwrap_or(usize::MAX);
                let bytes = bytes.get(ignored..).unwrap_or_default();
                let taken = bytes
                    .get(..usize::try_from(remaining).unwrap_or(usize::MAX))
                    .unwrap_or(bytes);
                self.samples.bytes.extend_from_slice(taken);
            }
            Take::Boxes => self.boxes.handle_input(position, bytes),
        }
        self.step();
    }

    /// Returns what the bytes handed over hold, one event at a time, or `None`
    /// once the demuxer needs more input
    pub fn poll_event(&mut self) -> Option<Event<'_>> {
        if self.pending.is_none() {
            self.step();
        }
        Some(match self.pending.take()? {
            Pending::Ready(event) => event,
            Pending::Sample(location) => Event::Sample(Sample {
                properties: location.properties,
                offset: location.offset,
                bytes: &self.samples.bytes,
            }),
        })
    }

    /// Ends the input at the bytes handed over so far
    ///
    /// The events that follow report a sample of a subscribed track whose
    /// bytes the input ends inside as [`Error::TruncatedSample`], and what the
    /// box reader reports of the end of the input. No sample is requested
    /// after it, and no error is reported for one none of whose bytes was
    /// handed over: a caller that cannot seek ends the input there.
    pub fn finish(&mut self) {
        self.finished = true;
        self.boxes.finish();
    }

    /// How much of the input the state takes next
    fn take(&self) -> Take {
        match self.state {
            State::Gathering { index } => Take::Sample(
                self.samples
                    .locations
                    .get(index)
                    .map_or(0, |location| u64::from(location.size))
                    .saturating_sub(length_of(self.samples.bytes.len())),
            ),
            State::NoMovie | State::Movie => Take::Boxes,
        }
    }

    /// What the demuxer holds, in its state, and what the box reader reported, that decides its next step
    fn observe(&self, read: Option<BoxRead>) -> Observation {
        // Why the box first: the box reader is asked only once every location
        // is reported, and the `moov` it reports brings the locations that
        // follow it.
        if let Some(read) = read {
            return Observation::Box(read);
        }
        let samples = &self.samples;
        if let Some(&location) = samples.locations.get(samples.reported) {
            return Observation::Unreported(location);
        }
        match self.state {
            State::Gathering { index } => match samples.locations.get(index) {
                Some(&location)
                    if usize::try_from(location.size).ok() == Some(samples.bytes.len()) =>
                {
                    Observation::SampleInHand(location)
                }
                Some(&location) if self.finished && !samples.bytes.is_empty() => {
                    Observation::Truncated(index, location)
                }
                Some(_) if self.finished => Observation::Abandoned,
                Some(_) | None => Observation::NeedMoreData,
            },
            // Why want no sample once the input ended: a caller that cannot
            // seek ends the input where the demuxer asks for bytes before
            // those it has read, and gets no error for the samples there.
            State::Movie if self.finished => Observation::NeedMoreData,
            State::Movie => match self.target(self.boxes.required_input()) {
                Some((index, location)) if u64::from(location.size) > self.sample_size_limit => {
                    Observation::OverLimit(index, location)
                }
                Some((index, location)) if location.size == 0 => {
                    Observation::Empty(index, location)
                }
                Some((index, _)) => Observation::Wanted(index),
                None => Observation::NeedMoreData,
            },
            State::NoMovie => Observation::NeedMoreData,
        }
    }

    /// Steps until an event is held for `poll_event`, or the demuxer waits for input
    fn step(&mut self) {
        loop {
            // Why ask the box reader first: a box it reported comes before the
            // samples it lets the demuxer request, such as an `mdat` before
            // the samples in it.
            let read = match self.state {
                State::NoMovie | State::Movie
                    if self.samples.reported == self.samples.locations.len() =>
                {
                    self.read_box()
                }
                State::NoMovie | State::Movie | State::Gathering { .. } => None,
            };
            let (next_state, step) = match (self.state, self.observe(read)) {
                (state, Observation::Unreported(location)) => {
                    self.samples.reported = self.samples.reported.saturating_add(1);
                    (
                        state,
                        Step::Output(Pending::Ready(Event::SampleLocation(location))),
                    )
                }
                (State::Movie, Observation::Wanted(index)) => {
                    self.samples.bytes.clear();
                    (State::Gathering { index }, Step::Continue)
                }
                (state, Observation::OverLimit(index, location)) => {
                    self.samples.next = index.saturating_add(1);
                    (
                        state,
                        Step::Output(Pending::Ready(Event::Error(Error::SampleOverLimit {
                            location,
                            limit: self.sample_size_limit,
                        }))),
                    )
                }
                (state, Observation::Empty(index, location)) => {
                    self.samples.bytes.clear();
                    self.samples.next = index.saturating_add(1);
                    (state, Step::Output(Pending::Sample(location)))
                }
                (State::Gathering { index }, Observation::SampleInHand(location)) => {
                    self.samples.next = index.saturating_add(1);
                    (State::Movie, Step::Output(Pending::Sample(location)))
                }
                (_, Observation::Abandoned) => (State::Movie, Step::Continue),
                (_, Observation::Truncated(index, location)) => {
                    self.samples.bytes.clear();
                    self.samples.next = index.saturating_add(1);
                    (
                        State::Movie,
                        Step::Output(Pending::Ready(Event::Error(Error::TruncatedSample {
                            location,
                        }))),
                    )
                }
                (_, Observation::Box(BoxRead::Movie(movie))) => (
                    State::Movie,
                    Step::Output(Pending::Ready(Event::Movie(movie))),
                ),
                (state, Observation::Box(BoxRead::Event(event))) => {
                    (state, Step::Output(Pending::Ready(event)))
                }
                (
                    state,
                    Observation::NeedMoreData
                    | Observation::Wanted(..)
                    | Observation::SampleInHand(_),
                ) => (state, Step::NeedMoreData),
            };
            self.state = next_state;
            match step {
                Step::Continue => {}
                Step::Output(pending) => {
                    self.pending = Some(pending);
                    return;
                }
                Step::NeedMoreData => return,
            }
        }
    }

    /// The next event of the box reader, read into the value of its box where the demuxer reads one
    fn read_box(&mut self) -> Option<BoxRead> {
        let event = self.boxes.poll_event()?;
        let read = match event {
            framing::Event::Box { offset, header } => BoxRead::Event(Event::Box { offset, header }),
            framing::Event::Gathered {
                offset,
                box_type,
                payload,
                ..
            } => {
                let decode_error = |source| {
                    BoxRead::Event(Event::Error(Error::Decode {
                        offset,
                        box_type,
                        source,
                    }))
                };
                if box_type == FileTypeBox::BOX_TYPE {
                    FileTypeBox::decode_payload(payload).map_or_else(decode_error, |file_type| {
                        BoxRead::Event(Event::FileType(file_type))
                    })
                } else if self.state != State::NoMovie {
                    BoxRead::Event(Event::Error(Error::DuplicateMovie { offset }))
                } else {
                    match MovieBox::decode_payload(payload) {
                        Err(source) => decode_error(source),
                        Ok(movie) => match self.structure_data.handle_movie(&movie) {
                            Err(error) => BoxRead::Event(Event::Error(Error::StructureData(error))),
                            Ok(locations) => {
                                self.samples.locations.clear();
                                self.samples.locations.extend(locations);
                                BoxRead::Movie(alloc::boxed::Box::new(movie))
                            }
                        },
                    }
                }
            }
            framing::Event::Error(error) => BoxRead::Event(Event::Error(Error::Framing(error))),
        };
        Some(read)
    }

    /// The next sample of a subscribed track whose bytes are wanted before what the box reader wants, with its index
    fn target(&self, boxes: Option<RequiredInput>) -> Option<(usize, SampleLocation)> {
        let samples = &self.samples;
        // Why also before the end of the input when the box reader wants
        // none: an `mdat` of size 0 runs to the end of the file, and its
        // samples are wanted though no box follows it.
        let before = boxes.map_or(u64::MAX, |required| required.position);
        let start = samples.next;
        samples
            .locations
            .get(start..)?
            .iter()
            .enumerate()
            .take_while(|(_, location)| location.offset < before)
            .find(|(_, location)| samples.subscribed.contains(&location.properties.track_id))
            .map(|(index, &location)| (start.saturating_add(index), location))
    }
}

fn length_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
