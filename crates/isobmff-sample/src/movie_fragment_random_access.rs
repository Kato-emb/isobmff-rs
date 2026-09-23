//! [`sync_sample_at`], a time looked up in the random access table of a track, ISO/IEC 14496-12 §8.8.10

use isobmff_boxes::{TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry};

/// Returns the entry of the latest sync sample `tfra` lists at or before presentation `time`
///
/// `time` is in the time scale of the track, the one its `mdhd` declares,
/// as the `time` of every entry is. The entries are searched whole rather
/// than taken to be in order, and `None` comes back when every entry lies
/// past `time` or the table lists none — the case of a track all of whose
/// samples are sync samples, where the file holds no place to look up.
///
/// The entry names the `moof` holding the sync sample; reading from there
/// yields the samples of that `moof` from its first on, the sync sample among
/// them at the `traf`, `trun` and sample numbers the entry states.
#[must_use]
pub fn sync_sample_at(
    tfra: &TrackFragmentRandomAccessBox,
    time: u64,
) -> Option<&TrackFragmentRandomAccessEntry> {
    tfra.entries()
        .iter()
        .filter(|entry| entry.time() <= time)
        .max_by_key(|entry| entry.time())
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use isobmff_boxes::{TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry};

    use super::sync_sample_at;

    /// Entry of the sync sample at `time`, first of the `moof` at `moof_offset`
    fn entry(time: u64, moof_offset: u64) -> TrackFragmentRandomAccessEntry {
        TrackFragmentRandomAccessEntry::new(
            time,
            moof_offset,
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            NonZeroU32::MIN,
        )
    }

    #[test]
    fn a_time_is_looked_up_in_the_latest_sync_sample_at_or_before_it() {
        let table = TrackFragmentRandomAccessBox::new(
            1,
            vec![entry(3_000, 5_000), entry(0, 1_000), entry(6_000, 9_000)],
        );

        assert_eq!(sync_sample_at(&table, 0), Some(&entry(0, 1_000)));
        assert_eq!(sync_sample_at(&table, 2_999), Some(&entry(0, 1_000)));
        assert_eq!(sync_sample_at(&table, 3_000), Some(&entry(3_000, 5_000)));
        assert_eq!(sync_sample_at(&table, u64::MAX), Some(&entry(6_000, 9_000)));
    }

    #[test]
    fn a_time_before_every_entry_or_a_table_listing_none_finds_nothing() {
        let late = TrackFragmentRandomAccessBox::new(1, vec![entry(3_000, 5_000)]);

        assert_eq!(sync_sample_at(&late, 2_999), None);
        assert_eq!(
            sync_sample_at(&TrackFragmentRandomAccessBox::new(1, Vec::new()), 0),
            None
        );
    }
}
