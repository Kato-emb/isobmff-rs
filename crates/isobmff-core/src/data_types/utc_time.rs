//! [`UtcTime`], the creation and modification times of ISO/IEC 14496-12 §6.2.2

/// Point in time, in seconds since 1904-01-01 UTC, as a box header states it
///
/// A header states when its box was created and when it was last modified. The
/// spec counts both in seconds since midnight of 1 January 1904 in UTC, an epoch
/// the format inherits from QuickTime rather than from the Unix one. The seconds
/// are what this type holds; turning them into a calendar date is a caller's to
/// do, with whatever calendar library it already has.
///
/// A time is not a duration. A `duration` field counts units of its box's own
/// time scale, has no epoch, and is held as the integer it is.
///
/// # Examples
///
/// ```
/// use isobmff_core::UtcTime;
///
/// // The epoch itself, which a writer states when it knows no better
/// assert_eq!(UtcTime::from_seconds(0).seconds(), 0);
///
/// // 2082844800 seconds on from the epoch is where the Unix one starts
/// let unix_epoch = UtcTime::from_seconds(2_082_844_800);
/// assert_eq!(unix_epoch.seconds(), 2_082_844_800);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UtcTime(u64);

impl UtcTime {
    /// Creates the time from the seconds since the epoch of 1904-01-01 UTC
    #[must_use]
    pub const fn from_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Returns the seconds since the epoch of 1904-01-01 UTC
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}
