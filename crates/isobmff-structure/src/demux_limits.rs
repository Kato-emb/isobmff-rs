//! [`DemuxLimits`], the most a demux FSM holds

use isobmff_sample::SampleReaderLimits;

/// The most a demux FSM holds of what a file declares
///
/// Each limit bounds memory the demux FSM is about to take, and is checked
/// before it is taken:
///
/// * [`payload`](Self::payload): the most payload one box read into a value
///   may declare, all of which is gathered before the box is read. Past it
///   the box is
///   [`PayloadLimitExceeded`](crate::Error::PayloadLimitExceeded).
/// * [`resolved_samples`](Self::resolved_samples): the most samples one
///   `moov` or one `moof` may declare, every track together. Past it the
///   box lays out none, as
///   [`SampleCountLimitExceeded`](isobmff_sample::Error::SampleCountLimitExceeded)
///   carried on [`Sample`](crate::Error::Sample).
/// * [`sample_reader`](Self::sample_reader): the limits the
///   [`SampleReader`](isobmff_sample::SampleReader) the samples are gathered
///   by is held to, whose failures are carried on
///   [`Sample`](crate::Error::Sample) as well.
///
/// No relation between them is checked: whichever is reached first refuses.
///
/// # Examples
///
/// ```
/// use isobmff_sample::SampleReaderLimits;
/// use isobmff_structure::{DemuxLimits, MovieDemuxFsm};
///
/// // Limits for a `moov` of many tracks and large samples
/// let limits = DemuxLimits::new()
///     .with_payload(64 * 1024 * 1024)
///     .with_sample_reader(SampleReaderLimits::new().with_held_bytes(256 * 1024 * 1024));
/// assert_eq!(limits.resolved_samples(), DemuxLimits::DEFAULT_RESOLVED_SAMPLES);
///
/// // A demux FSM is held to them from its creation
/// let demux_fsm = MovieDemuxFsm::with_limits(limits);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DemuxLimits {
    payload: u64,
    resolved_samples: u64,
    sample_reader: SampleReaderLimits,
}

impl DemuxLimits {
    /// Most payload one box read into a value may declare, where the caller names no limit: sixteen mebibytes
    pub const DEFAULT_PAYLOAD: u64 = 16 * 1024 * 1024;

    /// Most samples one `moov` or one `moof` may declare, where the caller names no limit: 1,048,576
    pub const DEFAULT_RESOLVED_SAMPLES: u64 = 1024 * 1024;

    /// Creates the limits a demux FSM holds where the caller names none
    ///
    /// The sample reader is held to [`SampleReaderLimits::new`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            payload: Self::DEFAULT_PAYLOAD,
            resolved_samples: Self::DEFAULT_RESOLVED_SAMPLES,
            sample_reader: SampleReaderLimits::new(),
        }
    }

    /// Sets the most payload one box read into a value may declare
    #[must_use]
    pub const fn with_payload(self, payload: u64) -> Self {
        Self { payload, ..self }
    }

    /// Sets the most samples one `moov` or one `moof` may declare
    #[must_use]
    pub const fn with_resolved_samples(self, resolved_samples: u64) -> Self {
        Self {
            resolved_samples,
            ..self
        }
    }

    /// Sets the limits the sample reader is held to
    #[must_use]
    pub const fn with_sample_reader(self, sample_reader: SampleReaderLimits) -> Self {
        Self {
            sample_reader,
            ..self
        }
    }

    /// Returns the most payload one box read into a value may declare
    #[must_use]
    pub const fn payload(self) -> u64 {
        self.payload
    }

    /// Returns the most samples one `moov` or one `moof` may declare
    #[must_use]
    pub const fn resolved_samples(self) -> u64 {
        self.resolved_samples
    }

    /// Returns the limits the sample reader is held to
    #[must_use]
    pub const fn sample_reader(self) -> SampleReaderLimits {
        self.sample_reader
    }
}

impl Default for DemuxLimits {
    fn default() -> Self {
        Self::new()
    }
}
