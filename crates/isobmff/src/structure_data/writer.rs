//! [`StructureDataWriter`], which writes the structure-data that declares samples: the sample tables of a movie, or its movie fragments

use alloc::vec::Vec;

use isobmff_boxes::{
    ChunkOffsetBox, ChunkOffsetVariant, CompactSampleToGroupBox, CompositionOffsetBox,
    DegradationPriorityBox, MovieBox, MovieExtendsBox, MovieFragmentBox, PaddingBitsBox,
    SampleAuxiliaryInformationOffsetsBox, SampleAuxiliaryInformationSizesBox,
    SampleDependencyTypeBox, SampleInfoSizes, SampleSizeBox, SampleToChunkBox, SampleToGroupBox,
    ShadowSyncSampleBox, SubSampleInformationBox, SubSampleReferenceTableBox, SyncSampleBox,
    TimeToSampleBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackRunBox,
    TrackRunSample, TrackRunSamples,
};
use isobmff_core::{BoxDefinition, BoxEncode};

mod sample_table;

use sample_table::TrackTable;

use super::data_reference_index;
use crate::error::MuxError;
use crate::framing::to_u64;
use crate::sample::Sample;

/// Writer of the structure-data of a file: a movie fragment for each run of samples, or the sample tables of its movie
///
/// Each writer holds the movie it was made from, and refuses samples the
/// movie does not declare a track, a sample entry or a data reference entry
/// in the file itself for, as a reader of the file would.
#[derive(Debug)]
pub(crate) enum StructureDataWriter {
    /// Movie fragments, §8.8
    Fragmented(MovieFragmentWriter),
    /// The sample tables of the movie, §8.5
    NonFragmented(SampleTableWriter),
}

/// Writer of the movie fragments of a movie, numbered on from `sequence_number`
#[derive(Debug)]
pub(crate) struct MovieFragmentWriter {
    movie: MovieBox,
    sequence_number: u32,
}

/// Writer of the sample table of each track of a movie, in the order of its tracks
#[derive(Debug)]
pub(crate) struct SampleTableWriter {
    movie: MovieBox,
    tables: Vec<TrackTable>,
}

/// Samples of one track that lie one after another, the first of them at `data_offset` from the media data of the samples written together
struct Run<'a> {
    /// Index of the track among the tracks of the movie
    track: usize,
    track_id: u32,
    sample_description_index: u32,
    /// Decoding time of the first sample
    decoding_time: u64,
    /// Decoding time past the last sample
    decoding_end: u64,
    samples: &'a [Sample],
    data_offset: u64,
}

impl MovieFragmentWriter {
    /// Creates the writer of the movie fragments of `movie`, the first numbered `first_sequence_number`
    ///
    /// # Errors
    ///
    /// * [`MissingTrackExtends`](MuxError::MissingTrackExtends): a track of
    ///   the movie has no `trex`, which its fragments take defaults from.
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a table of a
    ///   track declares samples, which belong in the fragments.
    pub(crate) fn new(movie: MovieBox, first_sequence_number: u32) -> Result<Self, MuxError> {
        refuse_samples_in_tables(&movie)?;
        for trak in &movie.trak {
            let track_id = trak.tkhd.track_id;
            if !movie
                .mvex
                .iter()
                .flat_map(|mvex| &mvex.trex)
                .any(|trex| trex.track_id == track_id)
            {
                return Err(MuxError::MissingTrackExtends { track_id });
            }
        }

        Ok(Self {
            movie,
            sequence_number: first_sequence_number,
        })
    }

    /// Returns the movie the writer was made from
    pub(crate) const fn movie(&self) -> &MovieBox {
        &self.movie
    }

    /// Returns the movie fragment of `samples`, whose media data lies in their order from `data_start` bytes past the end of the `moof`
    ///
    /// Each track has one track fragment, its base data offset the first byte
    /// of the `moof` (§8.8.7.1) and its `tfdt` the decoding time of its first
    /// sample (§8.8.12), and a track run for each run of its samples that lie
    /// one after another (§8.8.8). The next fragment takes the next sequence
    /// number.
    ///
    /// # Errors
    ///
    /// * The failures of the samples as for
    ///   [`SampleTableWriter::handle_samples`], but for their decoding times,
    ///   which follow from the durations within the fragment only.
    /// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of a sample lies further from the `moof` than a `trun` reaches.
    pub(crate) fn movie_fragment(
        &mut self,
        samples: &[Sample],
        data_start: u64,
    ) -> Result<MovieFragmentBox, MuxError> {
        let runs = runs(&self.movie, samples, None)?;
        let mut tracks: Vec<Vec<&Run<'_>>> = Vec::new();
        for run in &runs {
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
        moof.mfhd.sequence_number = self.sequence_number;
        for track_runs in &tracks {
            let Some(first) = track_runs.first() else {
                continue;
            };
            let mut traf = TrackFragmentBox::default();
            traf.tfhd.track_id = first.track_id;
            traf.tfhd.default_base_is_moof = true;
            traf.tfhd.sample_description_index =
                Some(first.sample_description_index).filter(|&index| {
                    self.movie
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
        self.sequence_number = self.sequence_number.wrapping_add(1);

        Ok(moof)
    }
}

impl SampleTableWriter {
    /// Creates the writer of the sample tables of `movie`
    ///
    /// # Errors
    ///
    /// * [`UnexpectedBox`](MuxError::UnexpectedBox): the movie has an `mvex`,
    ///   which declares a fragmented movie.
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a table of a
    ///   track declares samples already.
    pub(crate) fn new(movie: MovieBox) -> Result<Self, MuxError> {
        if movie.mvex.is_some() {
            return Err(MuxError::UnexpectedBox {
                box_type: MovieExtendsBox::BOX_TYPE,
            });
        }
        refuse_samples_in_tables(&movie)?;
        let tables = movie.trak.iter().map(|_| TrackTable::default()).collect();

        Ok(Self { movie, tables })
    }

    /// Adds `samples`, whose media data lies in their order from file offset `data_offset`, to the sample tables
    ///
    /// Each run of samples of one track that lie one after another is a
    /// chunk (§8.7.4).
    ///
    /// # Errors
    ///
    /// * [`UnknownTrackId`](MuxError::UnknownTrackId): a sample is of a track
    ///   the movie has none of.
    /// * [`UnknownSampleDescriptionIndex`](MuxError::UnknownSampleDescriptionIndex),
    ///   [`UnknownDataReferenceIndex`](MuxError::UnknownDataReferenceIndex),
    ///   [`ExternalDataReference`](MuxError::ExternalDataReference),
    ///   [`UnsupportedDataReference`](MuxError::UnsupportedDataReference): the
    ///   track has no sample entry the sample names, or its entry names a data
    ///   reference entry that is not the file itself.
    /// * [`MixedSampleDescriptions`](MuxError::MixedSampleDescriptions): the
    ///   samples of one track are described by more than one sample entry.
    /// * [`DecodingTimeMismatch`](MuxError::DecodingTimeMismatch): a sample
    ///   does not start where the one before it of its track ends.
    /// * [`SampleSizeOutOfRange`](MuxError::SampleSizeOutOfRange),
    ///   [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of a sample is longer than a table holds, or lies past 64-bit
    ///   file offsets.
    pub(crate) fn handle_samples(
        &mut self,
        samples: &[Sample],
        data_offset: u64,
    ) -> Result<(), MuxError> {
        let decoding_times: Vec<u64> = self.tables.iter().map(TrackTable::decoding_time).collect();
        let runs = runs(&self.movie, samples, Some(&decoding_times))?;
        if let Some(last) = runs.last() {
            let run_len = last
                .samples
                .iter()
                .map(|sample| to_u64(sample.data().len()));
            data_offset
                .checked_add(last.data_offset)
                .and_then(|offset| offset.checked_add(run_len.fold(0, u64::saturating_add)))
                .ok_or(MuxError::DataOffsetOutOfRange {
                    track_id: last.track_id,
                })?;
        }

        for run in runs {
            if let Some(table) = self.tables.get_mut(run.track) {
                table.push_chunk(
                    data_offset.saturating_add(run.data_offset),
                    run.sample_description_index,
                    run.samples,
                );
            }
        }

        Ok(())
    }

    /// Returns the movie with the sample tables of the samples written, and the durations they make up
    ///
    /// A track's `mdhd` takes the duration of its samples, and its `tkhd` the
    /// same in the timescale of the movie, the longest of which the `mvhd`
    /// takes (§8.2.2, §8.3.2, §8.4.2).
    pub(crate) fn finish(self) -> MovieBox {
        let mut movie = self.movie;
        let movie_timescale = movie.mvhd.timescale;
        let mut movie_duration = 0_u64;
        for (trak, table) in movie.trak.iter_mut().zip(self.tables) {
            let media_duration = table.decoding_time();
            let track_duration = u128::from(media_duration)
                .saturating_mul(u128::from(movie_timescale))
                .checked_div(u128::from(trak.mdia.mdhd.timescale))
                .map_or(0, |duration| u64::try_from(duration).unwrap_or(u64::MAX));
            trak.mdia.mdhd.duration = media_duration;
            trak.mdia.mdhd.version = trak.mdia.mdhd.version.max(version_for(media_duration));
            trak.tkhd.duration = track_duration;
            trak.tkhd.version = trak.tkhd.version.max(version_for(track_duration));
            movie_duration = movie_duration.max(track_duration);
            table.fill(&mut trak.mdia.minf.stbl);
        }
        movie.mvhd.duration = movie_duration;
        movie.mvhd.version = movie.mvhd.version.max(version_for(movie_duration));

        movie
    }
}

/// Refuses `movie` where a table of the sample table of a track declares samples
fn refuse_samples_in_tables(movie: &MovieBox) -> Result<(), MuxError> {
    for trak in &movie.trak {
        let stbl = &trak.mdia.minf.stbl;
        let declared = [
            (
                stbl.stts
                    .iter()
                    .flat_map(|stts| &stts.entries)
                    .any(|entry| entry.sample_count > 0),
                TimeToSampleBox::BOX_TYPE,
            ),
            (
                stbl.ctts
                    .iter()
                    .flat_map(|ctts| &ctts.entries)
                    .any(|entry| entry.sample_count > 0),
                CompositionOffsetBox::BOX_TYPE,
            ),
            (
                stbl.stsc.iter().any(|stsc| !stsc.entries.is_empty()),
                SampleToChunkBox::BOX_TYPE,
            ),
            (
                super::sample_table::sample_count(trak) > 0,
                SampleSizeBox::BOX_TYPE,
            ),
            (
                match &stbl.chunk_offset {
                    Some(ChunkOffsetVariant::ChunkOffset(stco)) => !stco.chunk_offsets.is_empty(),
                    Some(ChunkOffsetVariant::ChunkLargeOffset(co64)) => {
                        !co64.chunk_offsets.is_empty()
                    }
                    Some(_) | None => false,
                },
                ChunkOffsetBox::BOX_TYPE,
            ),
            (
                stbl.stss.iter().any(|stss| !stss.sample_numbers.is_empty()),
                SyncSampleBox::BOX_TYPE,
            ),
            (
                stbl.sdtp.iter().any(|sdtp| !sdtp.entries.is_empty()),
                SampleDependencyTypeBox::BOX_TYPE,
            ),
            (
                stbl.stsh.iter().any(|stsh| !stsh.entries.is_empty()),
                ShadowSyncSampleBox::BOX_TYPE,
            ),
            (
                stbl.padb.iter().any(|padb| !padb.pads.is_empty()),
                PaddingBitsBox::BOX_TYPE,
            ),
            (
                stbl.stdp.iter().any(|stdp| !stdp.priorities.is_empty()),
                DegradationPriorityBox::BOX_TYPE,
            ),
            (
                stbl.sbgp
                    .iter()
                    .flat_map(|sbgp| &sbgp.entries)
                    .any(|entry| entry.sample_count > 0),
                SampleToGroupBox::BOX_TYPE,
            ),
            (
                stbl.csgp
                    .iter()
                    .flat_map(|csgp| &csgp.patterns)
                    .any(|pattern| pattern.sample_count > 0),
                CompactSampleToGroupBox::BOX_TYPE,
            ),
            (
                stbl.subs.iter().any(|subs| !subs.entries.is_empty()),
                SubSampleInformationBox::BOX_TYPE,
            ),
            (
                stbl.ssrt.iter().any(|ssrt| !ssrt.entries.is_empty()),
                SubSampleReferenceTableBox::BOX_TYPE,
            ),
            (
                stbl.saiz.iter().any(|saiz| match &saiz.sample_info_sizes {
                    SampleInfoSizes::Constant { sample_count, .. } => *sample_count > 0,
                    SampleInfoSizes::Table { sample_info_sizes } => !sample_info_sizes.is_empty(),
                    _ => true,
                }),
                SampleAuxiliaryInformationSizesBox::BOX_TYPE,
            ),
            (
                stbl.saio.iter().any(|saio| !saio.offsets.is_empty()),
                SampleAuxiliaryInformationOffsetsBox::BOX_TYPE,
            ),
        ];
        if let Some((_, box_type)) = declared.into_iter().find(|(declared, _)| *declared) {
            return Err(MuxError::SampleTableNotEmpty {
                track_id: trak.tkhd.track_id,
                box_type,
            });
        }
    }

    Ok(())
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
/// * The failures of the samples as for [`SampleTableWriter::handle_samples`].
fn runs<'a>(
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

/// Returns the version of a header box that holds `duration`: 1 where it takes more than 32 bits
fn version_for(duration: u64) -> u8 {
    u8::from(u32::try_from(duration).is_err())
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{
        DataEntry, DataEntryUrlBox, MovieBox, MovieExtendsBox, SampleFlags, TrackBox,
        TrackExtendsBox,
    };
    use isobmff_core::BoxDefinition;

    use super::{MovieFragmentWriter, SampleTableWriter};
    use crate::error::MuxError;
    use crate::sample::{Sample, SampleProperties};
    use crate::structure_data::fixtures::{encoded, movie, track, written};

    /// Returns a track `track_id` of no sample, its timescale `timescale`
    fn empty_track(track_id: u32, timescale: u32) -> TrackBox {
        let mut trak = track(track_id, 0, &[], &[], &[]);
        trak.mdia.mdhd.timescale = timescale;
        trak
    }

    /// Returns a movie of tracks 1 and 2 of no sample, the movie and both tracks in a timescale of 1000 and 100
    fn two_tracks() -> MovieBox {
        let mut movie = movie([empty_track(1, 100), empty_track(2, 100)]);
        movie.mvhd.timescale = 1000;
        movie.mvhd.next_track_id = 3;
        movie
    }

    /// Returns [`two_tracks`] with a `trex` for each track, its samples described by sample entry 1
    fn two_fragmented_tracks() -> MovieBox {
        let mut movie = two_tracks();
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = [1, 2]
            .into_iter()
            .map(|track_id| {
                let mut trex = TrackExtendsBox::default();
                trex.track_id = track_id;
                trex.default_sample_description_index = 1;
                trex
            })
            .collect();
        movie.mvex = Some(mvex);
        movie
    }

    /// Returns a sample of `track_id` at `decoding_time`, lasting `sample_duration`, its media data `data`
    fn sample(track_id: u32, decoding_time: u64, sample_duration: u32, data: &[u8]) -> Sample {
        Sample::new(
            SampleProperties {
                track_id,
                decoding_time,
                sample_duration,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            data.to_vec(),
        )
    }

    /// Returns [`sample`] with a composition time offset of `offset`, and not a sync sample
    fn offset_sample(
        track_id: u32,
        decoding_time: u64,
        sample_duration: u32,
        data: &[u8],
        offset: i64,
    ) -> Sample {
        let mut sample = sample(track_id, decoding_time, sample_duration, data);
        let mut properties = *sample.properties();
        properties.sample_composition_time_offset = offset;
        properties.sample_flags.sample_is_non_sync_sample = true;
        sample = Sample::new(properties, sample.into_data());
        sample
    }

    #[test]
    fn a_movie_fragment_states_each_track_in_a_track_fragment_based_on_the_moof() {
        let mut writer = MovieFragmentWriter::new(two_fragmented_tracks(), 7).unwrap();
        let samples = [
            sample(1, 100, 10, b"AB"),
            sample(1, 110, 10, b"CDE"),
            offset_sample(2, 0, 20, b"FGHI", 5),
        ];

        let moof = writer.movie_fragment(&samples, 8).unwrap();

        // moof 184 bytes, so the media data starts at 184 + 8
        let traf_1 = written(
            b"traf",
            &[
                &written(b"tfhd", &[&[0, 0x02, 0, 0], &1_u32.to_be_bytes()]),
                &written(b"tfdt", &[&[0, 0, 0, 0], &100_u32.to_be_bytes()]),
                &written(
                    b"trun",
                    &[
                        &[0, 0, 0x07, 0x01],
                        &2_u32.to_be_bytes(),
                        &192_i32.to_be_bytes(),
                        &10_u32.to_be_bytes(),
                        &2_u32.to_be_bytes(),
                        &[0; 4],
                        &10_u32.to_be_bytes(),
                        &3_u32.to_be_bytes(),
                        &[0; 4],
                    ],
                ),
            ],
        );
        let traf_2 = written(
            b"traf",
            &[
                &written(b"tfhd", &[&[0, 0x02, 0, 0], &2_u32.to_be_bytes()]),
                &written(b"tfdt", &[&[0, 0, 0, 0], &0_u32.to_be_bytes()]),
                &written(
                    b"trun",
                    &[
                        &[0, 0, 0x0f, 0x01],
                        &1_u32.to_be_bytes(),
                        &197_i32.to_be_bytes(),
                        &20_u32.to_be_bytes(),
                        &4_u32.to_be_bytes(),
                        &[0, 0x01, 0, 0],
                        &5_u32.to_be_bytes(),
                    ],
                ),
            ],
        );
        let expected = written(
            b"moof",
            &[
                &written(b"mfhd", &[&[0; 4], &7_u32.to_be_bytes()]),
                &traf_1,
                &traf_2,
            ],
        );
        assert_eq!(encoded(&moof), expected);
        assert_eq!(
            writer
                .movie_fragment(&samples, 8)
                .map(|moof| moof.mfhd.sequence_number),
            Ok(8)
        );
    }

    #[test]
    fn samples_of_a_track_whose_runs_in_a_fragment_do_not_follow_on_are_refused() {
        let mut writer = MovieFragmentWriter::new(two_fragmented_tracks(), 1).unwrap();

        assert_eq!(
            writer
                .movie_fragment(
                    &[
                        sample(1, 0, 10, b"AB"),
                        sample(2, 0, 20, b"CD"),
                        sample(1, 15, 10, b"EF")
                    ],
                    8,
                )
                .map(|_| ()),
            Err(MuxError::DecodingTimeMismatch {
                track_id: 1,
                expected_decoding_time: 10,
                decoding_time: 15
            })
        );
    }

    #[test]
    fn the_sample_tables_declare_the_samples_in_chunks_of_each_track_and_the_durations_they_make_up()
     {
        let mut writer = SampleTableWriter::new(two_tracks()).unwrap();
        writer
            .handle_samples(
                &[
                    sample(1, 0, 10, b"AB"),
                    sample(1, 10, 10, b"CD"),
                    offset_sample(2, 0, 20, b"EFG", 5),
                ],
                1000,
            )
            .unwrap();
        writer
            .handle_samples(&[sample(1, 20, 10, b"HI")], 2000)
            .unwrap();

        let movie = writer.finish();

        let tables = |trak: &TrackBox| {
            let stbl = &trak.mdia.minf.stbl;
            [
                stbl.stts.as_ref().map(encoded),
                stbl.ctts.as_ref().map(encoded),
                stbl.stsc.as_ref().map(encoded),
                stbl.sample_size.as_ref().map(|stsz| match stsz {
                    isobmff_boxes::SampleSizeVariant::SampleSize(stsz) => encoded(stsz),
                    isobmff_boxes::SampleSizeVariant::CompactSampleSize(stz2) => encoded(stz2),
                    _ => Vec::new(),
                }),
                stbl.chunk_offset.as_ref().map(|stco| match stco {
                    isobmff_boxes::ChunkOffsetVariant::ChunkOffset(stco) => encoded(stco),
                    isobmff_boxes::ChunkOffsetVariant::ChunkLargeOffset(co64) => encoded(co64),
                    _ => Vec::new(),
                }),
                stbl.stss.as_ref().map(encoded),
                stbl.sdtp.as_ref().map(encoded),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .concat()
        };
        let full = |box_type: &[u8; 4], fields: &[&[u8]]| {
            written(box_type, &[&[&[0_u8; 4][..]], fields].concat())
        };
        let track_1 = [
            full(
                b"stts",
                &[
                    &1_u32.to_be_bytes(),
                    &3_u32.to_be_bytes(),
                    &10_u32.to_be_bytes(),
                ],
            ),
            full(
                b"stsc",
                &[
                    &2_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                    &2_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                    &2_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                ],
            ),
            full(b"stsz", &[&2_u32.to_be_bytes(), &3_u32.to_be_bytes()]),
            full(
                b"stco",
                &[
                    &2_u32.to_be_bytes(),
                    &1000_u32.to_be_bytes(),
                    &2000_u32.to_be_bytes(),
                ],
            ),
        ]
        .concat();
        let track_2 = [
            full(
                b"stts",
                &[
                    &1_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                    &20_u32.to_be_bytes(),
                ],
            ),
            full(
                b"ctts",
                &[
                    &1_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                    &5_u32.to_be_bytes(),
                ],
            ),
            full(
                b"stsc",
                &[
                    &1_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                    &1_u32.to_be_bytes(),
                ],
            ),
            full(b"stsz", &[&3_u32.to_be_bytes(), &1_u32.to_be_bytes()]),
            full(b"stco", &[&1_u32.to_be_bytes(), &1004_u32.to_be_bytes()]),
            full(b"stss", &[&0_u32.to_be_bytes()]),
        ]
        .concat();
        let durations: Vec<_> = movie
            .trak
            .iter()
            .map(|trak| (tables(trak), trak.mdia.mdhd.duration, trak.tkhd.duration))
            .collect();
        assert_eq!(durations, [(track_1, 30, 300), (track_2, 20, 200)]);
        assert_eq!(movie.mvhd.duration, 300);
    }

    #[test]
    fn samples_the_movie_does_not_declare_in_the_file_itself_are_refused() {
        let mut external = two_tracks();
        if let Some(trak) = external.trak.get_mut(1) {
            let mut url = DataEntryUrlBox::default();
            url.location = Some(isobmff_core::Utf8String::from_slice(b"other.mp4"));
            trak.mdia.minf.dinf.dref.data_entries = Vec::from([DataEntry::Url(url)]);
        }
        let mut writer = SampleTableWriter::new(external).unwrap();
        let mut other_description = sample(1, 0, 10, b"AB");
        let mut properties = *other_description.properties();
        properties.sample_description_index = 2;
        other_description = Sample::new(properties, other_description.into_data());

        let refused = [
            writer.handle_samples(&[sample(3, 0, 10, b"AB")], 0),
            writer.handle_samples(&[other_description.clone()], 0),
            writer.handle_samples(&[sample(2, 0, 10, b"AB")], 0),
            writer.handle_samples(
                &[
                    sample(1, 0, 10, b"AB"),
                    sample(1, 10, 10, b"CD"),
                    other_description,
                ],
                0,
            ),
            writer.handle_samples(&[sample(1, 5, 10, b"AB")], 0),
        ];

        assert_eq!(
            refused,
            [
                Err(MuxError::UnknownTrackId { track_id: 3 }),
                Err(MuxError::UnknownSampleDescriptionIndex {
                    track_id: 1,
                    sample_description_index: 2
                }),
                Err(MuxError::ExternalDataReference {
                    track_id: 2,
                    data_reference_index: 1
                }),
                Err(MuxError::MixedSampleDescriptions { track_id: 1 }),
                Err(MuxError::DecodingTimeMismatch {
                    track_id: 1,
                    expected_decoding_time: 0,
                    decoding_time: 5
                }),
            ]
        );
        assert_eq!(
            writer
                .finish()
                .trak
                .first()
                .map(|trak| trak.mdia.mdhd.duration),
            Some(0)
        );
    }

    #[test]
    fn a_movie_whose_auxiliary_tables_declare_samples_is_refused() {
        let declaring = |stbl: fn(&mut isobmff_boxes::SampleTableBox)| {
            let mut movie = two_tracks();
            if let Some(trak) = movie.trak.get_mut(1) {
                stbl(&mut trak.mdia.minf.stbl);
            }
            SampleTableWriter::new(movie).map(|_| ())
        };
        let refused = |box_type| {
            Err(MuxError::SampleTableNotEmpty {
                track_id: 2,
                box_type,
            })
        };

        assert_eq!(
            [
                declaring(|stbl| {
                    let mut saiz = isobmff_boxes::SampleAuxiliaryInformationSizesBox::default();
                    saiz.sample_info_sizes = isobmff_boxes::SampleInfoSizes::Table {
                        sample_info_sizes: Vec::from([16]),
                    };
                    stbl.saiz = Vec::from([saiz]);
                }),
                declaring(|stbl| {
                    let mut saio = isobmff_boxes::SampleAuxiliaryInformationOffsetsBox::default();
                    saio.offsets = Vec::from([100]);
                    stbl.saio = Vec::from([saio]);
                }),
                declaring(|stbl| {
                    let mut sbgp = isobmff_boxes::SampleToGroupBox::default();
                    let mut entry = isobmff_boxes::SampleToGroupEntry::default();
                    entry.sample_count = 1;
                    sbgp.entries = Vec::from([entry]);
                    stbl.sbgp = Vec::from([sbgp]);
                }),
                declaring(|stbl| {
                    let mut stdp = isobmff_boxes::DegradationPriorityBox::default();
                    stdp.priorities = Vec::from([1]);
                    stbl.stdp = Some(stdp);
                }),
            ],
            [
                refused(isobmff_boxes::SampleAuxiliaryInformationSizesBox::BOX_TYPE),
                refused(isobmff_boxes::SampleAuxiliaryInformationOffsetsBox::BOX_TYPE),
                refused(isobmff_boxes::SampleToGroupBox::BOX_TYPE),
                refused(isobmff_boxes::DegradationPriorityBox::BOX_TYPE),
            ]
        );
    }

    #[test]
    fn a_movie_that_does_not_fit_the_structure_it_is_written_in_is_refused() {
        let mut declared = two_tracks();
        declared.trak = Vec::from([track(1, 10, &[2], &[(1, 1)], &[0])]);
        let mut extends_one = two_fragmented_tracks();
        if let Some(mvex) = extends_one.mvex.as_mut() {
            mvex.trex.truncate(1);
        }

        assert_eq!(
            [
                SampleTableWriter::new(two_fragmented_tracks()).map(|_| ()),
                SampleTableWriter::new(declared).map(|_| ()),
                MovieFragmentWriter::new(extends_one, 1).map(|_| ()),
            ],
            [
                Err(MuxError::UnexpectedBox {
                    box_type: MovieExtendsBox::BOX_TYPE
                }),
                Err(MuxError::SampleTableNotEmpty {
                    track_id: 1,
                    box_type: isobmff_boxes::TimeToSampleBox::BOX_TYPE
                }),
                Err(MuxError::MissingTrackExtends { track_id: 2 }),
            ]
        );
    }
}
