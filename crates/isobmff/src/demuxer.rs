//! [`Demuxer`], which reads a file into its boxes and the samples of the tracks a caller subscribes to

use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox};
use isobmff_core::{BoxDecode, BoxDefinition};

use crate::framing::{self, BoxReader, FramerConfig};
use crate::structure_data::StructureDataReader;
use crate::{RequiredInput, SampleLocation};

mod event;

pub use event::{Error, Event, Sample};

/// The limits a [`Demuxer`] holds what it reads to
///
/// Each bounds an allocation whose size the input declares. Build one from the
/// default and set the fields that differ:
///
/// ```
/// use isobmff::{Demuxer, DemuxerConfig};
///
/// let mut config = DemuxerConfig::default();
/// config.sample_size_limit = 64 * 1024 * 1024;
/// let demuxer = Demuxer::new(config);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DemuxerConfig {
    /// The most bytes the payload of a box read into a value, the `ftyp` or
    /// the `moov`, may hold (16 MiB by default)
    ///
    /// A `moov` grows with the samples it declares, by some bytes a sample;
    /// 16 MiB holds that of a movie of hours.
    pub payload_limit: u64,
    /// The most samples the demuxer resolves from one `moov` (2^20 by
    /// default)
    ///
    /// The demuxer holds the location of each, some tens of bytes; 2^20 of
    /// them take some tens of MiB.
    pub sample_count_limit: u64,
    /// The most bytes a sample of a subscribed track may hold (16 MiB by
    /// default)
    ///
    /// A sample is a frame of video or audio; even a frame of 4K video coded
    /// as a sync sample takes a few MiB.
    pub sample_size_limit: u64,
}

impl Default for DemuxerConfig {
    // Why not derive Default: it would set every limit to 0, under which no
    // `moov` and no sample could be read.
    fn default() -> Self {
        Self {
            payload_limit: 16 * 1024 * 1024,
            sample_count_limit: 1 << 20,
            sample_size_limit: 16 * 1024 * 1024,
        }
    }
}

/// The event [`Demuxer::poll_event`] lends next
#[derive(Debug, Clone)]
enum Pending {
    Ready(Event<'static>),
    /// A sample whose bytes are in [`Samples::bytes`]
    Sample(SampleLocation),
}

/// What the demuxer holds that makes an event without more input
enum Observation {
    /// A location not yet reported
    Unreported(SampleLocation),
    /// The next sample wanted, larger than the limit
    OverLimit(usize, SampleLocation),
    /// The next sample wanted, of no bytes
    Empty(usize, SampleLocation),
    /// The sample being gathered, which the input ended inside
    Truncated(usize, SampleLocation),
    /// The input ended, and what the box reader reports of it follows
    Ended,
    /// Nothing without more input
    Waiting,
}

/// The samples of the `moov`, and how far they are reported and their bytes gathered
#[derive(Debug, Clone)]
struct Samples {
    /// The location of each sample, in the order of their file offsets
    locations: Vec<SampleLocation>,
    /// How many of `locations` were reported
    reported: usize,
    /// Index of the first location whose bytes may still be requested
    // Why not derive it from `position`: a sample of no bytes, or one over
    // the limit, is passed without a byte of it taken.
    next: usize,
    /// File offset before which no byte is requested again
    position: u64,
    /// Index of the location whose bytes are being gathered
    gathering: Option<usize>,
    /// The bytes of that sample gathered so far
    bytes: Vec<u8>,
    /// `track_ID` of each track whose samples are lent
    subscribed: Vec<u32>,
}

impl Samples {
    /// Moves `next` past the locations before `position`, whose bytes are never requested again
    fn pass(&mut self) {
        // Why not also past the samples of tracks not subscribed to: a track
        // subscribed to later wants those of them after `position`.
        while self.gathering.is_none()
            && self
                .locations
                .get(self.next)
                .is_some_and(|location| location.offset < self.position)
        {
            self.next = self.next.saturating_add(1);
        }
    }
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
/// [`subscribe_track`](Self::subscribe_track) names that lies after the
/// `moov`: it requests the ranges of those samples in the order of their file
/// offsets. Handing input over never fails: what cannot be read is reported as
/// [`Event::Error`]. Which tracks to subscribe to, and whether to read the
/// file where the demuxer asks or hand it over in order, are the caller's
/// choice; the demuxer neither seeks back to a sample before the `moov` nor
/// reads one of a track not subscribed to.
///
/// An event is held until it is polled, and one call of `handle_input` takes
/// no more input than the next event needs: a caller that reads a stream in
/// pieces hands each piece over again until the position `required_input`
/// names lies past its end.
#[derive(Debug, Clone)]
pub struct Demuxer {
    boxes: BoxReader,
    structure_data: StructureDataReader,
    sample_size_limit: u64,
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
            samples: Samples {
                locations: Vec::new(),
                reported: 0,
                next: 0,
                position: 0,
                gathering: None,
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
        if let Some(index) = self.samples.gathering
            && self
                .samples
                .locations
                .get(index)
                .is_some_and(|location| location.properties.track_id == track_id)
        {
            self.samples.gathering = None;
            self.samples.bytes.clear();
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
        match self.target(boxes) {
            Some((index, location)) => {
                let gathered = if self.samples.gathering == Some(index) {
                    length_of(self.samples.bytes.len())
                } else {
                    0
                };
                Some(RequiredInput {
                    position: location.offset.saturating_add(gathered),
                    length: Some(u64::from(location.size).saturating_sub(gathered)),
                })
            }
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
        self.samples.pass();
        let Some(required) = self.required_input() else {
            return;
        };
        assert!(
            position <= required.position,
            "input starts at offset {position}, after the offset {} the demuxer requires",
            required.position
        );
        match self.target(self.boxes.required_input()) {
            Some((index, location)) => {
                let ignored = usize::try_from(required.position.saturating_sub(position))
                    .unwrap_or(usize::MAX);
                let bytes = bytes.get(ignored..).unwrap_or_default();
                let wanted = usize::try_from(required.length.unwrap_or(0)).unwrap_or(usize::MAX);
                let taken = bytes.get(..wanted).unwrap_or(bytes);
                if self.samples.gathering.is_none() {
                    self.samples.bytes.clear();
                }
                self.samples.gathering = Some(index);
                self.samples.bytes.extend_from_slice(taken);
                self.samples.position = required.position.saturating_add(length_of(taken.len()));
                if self.samples.bytes.len() == usize::try_from(location.size).unwrap_or(usize::MAX)
                {
                    self.samples.gathering = None;
                    self.samples.next = index.saturating_add(1);
                    self.pending = Some(Pending::Sample(location));
                }
            }
            None => {
                self.boxes.handle_input(position, bytes);
                self.pending = self.boxes_event();
            }
        }
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
    /// The events that follow report a sample of a subscribed track the input
    /// ends inside as [`Error::TruncatedSample`], and what the box reader
    /// reports of the end of the input.
    pub fn finish(&mut self) {
        self.finished = true;
        self.boxes.finish();
    }

    /// What the demuxer holds that makes an event without more input
    fn observe(&self) -> Observation {
        let samples = &self.samples;
        if let Some(&location) = samples.locations.get(samples.reported) {
            return Observation::Unreported(location);
        }
        match (self.target(self.boxes.required_input()), self.finished) {
            (Some((index, location)), _) if u64::from(location.size) > self.sample_size_limit => {
                Observation::OverLimit(index, location)
            }
            (Some((index, location)), _) if location.size == 0 => {
                Observation::Empty(index, location)
            }
            (Some((index, location)), true) if samples.gathering == Some(index) => {
                Observation::Truncated(index, location)
            }
            (_, true) => Observation::Ended,
            (_, false) => Observation::Waiting,
        }
    }

    /// Makes the next event that needs no more input pending, if there is one
    fn step(&mut self) {
        self.samples.pass();
        self.pending = match self.observe() {
            Observation::Unreported(location) => {
                self.samples.reported = self.samples.reported.saturating_add(1);
                Some(Pending::Ready(Event::SampleLocation(location)))
            }
            Observation::OverLimit(index, location) => {
                self.samples.next = index.saturating_add(1);
                Some(Pending::Ready(Event::Error(Error::SampleOverLimit {
                    location,
                    limit: self.sample_size_limit,
                })))
            }
            Observation::Empty(index, location) => {
                self.samples.bytes.clear();
                self.samples.next = index.saturating_add(1);
                Some(Pending::Sample(location))
            }
            Observation::Truncated(index, location) => {
                self.samples.gathering = None;
                self.samples.bytes.clear();
                self.samples.next = index.saturating_add(1);
                Some(Pending::Ready(Event::Error(Error::TruncatedSample {
                    location,
                })))
            }
            Observation::Ended => self.boxes_event(),
            Observation::Waiting => None,
        };
    }

    /// The next event of the box reader, read into the value of its box where the demuxer reads one
    fn boxes_event(&mut self) -> Option<Pending> {
        let event = self.boxes.poll_event()?;
        let ready = match event {
            framing::Event::Box { offset, header } => {
                self.samples.position = self
                    .samples
                    .position
                    .max(offset.saturating_add(header.encoded_len()));
                Event::Box { offset, header }
            }
            framing::Event::Gathered {
                offset,
                box_type,
                payload_offset,
                payload,
            } => {
                self.samples.position = self
                    .samples
                    .position
                    .max(payload_offset.saturating_add(length_of(payload.len())));
                let decode_error = |source| {
                    Event::Error(Error::Decode {
                        offset,
                        box_type,
                        source,
                    })
                };
                if box_type == FileTypeBox::BOX_TYPE {
                    FileTypeBox::decode_payload(payload).map_or_else(decode_error, Event::FileType)
                } else {
                    match MovieBox::decode_payload(payload) {
                        Err(source) => decode_error(source),
                        Ok(movie) => match self.structure_data.handle_movie(&movie) {
                            Err(error) => Event::Error(Error::StructureData(error)),
                            Ok(locations) => {
                                let samples = &mut self.samples;
                                samples.locations.clear();
                                samples.locations.extend(locations);
                                samples.reported = 0;
                                samples.next = 0;
                                samples.gathering = None;
                                samples.bytes.clear();
                                Event::Movie(alloc::boxed::Box::new(movie))
                            }
                        },
                    }
                }
            }
            framing::Event::Error(error) => Event::Error(Error::Framing(error)),
        };
        Some(Pending::Ready(ready))
    }

    /// The next sample of a subscribed track whose bytes are wanted before what the box reader wants, with its index
    fn target(&self, boxes: Option<RequiredInput>) -> Option<(usize, SampleLocation)> {
        let samples = &self.samples;
        if let Some(index) = samples.gathering {
            return samples
                .locations
                .get(index)
                .map(|&location| (index, location));
        }
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
            .find(|(_, location)| {
                location.offset >= samples.position
                    && samples.subscribed.contains(&location.properties.track_id)
            })
            .map(|(index, &location)| (start.saturating_add(index), location))
    }
}

fn length_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
