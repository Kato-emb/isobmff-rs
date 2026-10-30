//! The movie fragment, ISO/IEC 14496-12 §8.8: the samples its track fragments declare, read off their track runs and the defaults they take, and the movie fragment written of runs of samples

use alloc::vec::Vec;
use core::iter;

use isobmff_boxes::{
    MovieBox, MovieFragmentBox, SampleFlags, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
    TrackRunBox, TrackRunSample, TrackRunSamples,
};
use isobmff_core::BoxEncode;

use super::sample_description::data_reference_index;
use super::sample_table::expand;
use crate::error::{DemuxError, MuxError};
use crate::framing::to_u64;
use crate::sample::{Sample, SampleLocation, SampleProperties};

/// Returns the number of samples the track runs of `moof` count, those of track fragments of empty duration left out
pub(super) fn sample_count(moof: &MovieFragmentBox) -> u64 {
    moof.traf
        .iter()
        .filter(|traf| !traf.tfhd.duration_is_empty)
        .flat_map(|traf| &traf.trun)
        .map(|trun| u64::from(run_sample_count(trun)))
        .fold(0, u64::saturating_add)
}

/// Resolves the samples the track fragments of `moof`, which starts at file offset `moof_offset`, declare into `locations`, in the order of the track fragments and their runs
///
/// Each sample takes a field its row does not carry from the `tfhd` of its
/// track fragment, or else from the `trex` of its track (§8.8.3, §8.8.7).
/// The data offset of a run counts from the base data offset of its track
/// fragment, which Table 6 of §8.8.7.1 resolves to a file offset: the
/// `base_data_offset` of the `tfhd`, else the first byte of the `moof` where
/// the `tfhd` says so or the track fragment is the first of the `moof`, else
/// the end of the data of the track fragment before it. A run without a data
/// offset starts where the run before it ends.
///
/// The decoding time of the first sample of a track fragment is the one its
/// `tfdt` gives, else the one its track has reached in `decoding_times`
/// (§8.8.12), which the samples move on. A track fragment of empty duration
/// declares no sample and moves the decoding time on by the default duration.
///
/// # Errors
///
/// * [`UnknownTrackId`](DemuxError::UnknownTrackId),
///   [`MissingTrackExtends`](DemuxError::MissingTrackExtends): a track
///   fragment is of a track the movie, or its `mvex`, has none of.
/// * [`MissingDecodingTime`](DemuxError::MissingDecodingTime): a track
///   fragment has no `tfdt`, and the decoding time its track has reached is
///   not known.
/// * The failures of the sample entry and the data reference entry the
///   samples are described by.
/// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange),
///   [`DecodingTimeOverflow`](DemuxError::DecodingTimeOverflow): the media
///   data or the decoding times run outside 64 bits.
pub(super) fn resolve_fragment(
    movie: &MovieBox,
    decoding_times: &mut [Option<u64>],
    moof: &MovieFragmentBox,
    moof_offset: u64,
    locations: &mut Vec<SampleLocation>,
) -> Result<(), DemuxError> {
    let mut previous_end = moof_offset;
    for traf in &moof.traf {
        let tfhd = &traf.tfhd;
        let track_id = tfhd.track_id;
        let (trak, decoding_time) = movie
            .trak
            .iter()
            .zip(decoding_times.iter_mut())
            .find(|(trak, _)| trak.tkhd.track_id == track_id)
            .ok_or(DemuxError::UnknownTrackId { track_id })?;
        let trex = movie
            .mvex
            .iter()
            .flat_map(|mvex| &mvex.trex)
            .find(|trex| trex.track_id == track_id)
            .ok_or(DemuxError::MissingTrackExtends { track_id })?;
        let mut time = traf
            .tfdt
            .as_ref()
            .map(|tfdt| tfdt.base_media_decode_time)
            .or(*decoding_time)
            .ok_or(DemuxError::MissingDecodingTime { track_id })?;
        let defaults = Defaults {
            sample_description_index: tfhd
                .sample_description_index
                .unwrap_or(trex.default_sample_description_index),
            sample_duration: tfhd
                .default_sample_duration
                .unwrap_or(trex.default_sample_duration),
            sample_size: tfhd.default_sample_size.unwrap_or(trex.default_sample_size),
            sample_flags: tfhd
                .default_sample_flags
                .unwrap_or(trex.default_sample_flags),
        };

        let base = tfhd
            .base_data_offset
            .unwrap_or(if tfhd.default_base_is_moof {
                moof_offset
            } else {
                previous_end
            });
        let mut data_offset = base;
        if tfhd.duration_is_empty {
            time = time
                .checked_add(u64::from(defaults.sample_duration))
                .ok_or(DemuxError::DecodingTimeOverflow { track_id })?;
        } else if traf.trun.iter().any(|trun| run_sample_count(trun) > 0) {
            let data_reference_index =
                data_reference_index(trak, defaults.sample_description_index)?;
            for trun in &traf.trun {
                if let Some(offset) = trun.data_offset {
                    data_offset = base
                        .checked_add_signed(i64::from(offset))
                        .ok_or(DemuxError::DataOffsetOutOfRange { track_id })?;
                }
                for (row_number, row) in rows(trun).enumerate() {
                    let first_sample_flags = trun.first_sample_flags.filter(|_| row_number == 0);
                    let sample_duration = row.sample_duration.unwrap_or(defaults.sample_duration);
                    let sample_size = row.sample_size.unwrap_or(defaults.sample_size);
                    let data_end = data_offset
                        .checked_add(u64::from(sample_size))
                        .ok_or(DemuxError::DataOffsetOutOfRange { track_id })?;
                    locations.push(SampleLocation::new(
                        SampleProperties {
                            track_id,
                            decoding_time: time,
                            sample_duration,
                            sample_composition_time_offset: row
                                .sample_composition_time_offset
                                .unwrap_or_default(),
                            sample_flags: first_sample_flags
                                .or(row.sample_flags)
                                .unwrap_or(defaults.sample_flags),
                            sample_description_index: defaults.sample_description_index,
                        },
                        data_reference_index,
                        data_offset..data_end,
                    ));
                    time = time
                        .checked_add(u64::from(sample_duration))
                        .ok_or(DemuxError::DecodingTimeOverflow { track_id })?;
                    data_offset = data_end;
                }
            }
        }

        *decoding_time = Some(time);
        previous_end = data_offset;
    }

    Ok(())
}

/// Values the samples of a track fragment take where their rows do not carry them: those of the `tfhd`, else those of the `trex`
struct Defaults {
    sample_description_index: u32,
    sample_duration: u32,
    sample_size: u32,
    sample_flags: SampleFlags,
}

/// Returns the number of samples `trun` adds
fn run_sample_count(trun: &TrackRunBox) -> u32 {
    match &trun.samples {
        TrackRunSamples::EmptyRows { sample_count } => *sample_count,
        TrackRunSamples::Rows { samples } => u32::try_from(samples.len()).unwrap_or(u32::MAX),
        _ => 0,
    }
}

/// Returns the row of each sample `trun` adds, an empty row for each sample of a run of empty rows
fn rows(trun: &TrackRunBox) -> impl Iterator<Item = TrackRunSample> {
    let (empty_rows, rows): (u32, &[TrackRunSample]) = match &trun.samples {
        TrackRunSamples::EmptyRows { sample_count } => (*sample_count, &[]),
        TrackRunSamples::Rows { samples } => (0, samples),
        _ => (0, &[]),
    };

    expand(iter::once((empty_rows, TrackRunSample::default()))).chain(rows.iter().copied())
}

/// Samples of one track that lie one after another, the first of them at `data_offset` from the media data of the samples written together
pub(super) struct Run<'a> {
    /// Index of the track among the tracks of the movie
    pub(super) track: usize,
    pub(super) track_id: u32,
    pub(super) sample_description_index: u32,
    /// Decoding time of the first sample
    pub(super) decoding_time: u64,
    /// Decoding time past the last sample
    pub(super) decoding_end: u64,
    pub(super) samples: &'a [Sample],
    pub(super) data_offset: u64,
}

/// Returns the runs of samples of one track that lie one after another in `samples`, checked against the tracks of `movie`
///
/// The decoding times of the samples of each track follow from the
/// durations before them in `samples`, from the first sample's own where
/// `decoding_times` is `None`, and otherwise from the decoding time
/// `decoding_times` holds for the track, in the order of the tracks of the
/// movie.
///
/// # Errors
///
/// * The failures of the samples as for [`SampleTableWriter::handle_samples`](super::writer::SampleTableWriter::handle_samples).
pub(super) fn runs<'a>(
    movie: &MovieBox,
    samples: &'a [Sample],
    decoding_times: Option<&[u64]>,
) -> Result<Vec<Run<'a>>, MuxError> {
    let mut runs: Vec<Run<'a>> = Vec::new();
    let mut rest = samples;
    let mut data_offset = 0_u64;
    while let Some(first) = rest.first() {
        let first = first.properties();
        let track_id = first.track_id;
        let (track, trak) = movie
            .trak
            .iter()
            .enumerate()
            .find(|(_, trak)| trak.tkhd.track_id == track_id)
            .ok_or(MuxError::UnknownTrackId { track_id })?;
        let len = rest
            .iter()
            .take_while(|sample| sample.properties().track_id == track_id)
            .count();
        let (run_samples, after) = rest.split_at_checked(len).unwrap_or((rest, &[]));
        let earlier = runs.iter().rev().find(|run| run.track == track);
        let sample_description_index = match earlier {
            Some(earlier) => earlier.sample_description_index,
            None => {
                data_reference_index(trak, first.sample_description_index)?;
                first.sample_description_index
            }
        };
        let decoding_start = earlier
            .map(|earlier| earlier.decoding_end)
            .or_else(|| decoding_times.map(|times| times.get(track).copied().unwrap_or_default()));
        let mut decoding_time = decoding_start.unwrap_or(first.decoding_time);
        let mut run_len = 0_u64;
        for sample in run_samples {
            let properties = sample.properties();
            if properties.sample_description_index != sample_description_index {
                return Err(MuxError::MixedSampleDescriptions { track_id });
            }
            let mismatch = MuxError::DecodingTimeMismatch {
                track_id,
                expected_decoding_time: decoding_time,
                decoding_time: properties.decoding_time,
            };
            if properties.decoding_time != decoding_time {
                return Err(mismatch);
            }
            decoding_time = decoding_time
                .checked_add(u64::from(properties.sample_duration))
                .ok_or(mismatch)?;
            let size = sample.data().len();
            if u32::try_from(size).is_err() {
                return Err(MuxError::SampleSizeOutOfRange {
                    track_id,
                    sample_bytes: to_u64(size),
                });
            }
            run_len = run_len.saturating_add(to_u64(size));
        }
        runs.push(Run {
            track,
            track_id,
            sample_description_index,
            decoding_time: first.decoding_time,
            decoding_end: decoding_time,
            samples: run_samples,
            data_offset,
        });
        data_offset = data_offset.saturating_add(run_len);
        rest = after;
    }

    Ok(runs)
}

/// Returns the movie fragment numbered `sequence_number` of the samples `runs` of the tracks of `movie`, their media data from `data_start` bytes past the end of the `moof`
///
/// Each track has one track fragment, its base data offset the first byte
/// of the `moof` (§8.8.7.1) and its `tfdt` the decoding time of its first
/// sample (§8.8.12), and a track run for each of its runs (§8.8.8). The data
/// offsets count from the `moof`, whose length is taken once.
///
/// # Errors
///
/// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
///   data of a sample lies further from the `moof` than a `trun` reaches.
pub(super) fn movie_fragment_box(
    movie: &MovieBox,
    sequence_number: u32,
    runs: &[Run<'_>],
    data_start: u64,
) -> Result<MovieFragmentBox, MuxError> {
    let mut tracks: Vec<Vec<&Run<'_>>> = Vec::new();
    for run in runs {
        match tracks.iter_mut().find(|track_runs| {
            track_runs
                .first()
                .is_some_and(|first| first.track == run.track)
        }) {
            Some(track_runs) => track_runs.push(run),
            None => tracks.push(Vec::from([run])),
        }
    }
    let mut moof = MovieFragmentBox::default();
    moof.mfhd.sequence_number = sequence_number;
    for track_runs in &tracks {
        let Some(first) = track_runs.first() else {
            continue;
        };
        let mut traf = TrackFragmentBox::default();
        traf.tfhd.track_id = first.track_id;
        traf.tfhd.default_base_is_moof = true;
        traf.tfhd.sample_description_index =
            Some(first.sample_description_index).filter(|&index| {
                movie
                    .mvex
                    .iter()
                    .flat_map(|mvex| &mvex.trex)
                    .find(|trex| trex.track_id == first.track_id)
                    .is_none_or(|trex| trex.default_sample_description_index != index)
            });
        let mut tfdt = TrackFragmentBaseMediaDecodeTimeBox::default();
        tfdt.version = u8::from(u32::try_from(first.decoding_time).is_err());
        tfdt.base_media_decode_time = first.decoding_time;
        traf.tfdt = Some(tfdt);
        traf.trun = track_runs
            .iter()
            .map(|run| track_run(run.samples))
            .collect();
        moof.traf.push(traf);
    }
    let moof_len = moof.encoded_len();
    for (traf, track_runs) in moof.traf.iter_mut().zip(&tracks) {
        for (trun, run) in traf.trun.iter_mut().zip(track_runs) {
            trun.data_offset = Some(
                moof_len
                    .checked_add(data_start)
                    .and_then(|offset| offset.checked_add(run.data_offset))
                    .and_then(|offset| i32::try_from(offset).ok())
                    .ok_or(MuxError::DataOffsetOutOfRange {
                        track_id: run.track_id,
                    })?,
            );
        }
    }

    Ok(moof)
}

/// Returns the track run of `samples`, each row carrying the fields of its sample, and the composition time offsets only where one is not zero
fn track_run(samples: &[Sample]) -> TrackRunBox {
    let offsets = samples
        .iter()
        .any(|sample| sample.properties().sample_composition_time_offset != 0);
    let mut trun = TrackRunBox::default();
    trun.version = u8::from(
        samples
            .iter()
            .any(|sample| sample.properties().sample_composition_time_offset < 0),
    );
    trun.data_offset = Some(0);
    trun.samples = TrackRunSamples::Rows {
        samples: samples
            .iter()
            .map(|sample| {
                let properties = sample.properties();
                let mut row = TrackRunSample::default();
                row.sample_duration = Some(properties.sample_duration);
                row.sample_size = u32::try_from(sample.data().len()).ok();
                row.sample_flags = Some(properties.sample_flags);
                row.sample_composition_time_offset =
                    offsets.then_some(properties.sample_composition_time_offset);
                row
            })
            .collect(),
    };
    trun
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::{
        MovieBox, MovieExtendsBox, MovieFragmentBox, SampleFlags, TrackExtendsBox,
        TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackRunBox, TrackRunSample,
        TrackRunSamples,
    };

    use crate::error::DemuxError;
    use crate::sample::{SampleLocation, SampleProperties};
    use crate::structure_data::fixtures::{UNLIMITED, movie, track};
    use crate::structure_data::{StructureDataLimits, StructureDataReader};

    /// File offset the fragments of the tests start at
    const MOOF_OFFSET: u64 = 1000;

    /// Returns a movie of tracks 1 and 2, each of the three samples of 10 ticks of the `moov`, and a `trex` of samples of 100 ticks and 8 bytes
    fn fragmented_movie() -> MovieBox {
        let mut movie = movie([
            track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]),
            track(2, 10, &[1, 1, 1], &[(1, 3)], &[3]),
        ]);
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = [1, 2]
            .map(|track_id| {
                let mut trex = TrackExtendsBox::default();
                trex.track_id = track_id;
                trex.default_sample_description_index = 1;
                trex.default_sample_duration = 100;
                trex.default_sample_size = 8;
                trex.default_sample_flags = SampleFlags::from_bits(0x0001_0000);
                trex
            })
            .to_vec();
        movie.mvex = Some(mvex);
        movie
    }

    /// Returns the track fragment of track `track_id` of runs of `sample_count` samples of the defaults, each at `data_offset`
    fn traf(track_id: u32, runs: &[(u32, Option<i32>)]) -> TrackFragmentBox {
        let mut traf = TrackFragmentBox::default();
        traf.tfhd.track_id = track_id;
        traf.trun = runs
            .iter()
            .map(|&(sample_count, data_offset)| {
                let mut trun = TrackRunBox::default();
                trun.data_offset = data_offset;
                trun.samples = TrackRunSamples::EmptyRows { sample_count };
                trun
            })
            .collect();
        traf
    }

    /// Returns the movie fragment of `trafs`
    fn moof(trafs: impl IntoIterator<Item = TrackFragmentBox>) -> MovieFragmentBox {
        let mut moof = MovieFragmentBox::default();
        moof.traf = trafs.into_iter().collect();
        moof
    }

    /// Returns a reader that holds [`fragmented_movie`]
    fn reader() -> StructureDataReader {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader.handle_movie(fragmented_movie()).unwrap();
        reader
    }

    /// Resolves `moof` at [`MOOF_OFFSET`], returning the track, decoding time and byte range of each sample
    fn resolve(
        reader: &mut StructureDataReader,
        moof: &MovieFragmentBox,
    ) -> Result<Vec<(u32, u64, Range<u64>)>, DemuxError> {
        Ok(reader
            .handle_movie_fragment(moof, MOOF_OFFSET)?
            .map(|location| {
                (
                    location.properties().track_id,
                    location.properties().decoding_time,
                    location.byte_range(),
                )
            })
            .collect())
    }

    #[test]
    fn the_first_track_fragment_counts_from_the_moof_and_the_next_from_the_end_of_its_data() {
        let moof = moof([traf(1, &[(2, Some(100))]), traf(2, &[(1, None)])]);

        assert_eq!(
            resolve(&mut reader(), &moof).unwrap(),
            [
                (1, 30, 1100..1108),
                (1, 130, 1108..1116),
                (2, 30, 1116..1124)
            ]
        );
    }

    #[test]
    fn a_base_data_offset_counts_from_the_start_of_the_file_and_default_base_is_moof_from_the_moof()
    {
        let mut from_file = traf(1, &[(1, Some(-16))]);
        from_file.tfhd.base_data_offset = Some(5000);
        let mut from_moof = traf(2, &[(1, Some(40))]);
        from_moof.tfhd.default_base_is_moof = true;
        let moof = moof([from_file, from_moof]);

        assert_eq!(
            resolve(&mut reader(), &moof).unwrap(),
            [(2, 30, 1040..1048), (1, 30, 4984..4992)]
        );
    }

    #[test]
    fn a_run_without_a_data_offset_starts_where_the_run_before_it_ends() {
        let moof = moof([traf(1, &[(1, Some(200)), (1, None), (1, Some(100))])]);

        assert_eq!(
            resolve(&mut reader(), &moof).unwrap(),
            [
                (1, 230, 1100..1108),
                (1, 30, 1200..1208),
                (1, 130, 1208..1216)
            ]
        );
    }

    #[test]
    fn the_decoding_time_runs_on_from_the_movie_through_the_fragments_unless_a_tfdt_gives_it() {
        let mut reader = reader();
        let first = moof([traf(1, &[(2, None)])]);
        let mut with_tfdt = traf(1, &[(1, None)]);
        let mut tfdt = TrackFragmentBaseMediaDecodeTimeBox::default();
        tfdt.base_media_decode_time = 900;
        with_tfdt.tfdt = Some(tfdt);
        let second = moof([with_tfdt]);
        let third = moof([traf(1, &[(1, None)])]);

        assert_eq!(
            resolve(&mut reader, &first).unwrap(),
            [(1, 30, 1000..1008), (1, 130, 1008..1016)]
        );
        assert_eq!(
            resolve(&mut reader, &second).unwrap(),
            [(1, 900, 1000..1008)]
        );
        assert_eq!(
            resolve(&mut reader, &third).unwrap(),
            [(1, 1000, 1000..1008)]
        );
    }

    #[test]
    fn after_a_resume_only_a_track_fragment_with_a_tfdt_resolves() {
        let mut reader = reader();
        reader.resume();
        let mut with_tfdt = traf(2, &[(1, None)]);
        with_tfdt.tfdt = Some(TrackFragmentBaseMediaDecodeTimeBox::default());

        assert_eq!(
            resolve(&mut reader, &moof([traf(1, &[(1, None)])])),
            Err(DemuxError::MissingDecodingTime { track_id: 1 })
        );
        assert_eq!(
            resolve(&mut reader, &moof([with_tfdt])).unwrap(),
            [(2, 0, 1000..1008)]
        );
        assert_eq!(
            resolve(&mut reader, &moof([traf(2, &[(1, None)])])).unwrap(),
            [(2, 100, 1000..1008)]
        );
    }

    #[test]
    fn a_track_fragment_of_empty_duration_declares_no_sample_and_moves_the_decoding_time_on() {
        let mut reader = reader();
        let mut empty = traf(1, &[]);
        empty.tfhd.duration_is_empty = true;
        empty.tfhd.default_sample_duration = Some(70);

        assert_eq!(resolve(&mut reader, &moof([empty])).unwrap(), []);
        assert_eq!(
            resolve(&mut reader, &moof([traf(1, &[(1, None)])])).unwrap(),
            [(1, 100, 1000..1008)]
        );
    }

    #[test]
    fn a_row_overrides_the_tfhd_which_overrides_the_trex() {
        let mut traf = traf(1, &[]);
        traf.tfhd.default_sample_duration = Some(50);
        traf.tfhd.default_sample_flags = Some(SampleFlags::from_bits(0x0201_0000));
        let mut trun = TrackRunBox::default();
        trun.first_sample_flags = Some(SampleFlags::from_bits(0x0200_0000));
        let mut row = TrackRunSample::default();
        row.sample_size = Some(3);
        row.sample_composition_time_offset = Some(-20);
        trun.samples = TrackRunSamples::Rows {
            samples: Vec::from([row, row]),
        };
        traf.trun = Vec::from([trun]);

        let sample = |decoding_time, byte_range, sample_flags| {
            SampleLocation::new(
                SampleProperties {
                    track_id: 1,
                    decoding_time,
                    sample_duration: 50,
                    sample_composition_time_offset: -20,
                    sample_flags: SampleFlags::from_bits(sample_flags),
                    sample_description_index: 1,
                },
                1,
                byte_range,
            )
        };

        assert_eq!(
            reader()
                .handle_movie_fragment(&moof([traf]), MOOF_OFFSET)
                .unwrap()
                .collect::<Vec<_>>(),
            [
                sample(30, 1000..1003, 0x0200_0000),
                sample(80, 1003..1006, 0x0201_0000)
            ]
        );
    }

    #[test]
    fn a_fragment_without_its_movie_or_of_a_track_it_does_not_extend_is_refused() {
        let mut without_trex = fragmented_movie();
        without_trex.mvex.as_mut().unwrap().trex.truncate(1);
        let mut reader_without_trex = StructureDataReader::new(UNLIMITED);
        reader_without_trex.handle_movie(without_trex).unwrap();

        assert_eq!(
            resolve(
                &mut StructureDataReader::new(UNLIMITED),
                &moof([traf(1, &[(1, None)])])
            ),
            Err(DemuxError::MissingMovie)
        );
        assert_eq!(
            resolve(&mut reader(), &moof([traf(9, &[(1, None)])])),
            Err(DemuxError::UnknownTrackId { track_id: 9 })
        );
        assert_eq!(
            resolve(&mut reader_without_trex, &moof([traf(2, &[(1, None)])])),
            Err(DemuxError::MissingTrackExtends { track_id: 2 })
        );
    }

    #[test]
    fn a_refused_fragment_leaves_the_decoding_times_as_they_were() {
        let mut reader = StructureDataReader::new(StructureDataLimits {
            sample_count: 6,
            ..UNLIMITED
        });
        reader.handle_movie(fragmented_movie()).unwrap();

        assert_eq!(
            resolve(&mut reader, &moof([traf(1, &[(1, None)]), traf(9, &[])])),
            Err(DemuxError::UnknownTrackId { track_id: 9 })
        );
        assert_eq!(
            resolve(&mut reader, &moof([traf(1, &[(7, None)])])),
            Err(DemuxError::SampleCountLimitExceeded {
                declared_samples: 7,
                limit_samples: 6
            })
        );
        assert_eq!(
            resolve(&mut reader, &moof([traf(1, &[(1, None)])])).unwrap(),
            [(1, 30, 1000..1008)]
        );
    }
}
