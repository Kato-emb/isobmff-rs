//! The samples the track fragments of a movie fragment declare, read off their track runs and the defaults they take, ISO/IEC 14496-12 §8.8

use alloc::vec::Vec;
use core::iter;

use isobmff_boxes::{
    MovieBox, MovieFragmentBox, SampleFlags, TrackRunBox, TrackRunSample, TrackRunSamples,
};

use super::{data_reference_index, expand};
use crate::error::DemuxError;
use crate::sample::{SampleLocation, SampleProperties};

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
