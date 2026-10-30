//! [`StructureDataWriter`], which writes the structure-data that declares samples: the sample tables of a movie, or its movie fragments

use alloc::vec::Vec;

use isobmff_boxes::{
    ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox, MovieBox, MovieExtendsBox,
    MovieFragmentBox, SampleDependencyTypeBox, SampleSizeBox, SampleTableBox, SampleToChunkBox,
    SyncSampleBox, TimeToSampleBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
    TrackRunBox, TrackRunSample, TrackRunSamples,
};
use isobmff_core::{BoxDefinition, BoxEncode};

mod sample_table;

use sample_table::TrackTable;

use super::data_reference_index;
use crate::error::MuxError;
use crate::framing::to_u64;
use crate::sample::Sample;

/// Writer of the structure-data of a file: the sample tables of its movie, or a movie fragment for each run of samples
///
/// The writer holds the movie it was made from, and refuses samples the
/// movie does not declare a track, a sample entry or a data reference entry
/// in the file itself for, as a reader of the file would.
#[derive(Debug)]
pub(crate) struct StructureDataWriter {
    movie: MovieBox,
    form: Form,
}

/// What the writer writes the samples into
#[derive(Debug)]
enum Form {
    /// Movie fragments, numbered on from `sequence_number` (§8.8)
    Fragmented { sequence_number: u32 },
    /// The sample table of each track of the movie, in the order of its tracks (§8.5)
    NonFragmented { tables: Vec<TrackTable> },
}

/// Samples of one track that lie one after another, the first of them at `data_offset` from the media data of the samples written together
struct Run<'a> {
    track: usize,
    track_id: u32,
    samples: &'a [Sample],
    data_offset: u64,
}

impl StructureDataWriter {
    /// Creates the writer of the movie fragments of `movie`, the first numbered `first_sequence_number`
    ///
    /// # Errors
    ///
    /// * [`MissingTrackExtends`](MuxError::MissingTrackExtends): a track of
    ///   the movie has no `trex`, which its fragments take defaults from.
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a table of a
    ///   track declares samples, which belong in the fragments.
    pub(crate) fn fragmented(
        movie: MovieBox,
        first_sequence_number: u32,
    ) -> Result<Self, MuxError> {
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
            form: Form::Fragmented {
                sequence_number: first_sequence_number,
            },
        })
    }

    /// Creates the writer of the sample tables of `movie`
    ///
    /// # Errors
    ///
    /// * [`UnexpectedBox`](MuxError::UnexpectedBox): the movie has an `mvex`,
    ///   which declares a fragmented movie.
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a table of a
    ///   track declares samples already.
    pub(crate) fn non_fragmented(movie: MovieBox) -> Result<Self, MuxError> {
        if movie.mvex.is_some() {
            return Err(MuxError::UnexpectedBox {
                box_type: MovieExtendsBox::BOX_TYPE,
            });
        }
        refuse_samples_in_tables(&movie)?;
        let tables = movie.trak.iter().map(|_| TrackTable::default()).collect();

        Ok(Self {
            movie,
            form: Form::NonFragmented { tables },
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
    /// * The failures of the samples as for [`handle_samples`](Self::handle_samples),
    ///   but for a decoding time, which a fragment states for each track.
    /// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of a sample lies further from the `moof` than a `trun` reaches.
    pub(crate) fn movie_fragment(
        &mut self,
        samples: &[Sample],
        data_start: u64,
    ) -> Result<MovieFragmentBox, MuxError> {
        let Form::Fragmented { sequence_number } = &mut self.form else {
            return Err(MuxError::UnexpectedBox {
                box_type: MovieFragmentBox::BOX_TYPE,
            });
        };
        let runs = runs(&self.movie, samples)?;
        let mut moof = MovieFragmentBox::default();
        moof.mfhd.sequence_number = *sequence_number;
        let mut tracks: Vec<usize> = Vec::new();
        for run in &runs {
            if !tracks.contains(&run.track) {
                tracks.push(run.track);
            }
        }
        for &track in &tracks {
            let track_runs: Vec<&Run<'_>> = runs.iter().filter(|run| run.track == track).collect();
            let Some(first) = track_runs
                .first()
                .and_then(|run| run.samples.first())
                .map(Sample::properties)
            else {
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
        for (traf, &track) in moof.traf.iter_mut().zip(&tracks) {
            let track_runs = runs.iter().filter(|run| run.track == track);
            for (trun, run) in traf.trun.iter_mut().zip(track_runs) {
                trun.data_offset = Some(
                    moof_len
                        .checked_add(data_start)
                        .and_then(|offset| offset.checked_add(run.data_offset))
                        .and_then(|offset| i32::try_from(offset).ok())
                        .ok_or(MuxError::DataOffsetOutOfRange {
                            track_id: traf.tfhd.track_id,
                        })?,
                );
            }
        }
        *sequence_number = sequence_number.wrapping_add(1);

        Ok(moof)
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
        let Form::NonFragmented { tables } = &mut self.form else {
            return Err(MuxError::UnexpectedBox {
                box_type: SampleTableBox::BOX_TYPE,
            });
        };
        let runs = runs(&self.movie, samples)?;
        let mut decoding_times: Vec<u64> = tables.iter().map(|table| table.decoding_time).collect();
        for run in &runs {
            let Some(decoding_time) = decoding_times.get_mut(run.track) else {
                continue;
            };
            for sample in run.samples {
                let properties = sample.properties();
                if properties.decoding_time != *decoding_time {
                    return Err(MuxError::DecodingTimeMismatch {
                        track_id: properties.track_id,
                        expected_decoding_time: *decoding_time,
                        decoding_time: properties.decoding_time,
                    });
                }
                *decoding_time = decoding_time
                    .checked_add(u64::from(properties.sample_duration))
                    .ok_or(MuxError::DecodingTimeMismatch {
                        track_id: properties.track_id,
                        expected_decoding_time: *decoding_time,
                        decoding_time: properties.decoding_time,
                    })?;
            }
            data_offset
                .checked_add(run.data_offset)
                .and_then(|offset| {
                    offset.checked_add(to_u64(
                        run.samples.iter().map(|sample| sample.data().len()).sum(),
                    ))
                })
                .ok_or(MuxError::DataOffsetOutOfRange {
                    track_id: run.track_id,
                })?;
        }

        for run in runs {
            let Some(table) = tables.get_mut(run.track) else {
                continue;
            };
            table
                .chunk_offsets
                .push(data_offset.saturating_add(run.data_offset));
            table.chunks.push((
                u32::try_from(run.samples.len()).unwrap_or(u32::MAX),
                run.samples
                    .first()
                    .map_or(0, |sample| sample.properties().sample_description_index),
            ));
            for sample in run.samples {
                let properties = sample.properties();
                table.decoding_time = table
                    .decoding_time
                    .saturating_add(u64::from(properties.sample_duration));
                table.samples.push((
                    properties.sample_duration,
                    u32::try_from(sample.data().len()).unwrap_or(u32::MAX),
                    properties.sample_composition_time_offset,
                    properties.sample_flags,
                ));
            }
        }

        Ok(())
    }

    /// Returns the movie with the sample tables of the samples written, and the durations they make up
    ///
    /// A track's `mdhd` takes the duration of its samples, and its `tkhd` the
    /// same in the timescale of the movie, the longest of which the `mvhd`
    /// takes (§8.2.2, §8.3.2, §8.4.2). A movie of movie fragments is returned
    /// as it was made from.
    pub(crate) fn finish(self) -> MovieBox {
        let mut movie = self.movie;
        let Form::NonFragmented { tables } = self.form else {
            return movie;
        };
        let movie_timescale = movie.mvhd.timescale;
        let mut movie_duration = 0_u64;
        for (trak, table) in movie.trak.iter_mut().zip(tables) {
            let media_duration = table.decoding_time;
            let media_timescale = trak.mdia.mdhd.timescale;
            let track_duration = u128::from(media_duration)
                .saturating_mul(u128::from(movie_timescale))
                .checked_div(u128::from(media_timescale))
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
/// # Errors
///
/// * The failures of the samples as for [`StructureDataWriter::handle_samples`],
///   but for a decoding time across the samples written before.
fn runs<'a>(movie: &MovieBox, samples: &'a [Sample]) -> Result<Vec<Run<'a>>, MuxError> {
    let mut runs: Vec<Run<'a>> = Vec::new();
    let mut descriptions: Vec<(usize, u32)> = Vec::new();
    let mut start = 0;
    let mut data_offset = 0_u64;
    while let Some(first) = samples.get(start) {
        let properties = first.properties();
        let track_id = properties.track_id;
        let track = movie
            .trak
            .iter()
            .position(|trak| trak.tkhd.track_id == track_id)
            .ok_or(MuxError::UnknownTrackId { track_id })?;
        let trak = movie
            .trak
            .get(track)
            .ok_or(MuxError::UnknownTrackId { track_id })?;
        let len = samples
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|sample| sample.properties().track_id == track_id)
            .count();
        let run_samples = samples
            .get(start..start.saturating_add(len))
            .unwrap_or_default();
        let sample_description_index = properties.sample_description_index;
        match descriptions
            .iter()
            .find(|(described, _)| *described == track)
        {
            Some(&(_, described)) if described != sample_description_index => {
                return Err(MuxError::MixedSampleDescriptions { track_id });
            }
            Some(_) => {}
            None => {
                data_reference_index(trak, sample_description_index)?;
                descriptions.push((track, sample_description_index));
            }
        }
        let mut decoding_time = properties.decoding_time;
        for sample in run_samples {
            let sample = sample.properties();
            if sample.sample_description_index != sample_description_index {
                return Err(MuxError::MixedSampleDescriptions { track_id });
            }
            if sample.decoding_time != decoding_time {
                return Err(MuxError::DecodingTimeMismatch {
                    track_id,
                    expected_decoding_time: decoding_time,
                    decoding_time: sample.decoding_time,
                });
            }
            decoding_time = decoding_time.saturating_add(u64::from(sample.sample_duration));
        }
        let mut run_len = 0_u64;
        for sample in run_samples {
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
            samples: run_samples,
            data_offset,
        });
        data_offset = data_offset.saturating_add(run_len);
        start = start.saturating_add(len);
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
    use isobmff_core::{BoxDefinition, BoxEncode};

    use super::StructureDataWriter;
    use crate::error::MuxError;
    use crate::sample::{Sample, SampleProperties};
    use crate::structure_data::fixtures::{movie, track};

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

    /// Returns `value` written as a box
    fn encoded(value: &(impl BoxEncode + BoxDefinition)) -> Vec<u8> {
        let mut bytes = alloc::vec![0; usize::try_from(value.encoded_len()).unwrap()];
        value.encode(&mut bytes).unwrap();
        bytes
    }

    /// Returns the box of `box_type` whose payload is `fields` laid end to end
    fn written(box_type: &[u8; 4], fields: &[&[u8]]) -> Vec<u8> {
        let payload = fields.concat();
        let size = u32::try_from(payload.len().checked_add(8).unwrap()).unwrap();
        [&size.to_be_bytes()[..], box_type, &payload].concat()
    }

    #[test]
    fn a_movie_fragment_states_each_track_in_a_track_fragment_based_on_the_moof() {
        let mut writer = StructureDataWriter::fragmented(two_fragmented_tracks(), 7).unwrap();
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
    fn the_sample_tables_declare_the_samples_in_chunks_of_each_track_and_the_durations_they_make_up()
     {
        let mut writer = StructureDataWriter::non_fragmented(two_tracks()).unwrap();
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
        let mut writer = StructureDataWriter::non_fragmented(external).unwrap();
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
    fn a_movie_that_does_not_fit_the_structure_it_is_written_in_is_refused() {
        let mut declared = two_tracks();
        declared.trak = Vec::from([track(1, 10, &[2], &[(1, 1)], &[0])]);
        let mut extends_one = two_fragmented_tracks();
        if let Some(mvex) = extends_one.mvex.as_mut() {
            mvex.trex.truncate(1);
        }

        assert_eq!(
            [
                StructureDataWriter::non_fragmented(two_fragmented_tracks()).map(|_| ()),
                StructureDataWriter::non_fragmented(declared).map(|_| ()),
                StructureDataWriter::fragmented(extends_one, 1).map(|_| ()),
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
