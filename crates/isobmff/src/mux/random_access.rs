//! The movie fragment random access table a [`FragmentedMovieFileMuxer`](super::FragmentedMovieFileMuxer) writes after its last fragment: the first sync sample of each track in each fragment

use alloc::vec::Vec;

use isobmff_boxes::{
    MovieFragmentBox, MovieFragmentRandomAccessBox, TrackFragmentRandomAccessBox,
    TrackFragmentRandomAccessEntry, TrackRunSamples,
};
use isobmff_core::BoxEncode;

use super::Error;
use super::output::to_bytes;
use crate::SampleProperties;

/// The first sync sample of a track in the fragment being written
#[derive(Debug, Clone)]
struct FirstSync {
    track_id: u32,
    /// Samples of the track taken in the fragment
    taken: usize,
    /// Position of the first sync sample among them, from 0, and its
    /// composition time, once one is taken
    sync: Option<(usize, u64)>,
}

/// What a fragmented muxer holds of its random access table: the entries of the fragments written, and the first sync sample of each track in the fragment being written
#[derive(Debug, Clone)]
pub(super) struct RandomAccess {
    /// The most entries the table holds
    limit: usize,
    /// The table of each track, in the order its first entry was recorded
    tracks: Vec<TrackFragmentRandomAccessBox>,
    /// The first sync sample of each track in the fragment being written
    first_syncs: Vec<FirstSync>,
}

impl RandomAccess {
    /// Returns an empty table of at most `limit` entries
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            tracks: Vec::new(),
            first_syncs: Vec::new(),
        }
    }

    /// Notes `properties`, those of a sample taken into the fragment being written
    pub(super) fn take_sample(&mut self, properties: &SampleProperties) {
        let index = self
            .first_syncs
            .iter()
            .position(|track| track.track_id == properties.track_id)
            .unwrap_or_else(|| {
                self.first_syncs.push(FirstSync {
                    track_id: properties.track_id,
                    taken: 0,
                    sync: None,
                });
                self.first_syncs.len().saturating_sub(1)
            });
        let Some(track) = self.first_syncs.get_mut(index) else {
            return;
        };
        if track.sync.is_none() && !properties.sample_flags.sample_is_non_sync_sample {
            let composition_time = i128::from(properties.decoding_time)
                .saturating_add(i128::from(properties.sample_composition_time_offset));
            // Why leave out a sync sample composed before 0 rather than refuse
            // it: the table is a hint, which need not list every sync sample,
            // and its `time` counts from 0.
            if let Ok(time) = u64::try_from(composition_time) {
                track.sync = Some((track.taken, time));
            }
        }
        track.taken = track.taken.saturating_add(1);
    }

    /// Records the first sync sample of each track in `moof`, the fragment written at `moof_offset`, while the table holds fewer entries than its limit
    pub(super) fn record(&mut self, moof: &MovieFragmentBox, moof_offset: u64) {
        let mut entry_count = self.tracks.iter().fold(0_usize, |count, tfra| {
            count.saturating_add(tfra.entries.len())
        });
        for first in core::mem::take(&mut self.first_syncs) {
            if entry_count >= self.limit {
                return;
            }
            let Some((index, time)) = first.sync else {
                continue;
            };
            let Some((traf_number, trun_number, sample_delta)) =
                locate(moof, first.track_id, index)
            else {
                continue;
            };
            let mut entry = TrackFragmentRandomAccessEntry::default();
            entry.time = time;
            entry.moof_offset = moof_offset;
            entry.traf_number = traf_number;
            entry.trun_number = trun_number;
            entry.sample_delta = sample_delta;
            match self
                .tracks
                .iter_mut()
                .find(|tfra| tfra.track_id == first.track_id)
            {
                Some(tfra) => tfra.entries.push(entry),
                None => {
                    let mut tfra = TrackFragmentRandomAccessBox::default();
                    tfra.version = 1;
                    tfra.track_id = first.track_id;
                    tfra.entries.push(entry);
                    self.tracks.push(tfra);
                }
            }
            entry_count = entry_count.saturating_add(1);
        }
    }

    /// Returns the bytes of the `mfra` of the table, or `None` where it holds no entry, and gives the table up
    ///
    /// # Errors
    /// * [`Error::Overflow`]: the `mfra` is longer than the 32 bits of the
    ///   `parent_size` of its `mfro` count.
    /// * [`Error::Encode`]: the `mfra` does not write.
    pub(super) fn into_bytes(self) -> Result<Option<Vec<u8>>, Error> {
        if self.tracks.is_empty() {
            return Ok(None);
        }
        let mut mfra = MovieFragmentRandomAccessBox::default();
        mfra.tfra = self
            .tracks
            .into_iter()
            .map(|mut tfra| {
                let widest = |number: fn(&TrackFragmentRandomAccessEntry) -> u32| match tfra
                    .entries
                    .iter()
                    .map(number)
                    .max()
                    .unwrap_or(0)
                {
                    0..=0xff => 0,
                    0x100..=0xffff => 1,
                    0x1_0000..=0xff_ffff => 2,
                    _ => 3,
                };
                tfra.length_size_of_traf_num = widest(|entry| entry.traf_number);
                tfra.length_size_of_trun_num = widest(|entry| entry.trun_number);
                tfra.length_size_of_sample_num = widest(|entry| entry.sample_delta);
                tfra
            })
            .collect();
        mfra.mfro.parent_size = u32::try_from(mfra.encoded_len()).map_err(|_| Error::Overflow)?;
        to_bytes(&mfra).map(Some)
    }
}

/// Returns the `traf_number`, the `trun_number` and the `sample_delta` of the sample at `index`, from 0, among those of track `track_id` in `moof`
fn locate(moof: &MovieFragmentBox, track_id: u32, index: usize) -> Option<(u32, u32, u32)> {
    let mut before = index;
    for (traf_index, traf) in moof.traf.iter().enumerate() {
        if traf.tfhd.track_id != track_id {
            continue;
        }
        for (trun_index, trun) in traf.trun.iter().enumerate() {
            let sample_count = match &trun.samples {
                TrackRunSamples::EmptyRows { sample_count } => {
                    usize::try_from(*sample_count).ok()?
                }
                TrackRunSamples::Rows { samples } => samples.len(),
                _ => 0,
            };
            if before < sample_count {
                return Some((
                    u32::try_from(traf_index.checked_add(1)?).ok()?,
                    u32::try_from(trun_index.checked_add(1)?).ok()?,
                    u32::try_from(before.checked_add(1)?).ok()?,
                ));
            }
            before = before.saturating_sub(sample_count);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentRandomAccessBox, SampleFlags, TrackFragmentBox,
        TrackFragmentRandomAccessEntry, TrackRunBox, TrackRunSamples,
    };
    use isobmff_core::BoxDecode;

    use super::RandomAccess;
    use crate::SampleProperties;

    fn sample(track_id: u32, decoding_time: u64, sync: bool) -> SampleProperties {
        let mut sample_flags = SampleFlags::default();
        sample_flags.sample_is_non_sync_sample = !sync;
        SampleProperties {
            track_id,
            decoding_time,
            sample_duration: 10,
            sample_composition_time_offset: 0,
            sample_flags,
            sample_description_index: 1,
        }
    }

    /// A `moof` of a `traf` for each `(track_ID, samples of each trun)`
    fn moof(trafs: &[(u32, &[u32])]) -> MovieFragmentBox {
        let mut moof = MovieFragmentBox::default();
        moof.traf = trafs
            .iter()
            .map(|&(track_id, runs)| {
                let mut traf = TrackFragmentBox::default();
                traf.tfhd.track_id = track_id;
                traf.trun = runs
                    .iter()
                    .map(|&sample_count| {
                        let mut trun = TrackRunBox::default();
                        trun.samples = TrackRunSamples::EmptyRows { sample_count };
                        trun
                    })
                    .collect();
                traf
            })
            .collect();
        moof
    }

    fn entry(
        time: u64,
        moof_offset: u64,
        numbers: (u32, u32, u32),
    ) -> TrackFragmentRandomAccessEntry {
        let mut entry = TrackFragmentRandomAccessEntry::default();
        entry.time = time;
        entry.moof_offset = moof_offset;
        (entry.traf_number, entry.trun_number, entry.sample_delta) = numbers;
        entry
    }

    /// The `track_ID` and the entries of each `tfra` of the table, and whether the `parent_size` of its `mfro` is its length
    fn table(
        random_access: &RandomAccess,
    ) -> (Vec<(u32, Vec<TrackFragmentRandomAccessEntry>)>, bool) {
        let bytes = random_access.clone().into_bytes().unwrap().unwrap();
        let (mfra, rest) = MovieFragmentRandomAccessBox::decode(&bytes).unwrap();
        let sized =
            rest.is_empty() && usize::try_from(mfra.mfro.parent_size).unwrap() == bytes.len();
        (
            mfra.tfra
                .iter()
                .map(|tfra| (tfra.track_id, tfra.entries.clone()))
                .collect(),
            sized,
        )
    }

    #[test]
    fn each_entry_points_at_the_first_sync_sample_of_its_track_through_the_traf_and_trun_that_hold_it()
     {
        let mut random_access = RandomAccess::new(16);
        for taken in [
            sample(1, 0, false),
            sample(2, 0, false),
            sample(1, 10, false),
            sample(2, 10, false),
            sample(1, 20, true),
            sample(2, 20, true),
        ] {
            random_access.take_sample(&taken);
        }
        random_access.record(&moof(&[(1, &[2, 1]), (2, &[1]), (2, &[1, 1])]), 500);

        assert_eq!(
            table(&random_access),
            (
                vec![
                    (1, vec![entry(20, 500, (1, 2, 1))]),
                    (2, vec![entry(20, 500, (3, 2, 1))])
                ],
                true
            )
        );
    }

    #[test]
    fn a_track_of_no_sync_sample_or_none_composed_from_0_has_no_entry_for_the_fragment() {
        let mut random_access = RandomAccess::new(16);
        let mut early = sample(2, 0, true);
        early.sample_composition_time_offset = -5;
        random_access.take_sample(&sample(1, 0, false));
        random_access.take_sample(&early);
        random_access.take_sample(&sample(3, 0, true));
        random_access.record(&moof(&[(1, &[1]), (2, &[1]), (3, &[1])]), 100);

        assert_eq!(
            table(&random_access),
            (vec![(3, vec![entry(0, 100, (3, 1, 1))])], true)
        );
    }

    #[test]
    fn past_the_limit_no_more_entries_are_recorded() {
        let mut random_access = RandomAccess::new(2);
        for fragment in 0..3 {
            random_access.take_sample(&sample(1, fragment * 10, true));
            random_access.record(&moof(&[(1, &[1])]), fragment * 100);
        }

        assert_eq!(
            table(&random_access),
            (
                vec![(1, vec![entry(0, 0, (1, 1, 1)), entry(10, 100, (1, 1, 1))])],
                true
            )
        );
    }

    #[test]
    fn a_number_wider_than_a_byte_widens_its_field_in_the_table() {
        let mut random_access = RandomAccess::new(16);
        for index in 0..300 {
            random_access.take_sample(&sample(1, index * 10, index == 299));
        }
        random_access.record(&moof(&[(1, &[300])]), 0);

        let bytes = random_access.into_bytes().unwrap().unwrap();
        let (mfra, _) = MovieFragmentRandomAccessBox::decode(&bytes).unwrap();
        let tfra = mfra.tfra.first().unwrap();
        assert_eq!(
            (
                tfra.length_size_of_traf_num,
                tfra.length_size_of_trun_num,
                tfra.length_size_of_sample_num,
                tfra.entries.clone()
            ),
            (0, 0, 1, vec![entry(2_990, 0, (1, 1, 300))])
        );
    }

    #[test]
    fn a_table_of_no_entry_is_not_written() {
        let mut random_access = RandomAccess::new(16);
        random_access.take_sample(&sample(1, 0, false));
        random_access.record(&moof(&[(1, &[1])]), 0);

        assert_eq!(random_access.into_bytes(), Ok(None));
    }
}
