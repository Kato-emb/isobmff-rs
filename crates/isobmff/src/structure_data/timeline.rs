//! When the samples of a track are decoded and composed, and how long they last
//!
//! The track time structures, ISO/IEC 14496-12 §8.6, give each sample of a
//! sample table its timing: its decoding time and duration (`stts`), its
//! composition offset (`ctts`), and whether it is a sync sample (`stss`) and
//! what it depends on (`sdtp`). [`media_duration`] and [`composition_end`]
//! say how long the samples of a sample table last.

use core::iter::FusedIterator;
use core::ops::RangeInclusive;
use core::slice;

use isobmff_boxes::{
    CompositionOffsetBox, CompositionOffsetEntry, SampleDependencyTypeBox,
    SampleDependencyTypeEntry, SampleFlags, SampleTableBox, SyncSampleBox, TimeToSampleBox,
    TimeToSampleEntry,
};
use isobmff_core::{BoxDefinition, BoxType};

use super::Error;

/// The timing the track time structures, ISO/IEC 14496-12 §8.6, give one sample
///
/// The decoding time and `sample_duration` come from the time-to-sample
/// table (§8.6.1.2), the `sample_composition_time_offset` from the
/// composition offset table (§8.6.1.3), and the `sample_flags` from the sync
/// sample table (§8.6.2) and the sample dependency types (§8.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SampleTiming {
    /// Decoding time of the sample, in the timescale of the media: the sum of
    /// the `sample_delta`s of the samples before it
    pub(super) decoding_time: u64,
    /// `sample_delta` of the sample, the difference between its decoding
    /// time and that of the next
    pub(super) sample_duration: u32,
    /// `sample_offset` of the sample, or 0 past the composition offset table
    /// or without one
    pub(super) sample_composition_time_offset: i64,
    /// Whether the sample is a sync sample, and what the sample dependency
    /// types declare of it; the other flags are 0
    pub(super) sample_flags: SampleFlags,
}

/// Returns the [`SampleTiming`] of each of the first `sample_count` samples the tables declare, in decoding order
///
/// The samples end where the time-to-sample table does, if it declares
/// fewer. A sample past the composition offset table, or the sample
/// dependency types, takes the value that table gives without it, and an
/// entry of the sync sample table that is not after the one before it is
/// passed over; [`check_sync_samples`] refuses such an entry.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the sample table reads the timing of its samples with it"
    )
)]
pub(super) fn sample_timings<'a>(
    stts: &'a TimeToSampleBox,
    ctts: Option<&'a CompositionOffsetBox>,
    stss: Option<&'a SyncSampleBox>,
    sdtp: Option<&'a SampleDependencyTypeBox>,
    sample_count: u32,
) -> SampleTimings<'a> {
    SampleTimings {
        sample_deltas: SampleRuns::new(&stts.entries),
        sample_offsets: SampleRuns::new(
            ctts.map(|ctts| ctts.entries.as_slice()).unwrap_or_default(),
        ),
        sync_samples: stss.map(|stss| stss.sample_numbers.as_slice()),
        dependencies: sdtp
            .map(|sdtp| sdtp.entries.as_slice())
            .unwrap_or_default()
            .iter(),
        decoding_time: 0,
        sample_numbers: 1..=sample_count,
    }
}

/// The [`SampleTiming`] of each sample of a sample table, as [`sample_timings`] returns them
#[derive(Debug, Clone)]
pub(super) struct SampleTimings<'a> {
    sample_deltas: SampleRuns<'a, TimeToSampleEntry>,
    sample_offsets: SampleRuns<'a, CompositionOffsetEntry>,
    /// The entries of the sync sample table from the first that may name the
    /// next sample, or `None` where the track has none: every sample is a
    /// sync sample, §8.6.2.1
    sync_samples: Option<&'a [u32]>,
    dependencies: slice::Iter<'a, SampleDependencyTypeEntry>,
    decoding_time: u64,
    /// Numbers, from 1, of the samples not yet taken
    sample_numbers: RangeInclusive<u32>,
}

impl Iterator for SampleTimings<'_> {
    type Item = SampleTiming;

    fn next(&mut self) -> Option<SampleTiming> {
        let sample_number = self.sample_numbers.next()?;
        let sample_duration = self.sample_deltas.next()?;
        let is_non_sync_sample = self.sync_samples.as_mut().is_some_and(|numbers| {
            let passed = numbers
                .iter()
                .take_while(|&&number| number < sample_number)
                .count();
            *numbers = numbers.get(passed..).unwrap_or_default();
            numbers.first() != Some(&sample_number)
        });
        let mut sample_flags = SampleFlags::default();
        if let Some(entry) = self.dependencies.next() {
            sample_flags.is_leading = entry.is_leading;
            sample_flags.sample_depends_on = entry.sample_depends_on;
            sample_flags.sample_is_depended_on = entry.sample_is_depended_on;
            sample_flags.sample_has_redundancy = entry.sample_has_redundancy;
        }
        sample_flags.sample_is_non_sync_sample = is_non_sync_sample;
        let timing = SampleTiming {
            decoding_time: self.decoding_time,
            sample_duration,
            sample_composition_time_offset: self.sample_offsets.next().unwrap_or(0),
            sample_flags,
        };
        // Why saturate rather than refuse a sum past `u64`: at most
        // `u32::MAX` samples of at most `u32::MAX` each sum below `u64::MAX`.
        self.decoding_time = self
            .decoding_time
            .saturating_add(u64::from(sample_duration));
        Some(timing)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.sample_numbers.size_hint().1)
    }
}

impl FusedIterator for SampleTimings<'_> {}

/// An entry of a table that gives a run of consecutive samples one value
trait SampleRun {
    /// Type of the value the entry gives each sample of its run
    type Value: Copy + Default;

    /// Number of the samples of the run
    fn sample_count(&self) -> u32;

    /// The value the entry gives each sample of its run
    fn value(&self) -> Self::Value;
}

impl SampleRun for TimeToSampleEntry {
    type Value = u32;

    fn sample_count(&self) -> u32 {
        self.sample_count
    }

    fn value(&self) -> u32 {
        self.sample_delta
    }
}

impl SampleRun for CompositionOffsetEntry {
    type Value = i64;

    fn sample_count(&self) -> u32 {
        self.sample_count
    }

    fn value(&self) -> i64 {
        self.sample_offset
    }
}

/// The value each sample takes from the runs of samples of a table, in the order of the samples
#[derive(Debug, Clone)]
struct SampleRuns<'a, Entry: SampleRun> {
    entries: slice::Iter<'a, Entry>,
    /// Number of the samples of the current run not yet taken
    remaining: u32,
    /// The value of the current run
    value: Entry::Value,
}

impl<'a, Entry: SampleRun> SampleRuns<'a, Entry> {
    fn new(entries: &'a [Entry]) -> Self {
        Self {
            entries: entries.iter(),
            remaining: 0,
            value: Entry::Value::default(),
        }
    }

    /// Takes up to `at_most` samples of the current run, from the next run where it is used up, and returns how many it took with their value, or `None` past the last run
    fn next_run(&mut self, at_most: u32) -> Option<(u32, Entry::Value)> {
        while self.remaining == 0 {
            let entry = self.entries.next()?;
            self.remaining = entry.sample_count();
            self.value = entry.value();
        }
        let taken = self.remaining.min(at_most);
        self.remaining = self.remaining.saturating_sub(taken);
        Some((taken, self.value))
    }
}

impl<Entry: SampleRun> Iterator for SampleRuns<'_, Entry> {
    type Item = Entry::Value;

    fn next(&mut self) -> Option<Entry::Value> {
        self.next_run(1).map(|(_, value)| value)
    }
}

/// Returns the number of samples the runs of `entries` hold, or `u64::MAX` past what a `u64` counts
fn run_total<Entry: SampleRun>(entries: &[Entry]) -> u64 {
    entries.iter().fold(0_u64, |total, entry| {
        total.saturating_add(u64::from(entry.sample_count()))
    })
}

/// Returns the number of samples each of `stts`, `ctts` and `sdtp` that is present declares, with its box type
///
/// A count past what a `u64` counts is `u64::MAX`.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the sample table holds each count against that of its sample size table"
    )
)]
pub(super) fn table_sample_counts(
    stts: &TimeToSampleBox,
    ctts: Option<&CompositionOffsetBox>,
    sdtp: Option<&SampleDependencyTypeBox>,
) -> impl Iterator<Item = (BoxType, u64)> {
    [
        Some((TimeToSampleBox::BOX_TYPE, run_total(&stts.entries))),
        ctts.map(|ctts| (CompositionOffsetBox::BOX_TYPE, run_total(&ctts.entries))),
        sdtp.map(|sdtp| {
            (
                SampleDependencyTypeBox::BOX_TYPE,
                u64::try_from(sdtp.entries.len()).unwrap_or(u64::MAX),
            )
        }),
    ]
    .into_iter()
    .flatten()
}

/// Refuses a sync sample table of track `track_id` whose entries do not go up strictly, or name a sample past `sample_count`
///
/// Each entry is checked against the samples first, then against the entry
/// before it, so an entry that fails both is reported out of range.
///
/// # Errors
///
/// * [`SyncSampleNumberOutOfRange`](Error::SyncSampleNumberOutOfRange): the
///   first entry whose `sample_number` is 0 or past `sample_count`, ISO/IEC
///   14496-12 §8.6.2.3.
/// * [`SyncSampleNumberNotIncreasing`](Error::SyncSampleNumberNotIncreasing):
///   the first entry whose `sample_number` is not after that of the entry
///   before it, §8.6.2.1.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the sample table refuses a sync sample table as the specification reads it"
    )
)]
pub(super) fn check_sync_samples(
    stss: &SyncSampleBox,
    sample_count: u32,
    track_id: u32,
) -> Result<(), Error> {
    let mut previous_sample_number = 0;
    for (entry, &sample_number) in stss.sample_numbers.iter().enumerate() {
        let entry = u64::try_from(entry).unwrap_or(u64::MAX);
        if sample_number == 0 || sample_number > sample_count {
            return Err(Error::SyncSampleNumberOutOfRange {
                track_id,
                entry,
                sample_number,
                sample_count,
            });
        }
        if sample_number <= previous_sample_number {
            return Err(Error::SyncSampleNumberNotIncreasing {
                track_id,
                entry,
                sample_number,
                previous_sample_number,
            });
        }
        previous_sample_number = sample_number;
    }
    Ok(())
}

/// Returns the duration of the media whose samples `stbl` declares: the sum of the durations of its samples, in the timescale of the media
///
/// The duration of the media is that of its samples laid end to end on the
/// decoding timeline, ISO/IEC 14496-12 §8.4.2, each lasting the
/// `sample_delta` the time-to-sample table gives it, §8.6.1.2. A table
/// without a time-to-sample table declares a duration of 0; one past what a
/// `u64` counts, `u64::MAX`.
///
/// ```
/// use isobmff::boxes::{SampleTableBox, TimeToSampleBox, TimeToSampleEntry};
/// use isobmff::structure_data::timeline::media_duration;
///
/// // 3 samples lasting 1,000 each, then 1 lasting 500
/// let mut stts = TimeToSampleBox::default();
/// stts.entries = [(3, 1_000), (1, 500)]
///     .into_iter()
///     .map(|(sample_count, sample_delta)| {
///         let mut run = TimeToSampleEntry::default();
///         (run.sample_count, run.sample_delta) = (sample_count, sample_delta);
///         run
///     })
///     .collect();
/// let mut stbl = SampleTableBox::default();
/// stbl.stts = Some(stts);
/// assert_eq!(media_duration(&stbl), 3_500);
/// ```
#[must_use]
pub fn media_duration(stbl: &SampleTableBox) -> u64 {
    stbl.stts.as_ref().map_or(0, |stts| {
        stts.entries.iter().fold(0_u64, |total, entry| {
            total.saturating_add(
                u64::from(entry.sample_count).saturating_mul(u64::from(entry.sample_delta)),
            )
        })
    })
}

/// Returns the latest time a sample of `stbl` is composed until: its decoding time, composition offset and duration added, in the timescale of the media, or 0 for no sample
///
/// The composition time of a sample is its decoding time, §8.6.1.2, plus
/// the offset the composition offset table gives it, §8.6.1.3; it lasts
/// its `sample_delta` from there. Without a composition offset table, the
/// samples are composed in decoding order and the end is the
/// [`media_duration`]. An end before 0 is 0; one past what a `u64` counts,
/// `u64::MAX`.
///
/// The specification takes the end of the sample with the largest
/// composition time, for the `duration` of an `mdhd`, §8.4.2.3, and the
/// `compositionEndTime` of a `cslg`, §8.6.1.4.3. Where samples last
/// differently, the end returned here can be later than that.
///
/// ```
/// use isobmff::boxes::{
///     CompositionOffsetBox, CompositionOffsetEntry, SampleTableBox, TimeToSampleBox,
///     TimeToSampleEntry,
/// };
/// use isobmff::structure_data::timeline::composition_end;
///
/// // 3 samples lasting 1 each, composed at 1, 3 and 2: the last ends at 4
/// let mut run = TimeToSampleEntry::default();
/// (run.sample_count, run.sample_delta) = (3, 1);
/// let mut stts = TimeToSampleBox::default();
/// stts.entries = vec![run];
/// let mut ctts = CompositionOffsetBox::default();
/// ctts.entries = [1, 2, 0]
///     .into_iter()
///     .map(|offset| {
///         let mut entry = CompositionOffsetEntry::default();
///         (entry.sample_count, entry.sample_offset) = (1, offset);
///         entry
///     })
///     .collect();
/// let mut stbl = SampleTableBox::default();
/// (stbl.stts, stbl.ctts) = (Some(stts), Some(ctts));
/// assert_eq!(composition_end(&stbl), 4);
/// ```
#[must_use]
pub fn composition_end(stbl: &SampleTableBox) -> u64 {
    let mut durations = SampleRuns::new(
        stbl.stts
            .as_ref()
            .map(|stts| stts.entries.as_slice())
            .unwrap_or_default(),
    );
    let mut offsets = SampleRuns::new(
        stbl.ctts
            .as_ref()
            .map(|ctts| ctts.entries.as_slice())
            .unwrap_or_default(),
    );
    let (mut decoding_time, mut end) = (0_i128, 0_i128);
    while let Some((mut left, sample_delta)) = durations.next_run(u32::MAX) {
        // Why not look at each sample: within samples of one duration and one
        // composition offset, each ends later than the one before, so the last
        // ends latest, and the walk is linear in the runs of both tables. The
        // inner loop ends: `next_run` takes at least 1 sample while `left > 0`.
        while left > 0 {
            let (taken, offset) = offsets.next_run(left).unwrap_or((left, 0));
            decoding_time = decoding_time
                .saturating_add(i128::from(sample_delta).saturating_mul(i128::from(taken)));
            end = end.max(decoding_time.saturating_add(i128::from(offset)));
            left = left.saturating_sub(taken);
        }
    }
    u64::try_from(end).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::collections::BTreeMap;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        CompositionOffsetBox, CompositionOffsetEntry, MovieBox, SampleDependencyTypeBox,
        SampleDependencyTypeEntry, SampleFlags, SampleSizeVariant, SampleSizes, SampleTableBox,
        SyncSampleBox, TimeToSampleBox, TimeToSampleEntry,
    };
    use isobmff_core::{BoxDecode, BoxDefinition, Boxes};
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use super::{
        Error, SampleTiming, check_sync_samples, composition_end, media_duration, sample_timings,
        table_sample_counts,
    };

    fn stts(runs: &[(u32, u32)]) -> TimeToSampleBox {
        let mut stts = TimeToSampleBox::default();
        stts.entries = runs
            .iter()
            .map(|&(sample_count, sample_delta)| {
                let mut entry = TimeToSampleEntry::default();
                (entry.sample_count, entry.sample_delta) = (sample_count, sample_delta);
                entry
            })
            .collect();
        stts
    }

    fn ctts(runs: &[(u32, i64)]) -> CompositionOffsetBox {
        let mut ctts = CompositionOffsetBox::default();
        ctts.entries = runs
            .iter()
            .map(|&(sample_count, sample_offset)| {
                let mut entry = CompositionOffsetEntry::default();
                (entry.sample_count, entry.sample_offset) = (sample_count, sample_offset);
                entry
            })
            .collect();
        ctts
    }

    fn stss(sample_numbers: &[u32]) -> SyncSampleBox {
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = sample_numbers.to_vec();
        stss
    }

    /// `sdtp` whose entries give each sample its `sample_depends_on`
    fn sdtp(sample_depends_on: &[u8]) -> SampleDependencyTypeBox {
        let mut sdtp = SampleDependencyTypeBox::default();
        sdtp.entries = sample_depends_on
            .iter()
            .map(|&depends_on| {
                let mut entry = SampleDependencyTypeEntry::default();
                entry.sample_depends_on = depends_on;
                entry
            })
            .collect();
        sdtp
    }

    /// The timing of a sample of a sync sample, with no composition offset and no dependency declared
    fn timing(decoding_time: u64, sample_duration: u32) -> SampleTiming {
        SampleTiming {
            decoding_time,
            sample_duration,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::default(),
        }
    }

    #[test]
    fn the_decoding_times_add_up_the_durations_of_the_runs_and_a_run_of_no_sample_gives_none() {
        let stts = stts(&[(2, 10), (0, 99), (1, 5)]);
        assert_eq!(
            sample_timings(&stts, None, None, None, 3).collect::<Vec<_>>(),
            [timing(0, 10), timing(10, 10), timing(20, 5)]
        );
    }

    #[test]
    fn the_samples_end_at_the_sample_count_or_where_the_time_to_sample_table_does() {
        let stts = stts(&[(3, 1)]);
        assert_eq!(sample_timings(&stts, None, None, None, 2).count(), 2);
        assert_eq!(sample_timings(&stts, None, None, None, 5).count(), 3);
        assert_eq!(sample_timings(&stts, None, None, None, 0).count(), 0);
    }

    /// `sample_flags` of a sample that is a sync sample or not, and depends on others as `sample_depends_on` says
    fn flags(is_non_sync_sample: bool, sample_depends_on: u8) -> SampleFlags {
        let mut flags = SampleFlags::default();
        (flags.sample_is_non_sync_sample, flags.sample_depends_on) =
            (is_non_sync_sample, sample_depends_on);
        flags
    }

    #[test]
    fn a_sample_past_the_composition_offset_table_has_an_offset_of_0() {
        let (stts, ctts) = (stts(&[(4, 1)]), ctts(&[(1, 2), (0, 7), (2, -1)]));
        let offset = |decoding_time, sample_composition_time_offset| SampleTiming {
            sample_composition_time_offset,
            ..timing(decoding_time, 1)
        };
        assert_eq!(
            sample_timings(&stts, Some(&ctts), None, None, 4).collect::<Vec<_>>(),
            [offset(0, 2), offset(1, -1), offset(2, -1), offset(3, 0)]
        );
    }

    #[test]
    fn the_sync_sample_table_names_the_sync_samples_and_an_entry_not_after_the_one_before_is_passed_over()
     {
        let stts = stts(&[(5, 1)]);
        let timings = |stss: Option<&SyncSampleBox>| {
            sample_timings(&stts, None, stss, None, 5).collect::<Vec<_>>()
        };
        let expected = |non_sync: [bool; 5]| {
            (0..5)
                .zip(non_sync)
                .map(|(decoding_time, is_non_sync_sample)| SampleTiming {
                    sample_flags: flags(is_non_sync_sample, 0),
                    ..timing(decoding_time, 1)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(timings(None), expected([false; 5]));
        assert_eq!(
            timings(Some(&stss(&[1, 4]))),
            expected([false, true, true, false, true])
        );
        assert_eq!(
            timings(Some(&stss(&[2, 2, 1, 3, 9]))),
            expected([true, false, false, true, true])
        );
    }

    #[test]
    fn the_sample_dependency_types_reach_their_samples_and_a_sample_past_them_declares_none() {
        let (stts, sdtp) = (stts(&[(3, 1)]), sdtp(&[2, 1]));
        let depending = |decoding_time, sample_depends_on| SampleTiming {
            sample_flags: flags(false, sample_depends_on),
            ..timing(decoding_time, 1)
        };
        assert_eq!(
            sample_timings(&stts, None, None, Some(&sdtp), 3).collect::<Vec<_>>(),
            [depending(0, 2), depending(1, 1), depending(2, 0)]
        );
    }

    /// Sample table of the runs of `stts` and, where given, of `ctts`
    fn stbl(stts_runs: &[(u32, u32)], ctts_runs: Option<&[(u32, i64)]>) -> SampleTableBox {
        let mut stbl = SampleTableBox::default();
        (stbl.stts, stbl.ctts) = (Some(stts(stts_runs)), ctts_runs.map(ctts));
        stbl
    }

    #[test]
    fn a_sample_table_without_a_time_to_sample_table_composes_nothing() {
        assert_eq!(composition_end(&SampleTableBox::default()), 0);
    }

    #[test]
    fn a_composition_that_ends_before_0_ends_at_0() {
        assert_eq!(composition_end(&stbl(&[(2, 1)], Some(&[(2, -10)]))), 0);
    }

    #[test]
    fn an_end_past_what_a_u64_counts_is_u64_max_even_after_a_negative_offset() {
        let runs = [(u32::MAX, u32::MAX), (u32::MAX, u32::MAX)];
        assert_eq!(composition_end(&stbl(&runs, None)), u64::MAX);
        let offsets = [(u32::MAX, i64::MIN), (u32::MAX, i64::MIN)];
        assert_eq!(composition_end(&stbl(&runs, Some(&offsets))), u64::MAX);
    }

    #[test]
    fn samples_past_a_short_composition_offset_table_end_at_their_decoding_end() {
        assert_eq!(composition_end(&stbl(&[(3, 10)], Some(&[(1, 25)]))), 35);
    }

    #[test]
    fn runs_of_no_sample_are_passed_over() {
        let stbl = stbl(
            &[(1, 10), (0, 99), (1, 10)],
            Some(&[(0, 50), (1, 5), (0, 7), (1, 0)]),
        );
        assert_eq!(composition_end(&stbl), 20);
    }

    #[test]
    fn runs_that_do_not_line_up_are_cut_where_either_ends() {
        let stbl = stbl(&[(3, 10), (2, 1)], Some(&[(2, 0), (2, 30), (1, 0)]));
        assert_eq!(composition_end(&stbl), 61);
    }

    #[test]
    fn without_a_composition_offset_table_the_end_is_the_media_duration() {
        let stbl = stbl(&[(3, 10), (2, 1)], None);
        assert_eq!((composition_end(&stbl), media_duration(&stbl)), (32, 32));
    }

    #[test]
    fn a_sample_that_lasts_past_the_one_composed_last_ends_the_composition() {
        assert_eq!(
            composition_end(&stbl(&[(1, 5), (1, 1)], Some(&[(1, 6), (1, 3)]))),
            11
        );
    }

    #[test]
    fn each_table_present_declares_the_samples_of_its_runs_or_entries() {
        let (stts, ctts, sdtp) = (stts(&[(2, 1), (3, 1)]), ctts(&[(4, 0)]), sdtp(&[0; 6]));
        assert_eq!(
            table_sample_counts(&stts, Some(&ctts), Some(&sdtp)).collect::<Vec<_>>(),
            [
                (TimeToSampleBox::BOX_TYPE, 5),
                (CompositionOffsetBox::BOX_TYPE, 4),
                (SampleDependencyTypeBox::BOX_TYPE, 6)
            ]
        );
        assert_eq!(
            table_sample_counts(&stts, None, None).collect::<Vec<_>>(),
            [(TimeToSampleBox::BOX_TYPE, 5)]
        );
    }

    #[test]
    fn a_sync_sample_number_that_names_no_sample_or_is_not_after_the_one_before_is_refused() {
        let out_of_range = |entry, sample_number| {
            Err(Error::SyncSampleNumberOutOfRange {
                track_id: 7,
                entry,
                sample_number,
                sample_count: 5,
            })
        };
        let not_increasing = |entry, sample_number, previous_sample_number| {
            Err(Error::SyncSampleNumberNotIncreasing {
                track_id: 7,
                entry,
                sample_number,
                previous_sample_number,
            })
        };
        assert_eq!(check_sync_samples(&stss(&[1, 3, 5]), 5, 7), Ok(()));
        assert_eq!(check_sync_samples(&stss(&[0]), 5, 7), out_of_range(0, 0));
        assert_eq!(check_sync_samples(&stss(&[3, 9]), 5, 7), out_of_range(1, 9));
        assert_eq!(
            check_sync_samples(&stss(&[3, 2]), 5, 7),
            not_increasing(1, 2, 3)
        );
        assert_eq!(
            check_sync_samples(&stss(&[3, 3]), 5, 7),
            not_increasing(1, 3, 3)
        );
    }

    #[test]
    fn a_sync_sample_number_out_of_range_and_not_after_the_one_before_is_reported_out_of_range() {
        assert_eq!(
            check_sync_samples(&stss(&[5, 0]), 5, 7),
            Err(Error::SyncSampleNumberOutOfRange {
                track_id: 7,
                entry: 1,
                sample_number: 0,
                sample_count: 5,
            })
        );
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe"]
    fn the_samples_of_each_track_are_composed_when_ffprobe_presents_them_and_decoded_at_its_times()
    {
        let (mut tracks, mut samples, mut differences) = (0, 0, Vec::<String>::new());
        for file in conformance_files() {
            let Some(moov) = Boxes::new(&file.bytes)
                .map_while(Result::ok)
                .find(|raw_box| raw_box.header().box_type() == MovieBox::BOX_TYPE)
                .and_then(|moov| MovieBox::decode_payload(moov.payload()).ok())
            else {
                continue;
            };
            let (name, path) = (&file.name, &file.path);
            if !ffprobe(&["-v", "error"], path).status.success() {
                continue;
            }
            let mut packets = BTreeMap::<u32, Vec<(i64, i64)>>::new();
            for packet in ffprobe_packets(&["-ignore_editlist", "1"], path) {
                if let (Some(dts), Some(pts)) = (packet.dts, packet.pts) {
                    packets.entry(packet.track_id).or_default().push((dts, pts));
                }
            }
            for trak in &moov.trak {
                let (track_id, stbl) = (trak.tkhd.track_id, &trak.mdia.minf.stbl);
                let sample_count = match &stbl.sample_size {
                    Some(SampleSizeVariant::SampleSize(stsz)) => match &stsz.sample_sizes {
                        SampleSizes::Constant { sample_count, .. } => *sample_count,
                        SampleSizes::Table { entry_sizes } => {
                            u32::try_from(entry_sizes.len()).unwrap()
                        }
                        _ => 0,
                    },
                    Some(SampleSizeVariant::CompactSampleSize(stz2)) => {
                        u32::try_from(stz2.entry_sizes.len()).unwrap()
                    }
                    Some(_) | None => 0,
                };
                let Some(stts) = &stbl.stts else { continue };
                let (ctts, stss, sdtp) =
                    (stbl.ctts.as_ref(), stbl.stss.as_ref(), stbl.sdtp.as_ref());
                // Why not compare a track whose tables disagree: the sample
                // table refuses it as the specification reads it.
                let agrees = table_sample_counts(stts, ctts, sdtp)
                    .all(|(_, count)| count == u64::from(sample_count))
                    && stss.is_none_or(|stss| {
                        check_sync_samples(stss, sample_count, track_id).is_ok()
                    });
                let theirs = packets
                    .get(&track_id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if !agrees || sample_count == 0 || theirs.is_empty() {
                    continue;
                }
                let ours = sample_timings(stts, ctts, stss, sdtp, sample_count)
                    .map(|timing| {
                        (
                            i128::from(timing.decoding_time),
                            i128::from(timing.decoding_time)
                                + i128::from(timing.sample_composition_time_offset),
                        )
                    })
                    .collect::<Vec<_>>();
                // Why compare the samples both have: ffprobe reads the samples
                // of the movie fragments after those of the `moov`, and fewer
                // where it cannot place their bytes, which is no matter of
                // their timing.
                let compared = ours.len().min(theirs.len());
                let shift = ours.first().unwrap().0 - i128::from(theirs.first().unwrap().0);
                let differing =
                    ours.iter()
                        .zip(theirs)
                        .position(|(&(decoding, composition), &(dts, pts))| {
                            decoding - i128::from(dts) != shift || composition != i128::from(pts)
                        });
                if let Some(sample) = differing {
                    differences.push(format!(
                        "{name} track {track_id}: sample {sample} differs, {:?} against ffprobe {:?}",
                        ours.get(sample),
                        theirs.get(sample)
                    ));
                }
                tracks += 1;
                samples += compared;
            }
        }
        std::println!("compared {tracks} tracks, {samples} samples");
        assert_eq!(differences, Vec::<String>::new());
    }
}
