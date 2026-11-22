//! [`Demuxer`], which reads a file into its boxes and the samples of the tracks a caller subscribes to

use alloc::vec;

use isobmff_boxes::{
    IdentifiedMediaDataBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox,
};
use isobmff_core::{BoxDecode, BoxDefinition};

use crate::framing::{self, BoxReader, FramerConfig, FrontPart};
use crate::structure_data::{self, StructureDataReader};
use crate::{RequiredInput, SampleLocation};

mod config;
mod error;
mod event;
mod gathered;
mod sample_reader;
mod samples;

pub use config::DemuxerConfig;
pub use error::Error;
pub use event::{Event, Sample};
use gathered::Gathered;
use sample_reader::SampleReader;
use samples::Samples;

/// Length of the `imda_identifier` that starts the payload of an `imda`, ISO/IEC 14496-12 §8.1.4.2
const IMDA_IDENTIFIER_LENGTH: u64 = 4;

/// The event [`Demuxer::poll_event`] lends next
#[derive(Debug, Clone)]
enum Pending {
    Ready(Event<'static>),
    /// The event the sample reader holds
    Sample,
}

/// Where the demuxer is in the file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No `moov` was read
    NoMovie,
    /// The `moov` was read
    Movie,
}

/// What the demuxer holds, in its state, that decides its next step
enum Observation {
    /// A location not yet reported
    Unreported(SampleLocation),
    /// The next sample of a subscribed track wanted, at this index of the
    /// locations
    Wanted(usize, SampleLocation),
    /// An event the sample reader holds
    SampleEvent,
    /// The sample reader is gathering a sample
    Gathering,
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

/// A box the box reader reported, read into its value where the demuxer reads one
enum BoxRead {
    /// The `moov`, whose locations now fill the [`Samples`]
    Movie(alloc::boxed::Box<MovieBox>),
    /// A `moof`, whose locations now follow those of the [`Samples`]
    /// still wanted
    MovieFragment(alloc::boxed::Box<MovieFragmentBox>),
    /// A `moov` or a `moof` the structure-data refuses as the specification
    /// reads it, with the error to report
    Refused {
        error: Error,
        refused: Refused,
    },
    /// A box refused before, its error reported, to be read lossily
    Held(Refused),
    Event(Event<'static>),
}

/// A `moov` or a `moof` the structure-data refused, held until its error is reported
#[derive(Debug, Clone)]
enum Refused {
    Movie(alloc::boxed::Box<MovieBox>),
    MovieFragment {
        /// File offset of the `moof`
        offset: u64,
        moof: alloc::boxed::Box<MovieFragmentBox>,
    },
}

/// Reader of a file into its boxes at file level and the samples of the tracks a caller subscribes to
///
/// The structure of a file, ISO/IEC 14496-12 §4.2.2, §8.2, §8.5, §8.8 and
/// §8.14.
///
/// The demuxer is driven as a [`BoxReader`] is: it names where it wants input
/// with [`required_input`](Self::required_input), takes bytes from any offset
/// up to that one with [`handle_input`](Self::handle_input), and reports one
/// event per call of [`poll_event`](Self::poll_event). It reads the `ftyp`,
/// each `styp`, the `moov`, each `moof`, each `sidx` and the `mfra` into their values, reports where
/// every sample of the `moov` and of each `moof` lies, and lends the bytes of
/// each sample of a track [`subscribe_track`](Self::subscribe_track) names:
/// once it has read the `moov` or a `moof`, it requests the ranges of those
/// samples in the order of their file offsets, those before the `moov` among
/// them. A file of movie fragments, or of segments laid one after another, is
/// read as one input, the decoding times of each track going on across them.
/// A sample before a `moof` is not requested once the `moof` is read. A
/// sample that lies in an `imda`, §8.1.4, is located once both the box that
/// declares it and that `imda` are read, in whichever order the file holds
/// them; one whose `imda` the file lacks is reported once the input ends. A
/// caller that cannot seek locates the samples of an `imda` that comes
/// before the box that declares them, and is not lent them. A sample whose
/// data reference names another file, §8.7.2, is located and not lent, nor
/// are bytes requested for it: a caller takes its bytes from that file, at
/// the offset and of the size the location gives.
/// Handing input over never fails: what cannot be read, such as a `moof`
/// before any `moov`, is reported as [`Event::Error`]. What the reading of
/// the specification refuses, the demuxer reads lossily once the
/// error is reported; what that refuses too is left out. Which tracks to
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
/// use isobmff::demux::Event;
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
    sample_count_limit: u64,
    random_access_entry_limit: u64,
    state: State,
    samples: Samples,
    /// Reader of the bytes of the sample of a subscribed track being
    /// gathered, which [`Samples`] picks and hands over one at a time
    sample_reader: SampleReader,
    pending: Option<Pending>,
    // Why hold the box refused rather than read it lossily at once:
    // its error is reported first, and one event is held at a time.
    refused: Option<Refused>,
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
        framer.gathered_box_types = Gathered::BOX_TYPES.to_vec();
        framer.front_part_box_types = vec![FrontPart {
            box_type: IdentifiedMediaDataBox::BOX_TYPE,
            length: IMDA_IDENTIFIER_LENGTH,
        }];
        Self {
            boxes: BoxReader::new(framer),
            structure_data: StructureDataReader::new(
                config.sample_count_limit,
                config.identified_media_data_limit,
            ),
            sample_count_limit: config.sample_count_limit,
            random_access_entry_limit: config.random_access_entry_limit,
            state: State::NoMovie,
            samples: Samples::default(),
            sample_reader: SampleReader::new(config.sample_size_limit),
            pending: None,
            refused: None,
            finished: false,
        }
    }

    /// Lends the bytes of the samples of track `track_id` from now on
    pub fn subscribe_track(&mut self, track_id: u32) {
        self.samples.subscribe(track_id);
    }

    /// Stops lending the bytes of the samples of track `track_id`, dropping those of one being gathered
    pub fn unsubscribe_track(&mut self, track_id: u32) {
        self.samples.unsubscribe(track_id);
        self.sample_reader.drop_track(track_id);
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
        if let Some(required) = self.sample_reader.required_input() {
            return Some(required);
        }
        let boxes = self.boxes.required_input();
        let target = match self.state {
            State::Movie => self
                .samples
                .target(boxes.map(|required| required.position), |location| {
                    Self::is_in_this_file(&self.structure_data, location)
                }),
            State::NoMovie => None,
        };
        match target {
            Some((_, location)) => Some(RequiredInput {
                position: location.offset,
                length: Some(u64::from(location.size)),
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
        if self.sample_reader.has_sample() {
            self.sample_reader.handle_input(position, bytes);
        } else {
            self.boxes.handle_input(position, bytes);
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
            Pending::Sample => self.sample_reader.poll_event()?,
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
        self.sample_reader.finish();
        self.boxes.finish();
    }

    /// What the demuxer holds, in its state, and what the box reader reported, that decides its next step
    fn observe(&self, read: Option<BoxRead>) -> Observation {
        // Why the box first: the box reader is asked only once every location
        // is reported, and the `moov` it reports brings the locations that
        // follow it.
        if let Some(read) = read {
            return Observation::Box(read);
        }
        if let Some(location) = self.samples.unreported() {
            return Observation::Unreported(location);
        }
        if self.sample_reader.has_event() {
            return Observation::SampleEvent;
        }
        if self.sample_reader.has_sample() {
            return Observation::Gathering;
        }
        match self.state {
            // Why want no sample once the input ended: a caller that cannot
            // seek ends the input where the demuxer asks for bytes before
            // those it has read, and gets no error for the samples there.
            State::Movie if self.finished => Observation::NeedMoreData,
            State::Movie => match self.samples.target(
                self.boxes
                    .required_input()
                    .map(|required| required.position),
                |location| Self::is_in_this_file(&self.structure_data, location),
            ) {
                Some((index, location)) => Observation::Wanted(index, location),
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
            let read = if self.samples.is_reported()
                && !self.sample_reader.has_sample()
                && !self.sample_reader.has_event()
            {
                match self.refused.take() {
                    Some(refused) => Some(BoxRead::Held(refused)),
                    None => self.read_box(),
                }
            } else {
                None
            };
            let (next_state, step) = match (self.state, self.observe(read)) {
                (state, Observation::Unreported(location)) => {
                    self.samples.report();
                    (
                        state,
                        Step::Output(Pending::Ready(Event::SampleLocation(location))),
                    )
                }
                (State::Movie, Observation::Wanted(index, location)) => {
                    self.samples.pass(index);
                    self.sample_reader.handle_location(location);
                    (State::Movie, Step::Continue)
                }
                (state, Observation::SampleEvent) => (state, Step::Output(Pending::Sample)),
                (_, Observation::Box(BoxRead::Movie(movie))) => (
                    State::Movie,
                    Step::Output(Pending::Ready(Event::Movie(movie))),
                ),
                (state, Observation::Box(BoxRead::MovieFragment(moof))) => (
                    state,
                    Step::Output(Pending::Ready(Event::MovieFragment(moof))),
                ),
                (state, Observation::Box(BoxRead::Refused { error, refused })) => {
                    self.refused = Some(refused);
                    (state, Step::Output(Pending::Ready(Event::Error(error))))
                }
                // Why read every box refused again rather than those whose
                // error a lossy reading reads past: no table of which errors
                // those are is kept, and reading again stops at the same
                // check, most before any sample is resolved.
                (state, Observation::Box(BoxRead::Held(Refused::Movie(movie)))) => {
                    let resolved = self
                        .structure_data
                        .handle_movie_lossy(&movie)
                        .map(|locations| self.samples.replace(locations));
                    match resolved {
                        Ok(()) => (
                            State::Movie,
                            Step::Output(Pending::Ready(Event::Movie(movie))),
                        ),
                        Err(_) => (state, Step::Continue),
                    }
                }
                (
                    state,
                    Observation::Box(BoxRead::Held(Refused::MovieFragment { offset, moof })),
                ) => {
                    let resolved = self
                        .structure_data
                        .handle_movie_fragment_lossy(&moof, offset)
                        .map(|locations| self.samples.extend(locations));
                    match resolved {
                        Ok(()) => (
                            state,
                            Step::Output(Pending::Ready(Event::MovieFragment(moof))),
                        ),
                        Err(_) => (state, Step::Continue),
                    }
                }
                (state, Observation::Box(BoxRead::Event(event))) => {
                    (state, Step::Output(Pending::Ready(event)))
                }
                (
                    state,
                    Observation::NeedMoreData | Observation::Wanted(..) | Observation::Gathering,
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

    /// Whether the bytes of the sample at `location`, which `structure_data` resolved, lie in this file, where the demuxer requests them
    fn is_in_this_file(structure_data: &StructureDataReader, location: &SampleLocation) -> bool {
        // Why read an error as another file rather than report it: the
        // structure-data checks the sample entry and the data reference entry
        // of every location it returns as it resolves them, and refuses the
        // box where it cannot, so no location it returned errs here.
        structure_data
            .is_in_this_file(
                location.properties.track_id,
                location.properties.sample_description_index,
            )
            .unwrap_or(false)
    }

    /// The next event of the box reader, read into the value of its box where the demuxer reads one
    fn read_box(&mut self) -> Option<BoxRead> {
        let Some(event) = self.boxes.poll_event() else {
            // Why only once the box reader reports nothing more, and the
            // input ends or the box reader wants none: no `imda` follows
            // then, but one may be the last box, and a box of size 0 runs to
            // the end of the file with no `finish` from a caller that reads
            // until no input is wanted.
            return (self.finished || self.boxes.required_input().is_none())
                .then(|| self.structure_data.finish().err())
                .flatten()
                .map(|error| BoxRead::Event(Event::Error(Error::EndOfInput(error))));
        };
        let read = match event {
            framing::Event::Box { offset, header } => BoxRead::Event(Event::Box { offset, header }),
            framing::Event::FrontPart { offset, header, .. }
                if header.box_type() != IdentifiedMediaDataBox::BOX_TYPE =>
            {
                BoxRead::Event(Event::Box { offset, header })
            }
            framing::Event::FrontPart {
                offset,
                header,
                front_part,
            } => {
                let box_type = header.box_type();
                let error = |error| BoxRead::Event(Event::Error(error));
                let refused = |source| {
                    error(Error::StructureData {
                        offset,
                        box_type,
                        source,
                    })
                };
                let imda = match IdentifiedMediaDataBox::decode_payload(front_part) {
                    Ok(imda) => imda,
                    Err(source) => {
                        return Some(error(Error::Decode {
                            offset,
                            box_type,
                            source,
                        }));
                    }
                };
                let resolved = match self.structure_data.handle_identified_media_data(
                    imda.imda_identifier,
                    offset.saturating_add(header.encoded_len()),
                ) {
                    Ok(resolved) => resolved,
                    Err(source) => return Some(refused(source)),
                };
                // Why count before holding them: an `imda` may resolve every
                // sample that waits for it at once.
                let held = self
                    .samples
                    .held()
                    .saturating_add(u64::try_from(resolved.len()).unwrap_or(u64::MAX));
                if held > self.sample_count_limit {
                    return Some(refused(structure_data::Error::SampleCountOverLimit {
                        sample_count: held,
                        limit: self.sample_count_limit,
                    }));
                }
                self.samples.extend(resolved);
                BoxRead::Event(Event::Box { offset, header })
            }
            framing::Event::Gathered {
                offset,
                box_type,
                payload_offset,
                payload,
            } => {
                let refused = |source| Error::StructureData {
                    offset,
                    box_type,
                    source,
                };
                // Why before reading the box: a second `moov` is reported as
                // such, whether it reads or not.
                if box_type == MovieBox::BOX_TYPE && self.state != State::NoMovie {
                    return Some(BoxRead::Event(Event::Error(Error::DuplicateMovie {
                        offset,
                    })));
                }
                // Why not read the box and then check its entries: reading it
                // allocates for each entry its `tfra`s declare.
                if box_type == MovieFragmentRandomAccessBox::BOX_TYPE {
                    let entry_count = MovieFragmentRandomAccessBox::entry_count(payload);
                    if entry_count > self.random_access_entry_limit {
                        return Some(BoxRead::Event(Event::Error(
                            Error::RandomAccessEntryOverLimit {
                                offset,
                                entry_count,
                                limit: self.random_access_entry_limit,
                            },
                        )));
                    }
                }
                match Gathered::decode(box_type, payload) {
                    Err(source) => BoxRead::Event(Event::Error(Error::Decode {
                        offset,
                        box_type,
                        source,
                    })),
                    Ok(Gathered::FileType(file_type)) => BoxRead::Event(Event::FileType(file_type)),
                    Ok(Gathered::MovieFragmentRandomAccess(mfra)) => {
                        BoxRead::Event(Event::MovieFragmentRandomAccess(mfra))
                    }
                    Ok(Gathered::SegmentType(styp)) => BoxRead::Event(Event::SegmentType(styp)),
                    Ok(Gathered::SegmentIndex(sidx)) => BoxRead::Event(Event::SegmentIndex {
                        // Why the end of the payload: the anchor is the first
                        // byte after the box, which a header of any length
                        // ends the payload of.
                        anchor: payload_offset
                            .saturating_add(u64::try_from(payload.len()).unwrap_or(u64::MAX)),
                        sidx,
                    }),
                    Ok(Gathered::MovieFragment(moof)) => {
                        let structure_data = &self.structure_data;
                        self.samples.drop_before(offset, |location| {
                            Self::is_in_this_file(structure_data, location)
                        });
                        // Why count before resolving: the samples of the
                        // fragments before may lie after this `moof` and
                        // stay held, and the decoding times move on only for
                        // a `moof` resolved.
                        let held = self
                            .samples
                            .held()
                            .saturating_add(structure_data::movie_fragment_sample_count(&moof));
                        if held > self.sample_count_limit {
                            BoxRead::Event(Event::Error(refused(
                                structure_data::Error::SampleCountOverLimit {
                                    sample_count: held,
                                    limit: self.sample_count_limit,
                                },
                            )))
                        } else {
                            let resolved = self
                                .structure_data
                                .handle_movie_fragment(&moof, offset)
                                .map(|locations| self.samples.extend(locations));
                            match resolved {
                                Ok(()) => BoxRead::MovieFragment(moof),
                                Err(error) => BoxRead::Refused {
                                    error: refused(error),
                                    refused: Refused::MovieFragment { offset, moof },
                                },
                            }
                        }
                    }
                    Ok(Gathered::Movie(movie)) => match self
                        .structure_data
                        .handle_movie(&movie)
                        .map(|locations| self.samples.replace(locations))
                    {
                        Err(error) => BoxRead::Refused {
                            error: refused(error),
                            refused: Refused::Movie(movie),
                        },
                        Ok(()) => BoxRead::Movie(movie),
                    },
                }
            }
            framing::Event::Error(error) => BoxRead::Event(Event::Error(Error::Framing(error))),
        };
        Some(read)
    }
}
