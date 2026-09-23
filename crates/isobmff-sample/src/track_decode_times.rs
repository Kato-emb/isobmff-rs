//! [`TrackDecodeTimes`], where the media timeline of every track stands between fragments

use alloc::collections::BTreeMap;

/// Where the media timeline of every track stands, carried from one fragment to the next
///
/// A movie fragment states when its first sample is decoded only where it
/// carries a `tfdt`; where it carries none, the samples of the track carry on
/// from where the samples before them left it — the sum of their durations,
/// ISO/IEC 14496-12 §8.8.12. This holds that sum per track. A caller
/// resolving fragments one after another owns one and hands it to
/// [`movie_fragment::sample_extents`](crate::movie_fragment::sample_extents)
/// with each, which moves it past the samples resolved.
///
/// A track no sample has been resolved for stands at zero, where its timeline
/// begins, in the times [`new`](Self::new) creates, and nowhere known in the
/// times [`unknown`](Self::unknown) creates for a reader that did not start
/// at the first fragment.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackDecodeTimes {
    decode_times: BTreeMap<u32, u64>,
    unresolved: Option<u64>,
}

impl TrackDecodeTimes {
    /// Creates the times of a movie no fragment of which has been resolved
    #[must_use]
    pub const fn new() -> Self {
        Self {
            decode_times: BTreeMap::new(),
            unresolved: Some(0),
        }
    }

    /// Creates the times of a movie read from a fragment past its first, where no track stands anywhere known
    ///
    /// A track is known again once a fragment carrying a `tfdt` for it is
    /// resolved.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            decode_times: BTreeMap::new(),
            unresolved: None,
        }
    }

    /// Returns where the next sample of `track_id` starts, in the time scale of the track, if that is known
    #[must_use]
    pub fn decode_time(&self, track_id: u32) -> Option<u64> {
        self.decode_times
            .get(&track_id)
            .copied()
            .or(self.unresolved)
    }

    /// Moves `track_id` to `decode_time`, where its next sample starts
    pub(crate) fn reach(&mut self, track_id: u32, decode_time: u64) {
        self.decode_times.insert(track_id, decode_time);
    }
}

impl Default for TrackDecodeTimes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::TrackDecodeTimes;

    #[test]
    fn a_track_no_sample_was_resolved_for_stands_at_zero() {
        assert_eq!(TrackDecodeTimes::new().decode_time(1), Some(0));
        assert_eq!(TrackDecodeTimes::default(), TrackDecodeTimes::new());
    }

    #[test]
    fn a_track_stands_where_it_was_last_moved_to() {
        let mut decode_times = TrackDecodeTimes::new();

        decode_times.reach(1, 125);
        decode_times.reach(2, 10);
        decode_times.reach(1, 4_100);

        assert_eq!(decode_times.decode_time(1), Some(4_100));
        assert_eq!(decode_times.decode_time(2), Some(10));
    }

    #[test]
    fn in_unknown_times_only_a_track_moved_somewhere_stands_anywhere_known() {
        let mut decode_times = TrackDecodeTimes::unknown();

        decode_times.reach(1, 125);

        assert_eq!(decode_times.decode_time(1), Some(125));
        assert_eq!(decode_times.decode_time(2), None);
    }
}
