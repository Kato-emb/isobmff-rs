//! [`SampleReaderLimits`], the most a [`SampleReader`](crate::SampleReader) holds

/// The most a [`SampleReader`](crate::SampleReader) holds of what a file declares
///
/// Each limit bounds memory the reader is about to take, and is checked before
/// it is taken:
///
/// * [`sample_size`](Self::sample_size): the most bytes one extent may
///   name, all of which the reader gathers before it reports the sample.
///   Past it the extent is
///   [`SampleSizeLimitExceeded`](crate::Error::SampleSizeLimitExceeded).
/// * [`held_extents`](Self::held_extents): the most extents held at once,
///   an extent counting until its sample is handed over. Past it the extent
///   is [`HeldExtentLimitExceeded`](crate::Error::HeldExtentLimitExceeded).
/// * [`held_bytes`](Self::held_bytes): the most bytes of samples held at
///   once. A sample counts the bytes its extent names from the input that
///   brings its first byte until
///   [`poll_sample`](crate::SampleReader::poll_sample) takes it. Past it
///   the input is
///   [`HeldBytesLimitExceeded`](crate::Error::HeldBytesLimitExceeded).
///
/// No relation between them is checked: whichever is reached first refuses.
///
/// # Examples
///
/// ```
/// use isobmff_sample::{SampleReader, SampleReaderLimits};
///
/// // Limits for whole uncompressed frames
/// let limits = SampleReaderLimits::new()
///     .with_sample_size(64 * 1024 * 1024)
///     .with_held_bytes(256 * 1024 * 1024);
/// assert_eq!(limits.held_extents(), SampleReaderLimits::DEFAULT_HELD_EXTENTS);
///
/// // A reader is held to them from its creation
/// let reader = SampleReader::with_limits(limits);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SampleReaderLimits {
    sample_size: u64,
    held_extents: u64,
    held_bytes: u64,
}

impl SampleReaderLimits {
    /// Most bytes one extent may name, where the caller names no limit: sixteen mebibytes
    pub const DEFAULT_SAMPLE_SIZE: u64 = 16 * 1024 * 1024;

    /// Most extents held at once, where the caller names no limit: 1,048,576
    pub const DEFAULT_HELD_EXTENTS: u64 = 1024 * 1024;

    /// Most bytes of samples held at once, where the caller names no limit: sixty-four mebibytes
    pub const DEFAULT_HELD_BYTES: u64 = 64 * 1024 * 1024;

    /// Creates the limits a reader holds where the caller names none
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sample_size: Self::DEFAULT_SAMPLE_SIZE,
            held_extents: Self::DEFAULT_HELD_EXTENTS,
            held_bytes: Self::DEFAULT_HELD_BYTES,
        }
    }

    /// Sets the most bytes one extent may name
    #[must_use]
    pub const fn with_sample_size(self, sample_size: u64) -> Self {
        Self {
            sample_size,
            ..self
        }
    }

    /// Sets the most extents held at once
    #[must_use]
    pub const fn with_held_extents(self, held_extents: u64) -> Self {
        Self {
            held_extents,
            ..self
        }
    }

    /// Sets the most bytes of samples held at once
    #[must_use]
    pub const fn with_held_bytes(self, held_bytes: u64) -> Self {
        Self { held_bytes, ..self }
    }

    /// Returns the most bytes one extent may name
    #[must_use]
    pub const fn sample_size(self) -> u64 {
        self.sample_size
    }

    /// Returns the most extents held at once
    #[must_use]
    pub const fn held_extents(self) -> u64 {
        self.held_extents
    }

    /// Returns the most bytes of samples held at once
    #[must_use]
    pub const fn held_bytes(self) -> u64 {
        self.held_bytes
    }
}

impl Default for SampleReaderLimits {
    fn default() -> Self {
        Self::new()
    }
}
