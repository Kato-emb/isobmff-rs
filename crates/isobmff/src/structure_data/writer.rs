//! [`StructureDataWriter`], which writes the structure-data that declares samples: the sample tables of a movie, or its movie fragments

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use isobmff_boxes::{
    MovieBox, MovieExtendsBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox,
    TrackExtendsBox,
};
use isobmff_core::BoxEncode;

use super::movie_fragment::{self, runs, track_fragments};
use super::random_access::RandomAccessTables;
use super::sample_table::{TrackTable, take_sample_tables};
use super::segment_index::SubsegmentTable;
use crate::error::MuxError;
use crate::framing::to_u64;
use crate::sample::Sample;

/// Writer of the structure-data of a file: a movie fragment for each run of samples, or the sample tables of its movie
///
/// Each writer holds the movie it was made from, and refuses samples the
/// movie does not declare a track, a sample entry or a data reference entry
/// in the file itself for: the media data it writes is in the file itself.
#[derive(Debug)]
pub(crate) enum StructureDataWriter {
    /// Movie fragments, §8.8
    Fragmented(MovieFragmentWriter),
    /// The sample tables of the movie, §8.5
    NonFragmented(SampleTableWriter),
}

impl StructureDataWriter {
    /// Returns the movie the writer was made from, in the form it writes it
    pub(crate) const fn movie(&self) -> &MovieBox {
        match self {
            Self::Fragmented(writer) => writer.movie(),
            Self::NonFragmented(writer) => writer.movie(),
        }
    }
}

/// Writer of the movie fragments of a movie, numbered on from `sequence_number`
#[derive(Debug)]
pub(crate) struct MovieFragmentWriter {
    movie: MovieBox,
    sequence_number: u32,
    /// Sync samples of the fragments written, where the file ends with an `mfra`
    random_access: Option<RandomAccessTables>,
    /// Subsegments of the fragments written, where the segment starts with a `sidx`
    segment_index: Option<SubsegmentTable>,
}

/// Writer of the sample table of each track of a movie, in the order of its tracks
#[derive(Debug)]
pub(crate) struct SampleTableWriter {
    movie: MovieBox,
    tables: Vec<TrackTable>,
}

impl MovieFragmentWriter {
    /// Creates the writer of the movie fragments of `movie`, the first numbered `first_sequence_number`, that lists their sync samples where `random_access` and indexes them as subsegments where `segment_index`
    ///
    /// The movie the writer holds has empty sample tables, and an `mvex` with a `trex` for each track.
    ///
    /// # Errors
    ///
    /// * The failure of [`take_sample_tables`].
    /// * [`EditListNotApplied`](MuxError::EditListNotApplied): the writer
    ///   indexes subsegments, and the edit list of the first track moves its
    ///   presentation times off its composition times.
    pub(crate) fn new(
        mut movie: MovieBox,
        first_sequence_number: u32,
        random_access: bool,
        segment_index: bool,
    ) -> Result<Self, MuxError> {
        take_sample_tables(&mut movie)?;
        let mvex = movie.mvex.get_or_insert_with(MovieExtendsBox::default);
        let extended: BTreeSet<u32> = mvex.trex.iter().map(|trex| trex.track_id).collect();
        for trak in &movie.trak {
            let track_id = trak.tkhd.track_id;
            if !extended.contains(&track_id) {
                let mut trex = TrackExtendsBox::default();
                trex.track_id = track_id;
                trex.default_sample_description_index = 1;
                mvex.trex.push(trex);
            }
        }

        Ok(Self {
            segment_index: segment_index
                .then(|| SubsegmentTable::new(&movie))
                .transpose()?,
            movie,
            sequence_number: first_sequence_number,
            random_access: random_access.then(RandomAccessTables::default),
        })
    }

    /// Returns the movie the writer was made from
    pub(crate) const fn movie(&self) -> &MovieBox {
        &self.movie
    }

    /// Returns the movie fragment of `samples`, its `moof` at file offset `moof_offset` and their media data in their order from `data_start` bytes past its end
    ///
    /// Each track has one track fragment, its base data offset the first byte
    /// of the `moof` (§8.8.7.1) and its `tfdt` the decoding time of its first
    /// sample (§8.8.12), and a track run for each run of its samples that lie
    /// one after another (§8.8.8). The next fragment takes the next sequence
    /// number. A writer that lists sync samples lists the first of each
    /// track in the fragment, and one that indexes subsegments indexes the
    /// fragment and its media data as one.
    ///
    /// # Errors
    ///
    /// * The failures of the samples as for
    ///   [`SampleTableWriter::handle_samples`], but for their decoding times,
    ///   which follow from the durations within the fragment only.
    /// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of a sample lies further from the `moof` than a `trun` reaches.
    /// * [`SegmentIndexOutOfRange`](MuxError::SegmentIndexOutOfRange): the
    ///   `sidx` cannot state the fragment as a subsegment.
    pub(crate) fn movie_fragment(
        &mut self,
        samples: &[Sample],
        moof_offset: u64,
        data_start: u64,
    ) -> Result<MovieFragmentBox, MuxError> {
        let runs = runs(&self.movie, samples, None)?;
        let track_fragments = track_fragments(&runs);
        let moof = movie_fragment::movie_fragment_box(
            &self.movie,
            self.sequence_number,
            &track_fragments,
            data_start,
        )?;
        if let Some(segment_index) = &mut self.segment_index {
            let referenced_size = samples
                .iter()
                .map(|sample| to_u64(sample.data().len()))
                .fold(
                    moof.encoded_len().saturating_add(data_start),
                    u64::saturating_add,
                );
            segment_index.handle_movie_fragment(&track_fragments, referenced_size)?;
        }
        self.sequence_number = self.sequence_number.wrapping_add(1);
        if let Some(random_access) = &mut self.random_access {
            random_access.handle_movie_fragment(&track_fragments, moof_offset);
        }

        Ok(moof)
    }

    /// Returns whether the writer indexes the fragments it writes as the subsegments of a `sidx`
    pub(crate) const fn indexes_subsegments(&self) -> bool {
        self.segment_index.is_some()
    }

    /// Returns the `mfra` of the sync samples listed and the `sidx` of the subsegments indexed, where the writer lists and indexes them, §8.8.9, §8.14.3
    ///
    /// # Errors
    ///
    /// * [`Box`](MuxError::Box): the `mfra` is longer than its `mfro` counts.
    pub(crate) fn finish(
        &mut self,
    ) -> Result<
        (
            Option<MovieFragmentRandomAccessBox>,
            Option<SegmentIndexBox>,
        ),
        MuxError,
    > {
        let mfra = self
            .random_access
            .take()
            .map(RandomAccessTables::finish)
            .transpose()?;

        Ok((mfra, self.segment_index.take().map(SubsegmentTable::finish)))
    }
}

impl SampleTableWriter {
    /// Creates the writer of the sample tables of `movie`
    ///
    /// The writer fills the sample tables from the samples it takes, and the movie it holds has no `mvex`.
    ///
    /// # Errors
    ///
    /// * The failure of [`take_sample_tables`].
    pub(crate) fn new(mut movie: MovieBox) -> Result<Self, MuxError> {
        take_sample_tables(&mut movie)?;
        movie.mvex = None;
        let tables = movie.trak.iter().map(|_| TrackTable::default()).collect();

        Ok(Self { movie, tables })
    }

    /// Returns the movie the writer was made from, its sample tables those of the samples to come
    pub(crate) const fn movie(&self) -> &MovieBox {
        &self.movie
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

    /// Fills the sample tables of the movie with the samples written, and its durations with those they make up, and returns the movie
    ///
    /// A track's `mdhd` takes the duration of its samples, and its `tkhd` the
    /// same in the timescale of the movie, the longest of which the `mvhd`
    /// takes (§8.2.2, §8.3.2, §8.4.2).
    pub(crate) fn finish(&mut self) -> &MovieBox {
        let movie = &mut self.movie;
        let movie_timescale = movie.mvhd.timescale;
        let mut movie_duration = 0_u64;
        for (trak, table) in movie.trak.iter_mut().zip(core::mem::take(&mut self.tables)) {
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

/// Returns the version of a header box that holds `duration`: 1 where it takes more than 32 bits
pub(super) fn version_for(duration: u64) -> u8 {
    u8::from(u32::try_from(duration).is_err())
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{MovieBox, MovieExtendsBox, SampleFlags, TrackBox, TrackExtendsBox};
    use isobmff_core::BoxDefinition;

    use super::{MovieFragmentWriter, SampleTableWriter};
    use crate::error::MuxError;
    use crate::sample::{Sample, SampleProperties};
    use crate::structure_data::fixtures::{encoded, in_another_file, movie, track, written};

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

    /// Returns the full box of `box_type` of version 0 and no flags, whose fields after those are `fields`
    fn full(box_type: &[u8; 4], fields: &[&[u8]]) -> Vec<u8> {
        written(box_type, &[&[&[0_u8; 4][..]], fields].concat())
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
        let mut writer =
            MovieFragmentWriter::new(two_fragmented_tracks(), 7, false, false).unwrap();
        let samples = [
            sample(1, 100, 10, b"AB"),
            sample(1, 110, 10, b"CDE"),
            offset_sample(2, 0, 20, b"FGHI", 5),
        ];

        let moof = writer.movie_fragment(&samples, 0, 8).unwrap();

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
                .movie_fragment(&samples, 0, 8)
                .map(|moof| moof.mfhd.sequence_number),
            Ok(8)
        );
    }

    #[test]
    fn samples_of_a_track_whose_runs_in_a_fragment_do_not_follow_on_are_refused() {
        let mut writer =
            MovieFragmentWriter::new(two_fragmented_tracks(), 1, false, false).unwrap();

        assert_eq!(
            writer
                .movie_fragment(
                    &[
                        sample(1, 0, 10, b"AB"),
                        sample(2, 0, 20, b"CD"),
                        sample(1, 15, 10, b"EF")
                    ],
                    0,
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
    fn the_padb_and_the_stdp_declare_the_padding_and_the_degradation_priority_of_each_sample() {
        let flagged = |sample: Sample, sample_padding_value, sample_degradation_priority| {
            let mut properties = *sample.properties();
            properties.sample_flags.sample_padding_value = sample_padding_value;
            properties.sample_flags.sample_degradation_priority = sample_degradation_priority;
            Sample::new(properties, sample.into_data())
        };
        let mut writer = SampleTableWriter::new(two_tracks()).unwrap();
        writer
            .handle_samples(
                &[
                    flagged(sample(1, 0, 10, b"A"), 3, 0),
                    flagged(sample(1, 10, 10, b"B"), 0, 0),
                    flagged(sample(1, 20, 10, b"C"), 5, 0x102),
                    sample(2, 0, 10, b"D"),
                ],
                1000,
            )
            .unwrap();

        let movie = writer.finish();

        let tables: Vec<_> = movie
            .trak
            .iter()
            .map(|trak| {
                let stbl = &trak.mdia.minf.stbl;
                (
                    stbl.padb.as_ref().map(encoded),
                    stbl.stdp.as_ref().map(encoded),
                )
            })
            .collect();
        assert_eq!(
            tables,
            [
                (
                    Some(full(b"padb", &[&3_u32.to_be_bytes(), &[0x30, 0x50]])),
                    Some(full(b"stdp", &[&[0, 0, 0, 0, 1, 2]])),
                ),
                (None, None),
            ]
        );
    }

    #[test]
    fn samples_the_movie_does_not_declare_in_the_file_itself_are_refused() {
        let mut external = two_tracks();
        if let Some(trak) = external.trak.get_mut(1) {
            *trak = in_another_file(trak.clone());
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
            ],
            [
                refused(isobmff_boxes::SampleAuxiliaryInformationSizesBox::BOX_TYPE),
                refused(isobmff_boxes::SampleAuxiliaryInformationOffsetsBox::BOX_TYPE),
                refused(isobmff_boxes::SampleToGroupBox::BOX_TYPE),
            ]
        );
    }

    /// Returns [`two_tracks`] whose track 2 declares two samples in each table a writer writes from samples
    fn two_tracks_declaring_samples() -> MovieBox {
        let mut movie = two_tracks();
        let mut declaring = track(2, 10, &[3, 4], &[(1, 2)], &[500]);
        declaring.mdia.mdhd.timescale = 100;
        let stbl = &mut declaring.mdia.minf.stbl;
        let mut stss = isobmff_boxes::SyncSampleBox::default();
        stss.sample_numbers = Vec::from([1]);
        stbl.stss = Some(stss);
        let mut stdp = isobmff_boxes::DegradationPriorityBox::default();
        stdp.priorities = Vec::from([1, 2]);
        stbl.stdp = Some(stdp);
        movie.trak = Vec::from([empty_track(1, 100), declaring]);
        movie
    }

    #[test]
    fn the_tables_a_writer_writes_from_samples_are_written_from_its_samples_whatever_the_movie_declares()
     {
        let samples = [sample(2, 0, 5, b"ab"), sample(2, 5, 5, b"cd")];
        let tables_of = |movie| {
            let mut writer = SampleTableWriter::new(movie)?;
            writer.handle_samples(&samples, 100)?;
            Ok::<_, MuxError>(writer.finish().clone())
        };
        let fragments_of = |mut movie: MovieBox| {
            movie.mvex = two_fragmented_tracks().mvex;
            MovieFragmentWriter::new(movie, 1, false, false).map(|writer| writer.movie().clone())
        };

        assert_eq!(
            (
                tables_of(two_tracks_declaring_samples()),
                fragments_of(two_tracks_declaring_samples())
            ),
            (tables_of(two_tracks()), fragments_of(two_tracks()))
        );
    }

    #[test]
    fn a_movie_written_without_fragments_has_no_mvex() {
        let mut writer = SampleTableWriter::new(two_fragmented_tracks()).unwrap();

        assert_eq!(writer.finish().mvex, None);
    }

    #[test]
    fn a_movie_written_in_fragments_has_a_trex_for_each_track_keeping_those_it_gave() {
        let mut expected = two_fragmented_tracks();
        if let Some(trex) = expected
            .mvex
            .as_mut()
            .and_then(|mvex| mvex.trex.first_mut())
        {
            trex.default_sample_duration = 7;
        }
        let mut given = expected.clone();
        if let Some(mvex) = given.mvex.as_mut() {
            mvex.trex.truncate(1);
        }
        let written_mvex = |movie| {
            MovieFragmentWriter::new(movie, 1, false, false)
                .map(|writer| writer.movie().mvex.clone())
        };

        assert_eq!(
            [written_mvex(two_tracks()), written_mvex(given)],
            [Ok(two_fragmented_tracks().mvex), Ok(expected.mvex)]
        );
    }
}
