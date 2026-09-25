//! [`Mp4EpochSeconds`], the creation and modification times of ISO/IEC 14496-12 §6.2.2

/// Point in time, counted in seconds from 1904-01-01 UTC, as a box header states it
///
/// A header states when its box was created and when it was last modified. The
/// spec counts both in seconds since midnight of 1 January 1904 in UTC — the
/// epoch of the MP4 family of formats, inherited from QuickTime rather than from
/// Unix. The seconds are what this type holds; turning them into a calendar date
/// is a caller's to do, with whatever calendar library it already has.
///
/// A time is not a duration. A `duration` field counts units of its box's own
/// time scale, has no epoch, and is held as the integer it is.
///
/// # Examples
///
/// ```
/// use isobmff_core::Mp4EpochSeconds;
///
/// // The epoch itself, which a writer states when it knows no better
/// assert_eq!(Mp4EpochSeconds::from_seconds(0).seconds(), 0);
///
/// // 2082844800 seconds on from the epoch is where the Unix one starts
/// let unix_epoch = Mp4EpochSeconds::from_seconds(2_082_844_800);
/// assert_eq!(unix_epoch.seconds(), 2_082_844_800);
/// assert_eq!(Mp4EpochSeconds::from_unix_seconds(0), Some(unix_epoch));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Mp4EpochSeconds(u64);

/// Seconds from 1904-01-01 UTC to the Unix epoch of 1970-01-01 UTC
const UNIX_EPOCH_SECONDS: u64 = 2_082_844_800;

impl Mp4EpochSeconds {
    /// Creates the time from the seconds since the epoch of 1904-01-01 UTC
    #[must_use]
    pub const fn from_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Creates the time from the seconds since the Unix epoch of 1970-01-01 UTC
    ///
    /// Seconds since 1970 reach no time before it, so a time between 1904 and
    /// 1970 is stated with [`from_seconds`](Self::from_seconds). Returns `None`
    /// for seconds so many that the count from 1904 does not fit in 64 bits.
    #[must_use]
    pub const fn from_unix_seconds(seconds: u64) -> Option<Self> {
        match seconds.checked_add(UNIX_EPOCH_SECONDS) {
            Some(seconds) => Some(Self(seconds)),
            None => None,
        }
    }

    /// Returns the seconds since the epoch of 1904-01-01 UTC
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::Mp4EpochSeconds;

    #[test]
    fn unix_seconds_past_what_the_count_from_1904_holds_state_no_time() {
        assert_eq!(Mp4EpochSeconds::from_unix_seconds(u64::MAX), None);
    }
}
