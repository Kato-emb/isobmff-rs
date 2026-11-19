//! [`Error`], what a [`MediaDataReader`](super::MediaDataReader) cannot assemble

use core::fmt;

use crate::SampleLocation;

/// What a [`MediaDataReader`](super::MediaDataReader) cannot assemble
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// A sample is one more than
    /// [`MediaDataReader::new`](super::MediaDataReader::new) allows the
    /// reader to hold; it is not taken
    SampleCountOverLimit {
        /// Number of samples the reader would hold
        sample_count: u64,
        /// The most samples the reader holds
        limit: u64,
    },
    /// A sample is larger than
    /// [`MediaDataReader::new`](super::MediaDataReader::new) allows; its
    /// bytes are not requested, and reading goes on at the next sample
    SampleOverLimit {
        /// Where the sample lies
        location: SampleLocation,
        /// The limit the sample is larger than
        limit: u64,
    },
    /// The input ends inside a sample
    TruncatedSample {
        /// Where the sample lies
        location: SampleLocation,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleCountOverLimit {
                sample_count,
                limit,
            } => write!(
                formatter,
                "{sample_count} samples held, over the limit of {limit}"
            ),
            Self::SampleOverLimit { location, limit } => write!(
                formatter,
                "the sample of track {} at offset {} is {} bytes, over the limit of {limit}",
                location.properties.track_id, location.offset, location.size
            ),
            Self::TruncatedSample { location } => write!(
                formatter,
                "the input ends inside the sample of track {} at offset {}",
                location.properties.track_id, location.offset
            ),
        }
    }
}

impl core::error::Error for Error {}
