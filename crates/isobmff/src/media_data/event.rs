//! What a [`MediaDataReader`](super::MediaDataReader) reports

use super::error::Error;
use crate::{SampleLocation, SampleProperties};

/// What a [`MediaDataReader`](super::MediaDataReader) assembled from its input
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event<'a> {
    /// A sample whose bytes are all in hand, lent from the reader's buffer
    Sample(Sample<'a>),
    /// A sample that cannot be assembled; reading goes on at the next sample
    Error(Error),
}

/// A `sample`, with its bytes
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sample<'a> {
    /// What the structure-data declares of the sample
    pub properties: SampleProperties,
    /// File offset of the first byte of the sample
    pub offset: u64,
    /// The bytes of the sample
    pub bytes: &'a [u8],
}

impl Sample<'_> {
    /// Returns where the bytes of the sample lie
    #[must_use]
    pub fn location(&self) -> SampleLocation {
        SampleLocation {
            properties: self.properties,
            offset: self.offset,
            size: u32::try_from(self.bytes.len()).unwrap_or(u32::MAX),
        }
    }
}
