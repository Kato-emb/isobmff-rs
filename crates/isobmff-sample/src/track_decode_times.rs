//! [`TrackDecodeTimes`], where the media timeline of every track stands between fragments

use alloc::collections::BTreeMap;

use crate::error::SampleError;
use crate::sample::SampleExtent;

/// Where the media timeline of every track stands, carried from one fragment to the next
///
/// A movie fragment states when its first sample is decoded only where it
/// carries a `tfdt`; where it carries none, the samples of the track carry on
/// from where the samples before them left it — the sum of their durations,
/// ISO/IEC 14496-12 §8.8.12. This holds that sum per track, and a caller
/// resolving fragments one after another [`advance`](Self::advance)s it past
/// each sample resolved.
///
/// A track no sample has been resolved for stands at zero, where its timeline
/// begins.
///
/// # Examples
///
/// ```
/// use isobmff_sample::{SampleExtent, TrackDecodeTimes};
///
/// let mut decode_times = TrackDecodeTimes::new();
/// assert_eq!(decode_times.decode_time(1), 0);
///
/// // The last sample resolved leaves the track where the next one starts
/// let last = SampleExtent::new(1, 3_000, 1_000, 0, 0, 1, 1, 64..128);
/// decode_times.advance(&last)?;
/// assert_eq!(decode_times.decode_time(1), 4_000);
/// assert_eq!(decode_times.decode_time(2), 0);
/// # Ok::<(), isobmff_sample::SampleError>(())
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct TrackDecodeTimes {
    decode_times: BTreeMap<u32, u64>,
}

impl TrackDecodeTimes {
    /// Creates the times of a movie no fragment of which has been resolved
    #[must_use]
    pub const fn new() -> Self {
        Self {
            decode_times: BTreeMap::new(),
        }
    }

    /// Returns where the next sample of `track_id` starts, in the time scale of the track
    #[must_use]
    pub fn decode_time(&self, track_id: u32) -> u64 {
        self.decode_times.get(&track_id).copied().unwrap_or(0)
    }

    /// Moves the track of `last` to where that sample ends
    ///
    /// # Errors
    ///
    /// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow): the
    ///   end of the sample runs past what 64 bits carry.
    pub fn advance(&mut self, last: &SampleExtent) -> Result<(), SampleError> {
        let end = last
            .decode_time()
            .checked_add(u64::from(last.sample_duration()))
            .ok_or(SampleError::decode_time_overflow(last.track_id()))?;
        self.decode_times.insert(last.track_id(), end);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::error::SampleError;
    use crate::sample::SampleExtent;

    use super::TrackDecodeTimes;

    fn extent(track_id: u32, decode_time: u64, sample_duration: u32) -> SampleExtent {
        SampleExtent::new(track_id, decode_time, sample_duration, 0, 0, 1, 1, 0..0)
    }

    #[test]
    fn a_track_no_sample_was_resolved_for_stands_at_zero() {
        assert_eq!(TrackDecodeTimes::new().decode_time(1), 0);
    }

    #[test]
    fn a_track_stands_where_the_last_sample_resolved_for_it_ends() {
        let mut decode_times = TrackDecodeTimes::new();

        decode_times.advance(&extent(1, 100, 25)).unwrap();
        decode_times.advance(&extent(2, 7, 3)).unwrap();
        decode_times.advance(&extent(1, 4_096, 4)).unwrap();

        assert_eq!(decode_times.decode_time(1), 4_100);
        assert_eq!(decode_times.decode_time(2), 10);
    }

    #[test]
    fn a_sample_ending_past_what_64_bits_carry_is_refused_and_moves_nothing() {
        let mut decode_times = TrackDecodeTimes::new();
        decode_times.advance(&extent(1, 100, 25)).unwrap();

        assert_eq!(
            decode_times.advance(&extent(1, u64::MAX, 1)),
            Err(SampleError::decode_time_overflow(1))
        );
        assert_eq!(decode_times.decode_time(1), 125);
    }
}
