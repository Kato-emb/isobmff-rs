//! [`TrackDecodeTimes`], where the media timeline of every track stands between fragments

use alloc::collections::BTreeMap;

use isobmff_boxes::MovieBox;

use crate::error::Error;

/// Where the media timeline of every track stands, carried from one fragment to the next
///
/// A movie fragment states when its first sample is decoded only where it
/// carries a `tfdt`; where it carries none, the samples of the track carry on
/// from where the samples before them left it — the sum of their durations,
/// those the sample table of the movie declares among them, ISO/IEC 14496-12
/// §8.8.12. This holds that sum per track. A caller resolving fragments one
/// after another owns one and hands it to
/// [`movie_fragment::sample_extents`](crate::movie_fragment::sample_extents)
/// with each, which moves it past the samples resolved.
///
/// A track no fragment has been resolved for stands where the sample table of
/// its `trak` leaves it in the times [`new`](Self::new) creates — at zero,
/// where its timeline begins, for a table declaring no sample — and nowhere
/// known in the times [`unknown`](Self::unknown) creates for a reader that did
/// not start at the first fragment.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackDecodeTimes {
    decode_times: BTreeMap<u32, u64>,
}

impl TrackDecodeTimes {
    /// Creates the times of `movie` no fragment of which has been resolved
    ///
    /// Each track stands at the sum of the sample deltas its `stts` states
    /// (§8.6.1.2).
    ///
    /// # Errors
    ///
    /// * [`DecodeTimeOverflow`](crate::ErrorKind::DecodeTimeOverflow): the
    ///   deltas of a track sum past what 64 bits carry.
    pub fn new(movie: &MovieBox) -> Result<Self, Error> {
        let decode_times = movie
            .trak()
            .iter()
            .map(|trak| {
                let track_id = trak.tkhd().track_id();
                let decode_time = trak.mdia().minf().stbl().stts().media_duration();

                decode_time
                    .map(|decode_time| (track_id, decode_time))
                    .ok_or(Error::decode_time_overflow(track_id))
            })
            .collect::<Result<_, _>>()?;

        Ok(Self { decode_times })
    }

    /// Creates the times of a movie read from a fragment past its first, where no track stands anywhere known
    ///
    /// A track is known again once a fragment carrying a `tfdt` for it is
    /// resolved.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            decode_times: BTreeMap::new(),
        }
    }

    /// Returns where the next sample of `track_id` starts, in the time scale of the track, if that is known
    #[must_use]
    pub fn decode_time(&self, track_id: u32) -> Option<u64> {
        self.decode_times.get(&track_id).copied()
    }

    /// Moves `track_id` to `decode_time`, where its next sample starts
    pub(crate) fn reach(&mut self, track_id: u32, decode_time: u64) {
        self.decode_times.insert(track_id, decode_time);
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::{
        ChunkOffsets, SampleSizeBox, SampleSizes, SampleToChunkBox, TimeToSampleBox,
        TimeToSampleEntry, TrackBox,
    };
    use isobmff_test_support::{
        movie_declaring, sample_table, self_contained_data_reference, track, track_laid_out,
        unfragmented_movie,
    };

    use super::TrackDecodeTimes;
    use crate::error::Error;

    /// Track `track_id` whose sample table states `stts`
    fn track_timed_by(track_id: u32, stts: TimeToSampleBox) -> TrackBox {
        track_laid_out(
            track_id,
            self_contained_data_reference(),
            sample_table(
                stts,
                SampleToChunkBox::new(vec![]),
                SampleSizes::Stsz(SampleSizeBox::from_sizes([])),
                ChunkOffsets::from_offsets(vec![]),
            ),
        )
    }

    #[test]
    fn each_track_stands_where_its_sample_table_leaves_it() {
        let movie = movie_declaring(vec![
            track_timed_by(1, TimeToSampleBox::from_deltas([1_024, 1_024, 512])),
            track(2),
            track_timed_by(3, TimeToSampleBox::from_deltas([3_000])),
        ]);

        let decode_times = TrackDecodeTimes::new(&movie).unwrap();

        assert_eq!(
            [1, 2, 3, 4].map(|track_id| decode_times.decode_time(track_id)),
            [Some(2_560), Some(0), Some(3_000), None]
        );
    }

    #[test]
    fn a_sample_table_whose_deltas_sum_past_64_bits_is_rejected() {
        let longest_run = TimeToSampleEntry::new(u32::MAX, u32::MAX);
        let movie = movie_declaring(vec![
            track(1),
            track_timed_by(2, TimeToSampleBox::new(vec![longest_run, longest_run])),
        ]);

        assert_eq!(
            TrackDecodeTimes::new(&movie),
            Err(Error::decode_time_overflow(2))
        );
    }

    #[test]
    fn a_track_stands_where_it_was_last_moved_to() {
        let mut decode_times = TrackDecodeTimes::new(&unfragmented_movie()).unwrap();

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
