//! [`Demuxer`], which reads a file into its boxes and the location of each sample of its movie

use crate::RequiredInput;
use crate::file_level::{BoxReader, BoxReaderConfig};

mod config;
mod error;
mod event;
mod observation;
mod samples;

pub use config::DemuxerConfig;
pub use error::Error;
pub use event::Event;
use observation::{Observation, PAYLOAD_BOX_TYPES, observe};
use samples::Samples;

/// Where the demuxer is in the file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No `moov` was read
    NoMovie,
    /// The `moov` was read
    Movie,
}

/// Reader of a file into its boxes at file level and the location of each sample of its movie
///
/// The structure of a file, ISO/IEC 14496-12 §4.2.2 and §8.2, and the
/// location of samples, §6.1
///
/// The demuxer is driven as a [`BoxReader`] is: it names where it wants input
/// with [`required_input`](Self::required_input), takes bytes from any offset
/// up to that one with [`handle_input`](Self::handle_input), and reports one
/// event per call of [`poll_event`](Self::poll_event). It reads the `ftyp`
/// and the `moov` into their values, reports every other box as
/// [`Event::Box`] without requesting its payload, and, once it has read the
/// `moov`, reports where every sample of it lies, in the order of their
/// offsets, before the box that follows. Handing input over never fails: what
/// cannot be read, such as a second `moov`, is reported as
/// [`Event::Error`], and reading goes on. Whether to hand over bytes where
/// the demuxer asks is the caller's choice: the demuxer does not seek, and
/// requests the bytes of no sample.
///
/// A movie file whose `moov` comes last, read where the demuxer asks:
///
/// ```
/// use isobmff::demux::Event;
/// use isobmff::{DataOrigin, Demuxer, DemuxerConfig};
///
/// let file: &[u8] = include_bytes!("../tests/fixtures/movie_moov_last.mp4");
/// let mut demuxer = Demuxer::new(DemuxerConfig::default());
/// let mut locations = Vec::new();
/// loop {
///     // Take what the bytes handed over hold
///     while let Some(event) = demuxer.poll_event() {
///         if let Event::SampleLocation(location) = event {
///             locations.push(location);
///         }
///     }
///     // Hand over the bytes from where the demuxer asks
///     let Some(required) = demuxer.required_input() else { break };
///     match file.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
///         _ => demuxer.finish(),
///     }
/// }
///
/// // Every sample of both tracks, in the order of their file offsets
/// assert_eq!(locations.len(), 12 + 58);
/// assert!(locations.windows(2).all(|pair| pair[0].offset <= pair[1].offset));
/// assert!(locations.iter().all(|location| location.data_origin == DataOrigin::Container));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct Demuxer {
    boxes: BoxReader,
    samples: Samples,
    state: State,
}

impl Demuxer {
    /// Creates a demuxer of a file from its first byte
    #[must_use]
    pub fn new(config: DemuxerConfig) -> Self {
        let mut boxes = BoxReaderConfig::new(config.payload_limit);
        boxes.payload_box_types = PAYLOAD_BOX_TYPES
            .iter()
            .map(|&(box_type, _)| box_type)
            .collect();
        Self {
            boxes: BoxReader::new(boxes),
            samples: Samples::new(config.sample_count_limit),
            state: State::NoMovie,
        }
    }

    /// Returns where the demuxer wants input next, or `None` once it wants none
    ///
    /// The position is the file offset of the first byte wanted, which the
    /// box reader wants.
    #[must_use]
    pub fn required_input(&self) -> Option<RequiredInput> {
        self.boxes.required_input()
    }

    /// Takes bytes of the file that start at the file offset `position`
    ///
    /// Bytes before the position [`required_input`](Self::required_input)
    /// names are ignored. The demuxer takes bytes until it has the next event,
    /// and none while an event waits for [`poll_event`](Self::poll_event); it
    /// copies the box headers and the payloads it reads into values.
    ///
    /// # Panics
    /// Panics if `position` is after the position `required_input` names, or
    /// if called after [`finish`](Self::finish): either is a bug of the caller.
    pub fn handle_input(&mut self, position: u64, bytes: &[u8]) {
        // Why no bytes while a location waits: the box reader still asserts
        // where the input starts and that it has not ended, and takes nothing.
        let bytes = if self.samples.unreported().is_some() {
            &[]
        } else {
            bytes
        };
        self.boxes.handle_input(position, bytes);
    }

    /// Returns what the bytes handed over hold, one event at a time, or `None`
    /// once the demuxer needs more input
    pub fn poll_event(&mut self) -> Option<Event> {
        let observation = match self.samples.unreported() {
            Some(location) => Observation::Unreported(location),
            None => self
                .boxes
                .poll_event()
                .map_or(Observation::NeedMoreData, observe),
        };
        let (state, event) = match (self.state, observation) {
            (state, Observation::Unreported(location)) => {
                self.samples.report();
                (state, Some(Event::SampleLocation(location)))
            }
            (State::NoMovie, Observation::Movie { offset, movie }) => {
                match self.samples.handle_movie(offset, &movie) {
                    Ok(()) => (State::Movie, Some(Event::Movie(movie))),
                    Err(error) => (State::NoMovie, Some(Event::Error(error))),
                }
            }
            (State::Movie, Observation::Movie { offset, .. }) => (
                State::Movie,
                Some(Event::Error(Error::DuplicateMovie { offset })),
            ),
            (state, Observation::FileType(file_type)) => (state, Some(Event::FileType(file_type))),
            (state, Observation::Box { offset, header }) => {
                (state, Some(Event::Box { offset, header }))
            }
            (
                state,
                Observation::Decode {
                    offset,
                    box_type,
                    source,
                },
            ) => (
                state,
                Some(Event::Error(Error::Decode {
                    offset,
                    box_type,
                    source,
                })),
            ),
            (state, Observation::FileLevel(error)) => {
                (state, Some(Event::Error(Error::FileLevel(error))))
            }
            (state, Observation::NeedMoreData) => (state, None),
        };
        self.state = state;
        event
    }

    /// Ends the input at the bytes handed over so far
    ///
    /// The events that follow report what the box reader reports of the end
    /// of the input, such as a `moov` the input ends inside.
    pub fn finish(&mut self) {
        self.boxes.finish();
    }
}
