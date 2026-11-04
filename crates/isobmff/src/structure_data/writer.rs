//! [`MovieFragmentWriter`] and [`SampleTableWriter`], which write the structure-data that declares samples: the movie fragments of a movie, or its sample tables

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use isobmff_boxes::{
    MovieBox, MovieExtendsBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox,
    TrackExtendsBox,
};
use isobmff_core::BoxEncode;

use super::movie_fragment::{self, runs, track_fragments};
use super::random_access::RandomAccessTables;
use super::sample_description::located_in_file;
use super::sample_table::{TrackTable, take_sample_tables};
use super::segment_index::SubsegmentTable;
use crate::error::MuxError;
use crate::file_structure::{FragmentedOptions, SegmentOptions};
use crate::sample::{Sample, SampleLocation};
use crate::timeline;

/// Writer of the movie fragments of a movie, ISO/IEC 14496-12 §8.8
///
/// The writer does no I/O. It takes the samples of each movie fragment, and
/// returns the `moof` that declares them, its media data in an `mdat` that
/// follows it, and the [`SampleLocation`] of each. It refuses samples the
/// movie does not declare a track, a sample entry or a data reference entry
/// in the file itself for. A fragmented movie file ends with the `mfra` it
/// returns as it finishes, where it lists sync samples, and a segment has
/// the `sidx` it returns after its `styp`, where it indexes subsegments.
#[doc(alias = "moof")]
#[derive(Debug)]
pub struct MovieFragmentWriter {
    movie: MovieBox,
    sequence_number: u32,
    /// Sync samples of the fragments written, where the file ends with an `mfra`
    random_access: Option<RandomAccessTables>,
    /// Subsegments of the fragments written, where the segment starts with a `sidx`
    segment_index: Option<SubsegmentTable>,
    finished: bool,
}

/// Writer of the sample table of each track of a movie, ISO/IEC 14496-12 §8.5
///
/// The writer does no I/O. It takes the [`SampleLocation`] of each sample,
/// wherever its media data lies in the file, and fills the sample tables of
/// the movie with them as it finishes. It refuses samples the movie does not
/// declare a track, a sample entry or a data reference entry in the file
/// itself for.
#[doc(alias = "stbl")]
#[derive(Debug)]
pub struct SampleTableWriter {
    movie: MovieBox,
    /// Table of each track, until the writer finishes
    tables: Option<Vec<TrackTable>>,
    /// Whether the movie given was fragmented, in which a last edit of duration 0 lasts to the end of the media (§8.6.6.1)
    fragmented: bool,
}

impl MovieFragmentWriter {
    /// Creates the writer of the movie fragments of a fragmented movie file of `movie`, written with `options`
    ///
    /// The movie the writer holds has empty sample tables, and an `mvex` with
    /// a `trex` for each track. Where the file ends with an `mfra`, the
    /// writer lists the sync samples of the fragments it writes.
    ///
    /// # Errors
    ///
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a sample
    ///   group, sub-sample, sample auxiliary information or shadow sync table
    ///   of a track declares samples, which the writer does not write.
    pub fn fragmented(movie: MovieBox, options: FragmentedOptions) -> Result<Self, MuxError> {
        let mut writer = Self::new(movie, options.first_sequence_number)?;
        writer.random_access = options
            .movie_fragment_random_access
            .then(RandomAccessTables::default);
        Ok(writer)
    }

    /// Creates the writer of the movie fragments of a media segment of `movie`, written with `options`
    ///
    /// The movie the writer holds has empty sample tables, and an `mvex` with
    /// a `trex` for each track. Where a `sidx` indexes the segment, the
    /// writer indexes each fragment it writes as a subsegment, as many as
    /// [`SegmentOptions::segment_index`] says: it refuses a fragment past the
    /// number, and [`finish`](Self::finish) refuses fewer.
    /// [`segment_index_len`](Self::segment_index_len) is the length of the
    /// `sidx` from the start, so that a writer can reserve its space: the
    /// `sidx` is of version 1, its earliest presentation time and first
    /// offset in 64 bits whatever they are.
    ///
    /// # Errors
    ///
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a sample
    ///   group, sub-sample, sample auxiliary information or shadow sync table
    ///   of a track declares samples, which the writer does not write.
    /// * [`Timeline`](MuxError::Timeline): a `sidx` indexes the segment, and
    ///   the edit list of the first track does not map its composition
    ///   timeline.
    /// * [`SegmentIndexOutOfRange`](MuxError::SegmentIndexOutOfRange): the
    ///   number of subsegments is past the 65,535 a `sidx` references.
    pub fn segment(movie: MovieBox, options: SegmentOptions) -> Result<Self, MuxError> {
        let mut writer = Self::new(movie, options.first_sequence_number)?;
        writer.segment_index = options
            .segment_index
            .map(|subsegments| SubsegmentTable::new(&writer.movie, subsegments))
            .transpose()?;
        Ok(writer)
    }

    /// Returns the length of the `sidx` [`finish`](Self::finish) returns, where the writer indexes subsegments
    #[must_use]
    pub fn segment_index_len(&self) -> Option<u64> {
        self.segment_index
            .as_ref()
            .map(SubsegmentTable::encoded_len)
    }

    /// Creates the writer of the movie fragments of `movie`, the first numbered `first_sequence_number`, that lists no sync sample and indexes no subsegment
    ///
    /// # Errors
    ///
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty), as for
    ///   [`fragmented`](Self::fragmented).
    fn new(mut movie: MovieBox, first_sequence_number: u32) -> Result<Self, MuxError> {
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
            movie,
            sequence_number: first_sequence_number,
            random_access: None,
            segment_index: None,
            finished: false,
        })
    }

    /// Returns the movie the writer was made from, in the form the fragments extend: its `moov`
    #[must_use]
    pub const fn movie(&self) -> &MovieBox {
        &self.movie
    }

    /// Returns the movie fragment of `samples`, its `moof` at file offset `moof_offset` and their media data in their order from `data_start` bytes past its end, and the location of each sample in that order
    ///
    /// Each track has one track fragment, its base data offset the first byte
    /// of the `moof` (§8.8.7.1) and its `tfdt` the decoding time of its first
    /// sample (§8.8.12), and a track run for each run of its samples that lie
    /// one after another (§8.8.8). The next fragment takes the next sequence
    /// number. A writer that lists sync samples lists the first of each
    /// track in the fragment, and one that indexes subsegments indexes the
    /// fragment and its media data as one.
    ///
    /// The `mdat` goes right after the `moof`, its header `data_start` bytes
    /// long, as a [`MediaDataWriter`](crate::media_data::MediaDataWriter)
    /// writes it. The locations are those a
    /// [`StructureDataReader`](super::StructureDataReader) resolves out of the
    /// `moof`: the file offsets of the media data of each sample, and the key
    /// of the data reference entry its sample entry names.
    ///
    /// A refused call leaves the writer as it was.
    ///
    /// # Errors
    ///
    /// * The failures of the samples as for
    ///   [`SampleTableWriter::handle_locations`], but for their decoding
    ///   times, which follow from the durations within the fragment only.
    /// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of a sample lies further from the `moof` than a `trun` reaches,
    ///   or past 64-bit file offsets.
    /// * [`SegmentIndexOutOfRange`](MuxError::SegmentIndexOutOfRange): the
    ///   `sidx` cannot state the fragment as a subsegment.
    /// * [`SegmentIndexCountMismatch`](MuxError::SegmentIndexCountMismatch):
    ///   the fragment is past the number of subsegments the writer indexes.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the writer is
    ///   finished.
    pub fn movie_fragment(
        &mut self,
        samples: &[Sample],
        moof_offset: u64,
        data_start: u64,
    ) -> Result<(MovieFragmentBox, Vec<SampleLocation>), MuxError> {
        if self.finished {
            return Err(MuxError::AlreadyFinished);
        }
        let laid_out = located_in_file(&self.movie, samples, 0)?;
        let data_len = laid_out.last().map_or(0, |last| last.byte_range().end);
        let runs = runs(&self.movie, &laid_out, &[])?;
        let track_fragments = track_fragments(&runs);
        // Why not a moof of the data offsets the samples end up at: its length does not depend on them, so the moof is laid out once and its data offsets count from where it ends
        let moof = movie_fragment::movie_fragment_box(
            &self.movie,
            self.sequence_number,
            &track_fragments,
            data_start,
        )?;
        let moof_len = moof.encoded_len();
        let data_origin = match runs.first() {
            Some(first) => moof_offset
                .checked_add(moof_len)
                .and_then(|end| end.checked_add(data_start))
                .filter(|origin| origin.checked_add(data_len).is_some())
                .ok_or(MuxError::DataOffsetOutOfRange {
                    track_id: first.track_id,
                })?,
            None => 0,
        };
        if let Some(segment_index) = &mut self.segment_index {
            let referenced_size = moof_len.saturating_add(data_start).saturating_add(data_len);
            segment_index.handle_movie_fragment(&self.movie, &track_fragments, referenced_size)?;
        }
        self.sequence_number = self.sequence_number.wrapping_add(1);
        if let Some(random_access) = &mut self.random_access {
            random_access.handle_movie_fragment(&track_fragments, moof_offset);
        }
        let locations = laid_out
            .iter()
            .map(|location| {
                let range = location.byte_range();
                location.with_byte_range(
                    data_origin.saturating_add(range.start)..data_origin.saturating_add(range.end),
                )
            })
            .collect();

        Ok((moof, locations))
    }

    /// Returns the `mfra` of the sync samples listed and the `sidx` of the subsegments indexed, where the writer lists and indexes them, §8.8.9, §8.14.3
    ///
    /// The `mfra` goes at the end of the file. The `sidx` goes before the
    /// first `moof`, and its first subsegment starts right after it. A second
    /// call returns neither, and the writer takes no more samples.
    ///
    /// # Errors
    ///
    /// * [`Box`](MuxError::Box): the `mfra` is longer than its `mfro` counts.
    /// * [`SegmentIndexCountMismatch`](MuxError::SegmentIndexCountMismatch):
    ///   the writer indexes fewer subsegments than it was made for.
    pub fn finish(
        &mut self,
    ) -> Result<
        (
            Option<MovieFragmentRandomAccessBox>,
            Option<SegmentIndexBox>,
        ),
        MuxError,
    > {
        self.finished = true;
        let mfra = self
            .random_access
            .take()
            .map(RandomAccessTables::finish)
            .transpose()?;

        let sidx = self
            .segment_index
            .take()
            .map(SubsegmentTable::finish)
            .transpose()?;

        Ok((mfra, sidx))
    }
}

impl SampleTableWriter {
    /// Creates the writer of the sample tables of `movie`
    ///
    /// The writer fills the sample tables from the samples it takes, and the movie it holds has no `mvex`.
    ///
    /// # Errors
    ///
    /// * [`SampleTableNotEmpty`](MuxError::SampleTableNotEmpty): a sample
    ///   group, sub-sample, sample auxiliary information or shadow sync table
    ///   of a track declares samples, which the writer does not write.
    pub fn new(mut movie: MovieBox) -> Result<Self, MuxError> {
        take_sample_tables(&mut movie)?;
        let fragmented = movie.mvex.take().is_some();
        let tables = Some(movie.trak.iter().map(|_| TrackTable::default()).collect());

        Ok(Self {
            movie,
            tables,
            fragmented,
        })
    }

    /// Returns the movie the writer was made from, its sample tables those of the samples to come
    #[must_use]
    pub const fn movie(&self) -> &MovieBox {
        &self.movie
    }

    /// Adds the samples of `locations` to the sample tables
    ///
    /// Each run of samples of one track whose media data lies one after
    /// another is a chunk (§8.7.4), its chunk offset the file offset of its
    /// first sample (§8.7.5). The data reference entry of a sample is the one
    /// its sample entry names (§8.5.2), and the key a location carries is not
    /// read.
    ///
    /// A refused call leaves the writer as it was.
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
    ///   does not start where the one before it of its track ends, or the
    ///   first of a track without an edit list does not start at 0.
    /// * [`MediaTimeOutOfRange`](MuxError::MediaTimeOutOfRange): an edit of
    ///   the track of the first sample of a track starts before that sample.
    /// * [`SampleSizeOutOfRange`](MuxError::SampleSizeOutOfRange): the media
    ///   data of a sample is longer than a table holds.
    /// * [`AlreadyFinished`](MuxError::AlreadyFinished): the writer is
    ///   finished.
    pub fn handle_locations(&mut self, locations: &[SampleLocation]) -> Result<(), MuxError> {
        let tables = self.tables.as_mut().ok_or(MuxError::AlreadyFinished)?;
        let decoding_times: Vec<Option<u64>> = self
            .movie
            .trak
            .iter()
            .zip(tables.iter())
            .map(|(trak, table)| {
                let edited = !timeline::edits(trak).is_empty();
                table.decoding_end().or((!edited).then_some(0))
            })
            .collect();
        let runs = runs(&self.movie, locations, &decoding_times)?;
        for (index, run) in runs.iter().enumerate() {
            let earlier = runs.get(..index).unwrap_or_default();
            let first = decoding_times.get(run.track).copied().flatten().is_none()
                && !earlier.iter().any(|earlier| earlier.track == run.track);
            let edits = self.movie.trak.get(run.track).map(timeline::edits);
            let before = edits.unwrap_or_default().iter().find(|edit| {
                edit.media_time >= 0 && i128::from(edit.media_time) < i128::from(run.decoding_time)
            });
            if let Some(edit) = before.filter(|_| first) {
                return Err(MuxError::MediaTimeOutOfRange {
                    track_id: run.track_id,
                    media_time: edit.media_time,
                    decoding_time: run.decoding_time,
                });
            }
        }

        for run in runs {
            if let Some(table) = tables.get_mut(run.track) {
                table.push_chunk(run.sample_description_index, run.samples);
            }
        }

        Ok(())
    }

    /// Fills the sample tables of the movie with the samples written, and its durations with those they make up, and returns the movie
    ///
    /// A track's `mdhd` takes the duration of its samples, the longest
    /// `tkhd` the `mvhd` takes, and a track's `tkhd` (§8.2.2, §8.3.2,
    /// §8.4.2):
    ///
    /// * without an edit list, the duration of its samples in the timescale
    ///   of the movie;
    /// * with one that repeats, the duration the movie gives, up to which it
    ///   repeats;
    /// * otherwise the sum of its edit durations.
    ///
    /// The sample table counts decoding times from 0 at the first sample of
    /// a track (§8.6.1.2): the edits of a track whose first sample is at a
    /// later decoding time move their `media_time` back by as much. Where
    /// the movie given was fragmented, a last edit of duration 0 that is not
    /// empty, which lasts to the end of the media there (§8.6.6.1), takes the
    /// duration from its `media_time` to the latest composition time a sample
    /// is composed up to, rounded up to a tick of the movie, and the edit
    /// list, which is presented once there, does not repeat.
    ///
    /// The writer then takes no more samples, and a second call returns the
    /// same movie.
    pub fn finish(&mut self) -> &MovieBox {
        let Some(tables) = self.tables.take() else {
            return &self.movie;
        };
        let movie = &mut self.movie;
        let movie_timescale = u128::from(movie.mvhd.timescale);
        let mut movie_duration = 0_u64;
        for (trak, table) in movie.trak.iter_mut().zip(tables) {
            let media_duration = table.decoding_time();
            let media_timescale = u128::from(trak.mdia.mdhd.timescale);
            let in_movie = |duration: Option<u128>| {
                duration.map_or(0, |duration| u64::try_from(duration).unwrap_or(u64::MAX))
            };
            trak.mdia.mdhd.duration = media_duration;
            trak.mdia.mdhd.version = trak.mdia.mdhd.version.max(version_for(media_duration));
            let mut track_duration = in_movie(
                u128::from(media_duration)
                    .saturating_mul(movie_timescale)
                    .checked_div(media_timescale),
            );
            let decoding_start = table.decoding_start().unwrap_or_default();
            let elst = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut());
            if let Some(elst) = elst.filter(|elst| !elst.entries.is_empty()) {
                if decoding_start > 0 {
                    let shift = i64::try_from(decoding_start).unwrap_or(i64::MAX);
                    for edit in elst.entries.iter_mut().filter(|edit| edit.media_time >= 0) {
                        edit.media_time = edit.media_time.saturating_sub(shift);
                    }
                }
                let open = elst
                    .entries
                    .last_mut()
                    .filter(|edit| self.fragmented && timeline::lasts_to_end_of_media(edit));
                if let Some(edit) = open {
                    let media = table
                        .composition_end()
                        .saturating_sub(i128::from(edit.media_time));
                    let media = u128::try_from(media).unwrap_or_default();
                    let scaled = media.saturating_mul(movie_timescale);
                    edit.edit_duration =
                        in_movie((media_timescale > 0).then(|| scaled.div_ceil(media_timescale)));
                    elst.version = elst.version.max(version_for(edit.edit_duration));
                    elst.repeat_edits = false;
                }
                track_duration = if elst.repeat_edits {
                    trak.tkhd.duration
                } else {
                    elst.entries
                        .iter()
                        .map(|edit| edit.edit_duration)
                        .fold(0, u64::saturating_add)
                };
            }
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

    use isobmff_boxes::{
        FileTypeBox, MovieBox, MovieExtendsBox, MovieFragmentBox, SampleFlags, TrackBox,
        TrackExtendsBox,
    };
    use isobmff_core::{BoxDecode, BoxDefinition};

    use super::{MovieFragmentWriter, SampleTableWriter};
    use crate::error::MuxError;
    use crate::file_structure::FragmentedOptions;
    use crate::sample::{DataReferenceKey, Sample, SampleLocation, SampleProperties};
    use crate::structure_data::StructureDataReader;
    use crate::structure_data::fixtures::{
        UNLIMITED, encoded, in_another_file, laid_out, movie, track, written,
    };

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
        let mut writer = MovieFragmentWriter::new(two_fragmented_tracks(), 7).unwrap();
        let samples = [
            sample(1, 100, 10, b"AB"),
            sample(1, 110, 10, b"CDE"),
            offset_sample(2, 0, 20, b"FGHI", 5),
        ];

        let (moof, _) = writer.movie_fragment(&samples, 0, 8).unwrap();

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
                .map(|(moof, _)| moof.mfhd.sequence_number),
            Ok(8)
        );
    }

    #[test]
    fn the_locations_of_a_movie_fragment_are_those_a_reader_resolves_out_of_its_moof() {
        let mut writer = MovieFragmentWriter::new(two_fragmented_tracks(), 1).unwrap();
        let samples = [
            sample(1, 1 << 32, 10, b"AB"),
            sample(1, (1 << 32) + 10, 10, b"CDE"),
            offset_sample(2, 0, 20, b"FGHI", -5),
            sample(1, (1 << 32) + 20, 10, b"J"),
        ];

        let (moof, locations) = writer.movie_fragment(&samples, 1000, 16).unwrap();

        let mut reader = StructureDataReader::new(UNLIMITED);
        reader.handle_movie(writer.movie().clone()).unwrap();
        let moof = MovieFragmentBox::decode(&encoded(&moof)).unwrap().0;
        let read: Vec<SampleLocation> =
            reader.handle_movie_fragment(&moof, 1000).unwrap().collect();
        assert_eq!(locations, read);
    }

    #[test]
    fn a_finished_writer_takes_no_more_samples_and_finishes_as_it_did() {
        let mut tables = SampleTableWriter::new(two_tracks()).unwrap();
        tables
            .handle_locations(&laid_out(&[sample(1, 0, 10, b"AB")], 0))
            .unwrap();
        let movie = tables.finish().clone();
        let mut fragments = MovieFragmentWriter::fragmented(
            two_fragmented_tracks(),
            FragmentedOptions {
                first_sequence_number: 1,
                movie_fragment_random_access: true,
            },
        )
        .unwrap();
        fragments.finish().unwrap();

        assert_eq!(
            (
                tables.handle_locations(&laid_out(&[sample(1, 10, 10, b"CD")], 2)),
                tables.finish().clone(),
                fragments
                    .movie_fragment(&[sample(1, 0, 10, b"AB")], 0, 8)
                    .map(|_| ()),
                fragments.finish(),
            ),
            (
                Err(MuxError::AlreadyFinished),
                movie,
                Err(MuxError::AlreadyFinished),
                Ok((None, None)),
            )
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
            .handle_locations(&laid_out(
                &[
                    sample(1, 0, 10, b"AB"),
                    sample(1, 10, 10, b"CD"),
                    offset_sample(2, 0, 20, b"EFG", 5),
                ],
                1000,
            ))
            .unwrap();
        writer
            .handle_locations(&laid_out(&[sample(1, 20, 10, b"HI")], 2000))
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

    /// Returns the movie of `file`, which is its `ftyp` then its `moov`
    fn movie_of(file: &[u8]) -> MovieBox {
        let after_ftyp = FileTypeBox::decode(file).unwrap().1;
        MovieBox::decode(after_ftyp).unwrap().0
    }

    /// Returns the locations of the samples of `movie` in decoding order
    fn locations_of(movie: &MovieBox) -> Vec<SampleLocation> {
        StructureDataReader::new(UNLIMITED)
            .handle_movie(movie.clone())
            .unwrap()
            .into_decoding_order()
    }

    /// Files of a movie of one track, the media data of its chunks in decoding order and out of it
    const FILES: [&[u8]; 2] = [
        include_bytes!("../../tests/data/movie.mp4"),
        include_bytes!("../../tests/data/out_of_order.mp4"),
    ];

    #[test]
    fn the_moov_written_from_the_locations_of_samples_written_is_the_moov_written_from_the_samples()
    {
        for file in FILES {
            let movie = movie_of(file);
            let samples = locations_of(&movie).into_iter().map(|location| {
                let range = location.byte_range();
                let data = file.get(
                    usize::try_from(range.start).unwrap()..usize::try_from(range.end).unwrap(),
                );
                Sample::new(*location.properties(), data.unwrap().to_vec())
            });
            let mut muxer = crate::Muxer::non_fragmented(crate::NonFragmentedOptions::default());
            muxer.handle_movie(movie.clone()).unwrap();
            muxer.handle_samples(samples).unwrap();
            muxer.finish().unwrap();
            let written: Vec<Vec<u8>> = core::iter::from_fn(|| muxer.poll_output())
                .map(crate::output::Output::into_bytes)
                .collect();
            let written_moov = written.last().unwrap();

            let mut writer = SampleTableWriter::new(movie).unwrap();
            writer
                .handle_locations(&locations_of(&MovieBox::decode(written_moov).unwrap().0))
                .unwrap();

            assert_eq!(&encoded(writer.finish()), written_moov);
        }
    }

    #[test]
    fn the_moov_written_from_the_locations_of_a_file_declares_them_where_they_lie() {
        for file in FILES {
            let movie = movie_of(file);
            let locations = locations_of(&movie);

            let mut writer = SampleTableWriter::new(movie).unwrap();
            writer.handle_locations(&locations).unwrap();

            assert_eq!(locations_of(writer.finish()), locations);
        }
    }

    #[test]
    fn samples_of_a_track_whose_media_data_does_not_follow_on_lie_in_chunks_of_their_own() {
        let located = |track_id, decoding_time, byte_range| {
            SampleLocation::new(
                *sample(track_id, decoding_time, 10, b"").properties(),
                DataReferenceKey::new(0),
                byte_range,
            )
        };
        let mut writer = SampleTableWriter::new(two_tracks()).unwrap();
        writer
            .handle_locations(&[
                located(1, 0, 100..102),
                located(1, 10, 102..104),
                located(1, 20, 300..302),
                located(2, 0, 104..107),
                located(1, 30, 302..304),
            ])
            .unwrap();

        let movie = writer.finish();

        let chunks: Vec<_> = movie
            .trak
            .iter()
            .map(|trak| {
                let stbl = &trak.mdia.minf.stbl;
                (
                    stbl.stsc.as_ref().map(encoded),
                    stbl.chunk_offset.as_ref().map(|stco| match stco {
                        isobmff_boxes::ChunkOffsetVariant::ChunkOffset(stco) => encoded(stco),
                        isobmff_boxes::ChunkOffsetVariant::ChunkLargeOffset(co64) => encoded(co64),
                        _ => Vec::new(),
                    }),
                )
            })
            .collect();
        let stsc = |entries: &[[u32; 3]]| {
            let count = u32::try_from(entries.len()).unwrap().to_be_bytes();
            let fields: Vec<u8> = entries
                .iter()
                .flatten()
                .flat_map(|field| field.to_be_bytes())
                .collect();
            Some(full(b"stsc", &[&count, &fields]))
        };
        let stco = |offsets: &[u32]| {
            let count = u32::try_from(offsets.len()).unwrap().to_be_bytes();
            let fields: Vec<u8> = offsets
                .iter()
                .flat_map(|offset| offset.to_be_bytes())
                .collect();
            Some(full(b"stco", &[&count, &fields]))
        };
        assert_eq!(
            chunks,
            [
                (stsc(&[[1, 2, 1], [2, 1, 1]]), stco(&[100, 300, 302])),
                (stsc(&[[1, 1, 1]]), stco(&[104])),
            ]
        );
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
            .handle_locations(&laid_out(
                &[
                    flagged(sample(1, 0, 10, b"A"), 3, 0),
                    flagged(sample(1, 10, 10, b"B"), 0, 0),
                    flagged(sample(1, 20, 10, b"C"), 5, 0x102),
                    sample(2, 0, 10, b"D"),
                ],
                1000,
            ))
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
            writer.handle_locations(&laid_out(&[sample(3, 0, 10, b"AB")], 0)),
            writer.handle_locations(&laid_out(&[other_description.clone()], 0)),
            writer.handle_locations(&laid_out(&[sample(2, 0, 10, b"AB")], 0)),
            writer.handle_locations(&laid_out(
                &[
                    sample(1, 0, 10, b"AB"),
                    sample(1, 10, 10, b"CD"),
                    other_description,
                ],
                0,
            )),
            writer.handle_locations(&laid_out(&[sample(1, 5, 10, b"AB")], 0)),
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
    fn the_samples_of_a_track_with_an_edit_list_start_at_the_decoding_time_of_the_first() {
        let mut writer = SampleTableWriter::new(edited(&[(0, -1), (0, 8)])).unwrap();

        writer
            .handle_locations(&laid_out(&[sample(1, 5, 10, b"AB")], 0))
            .unwrap();
        let next = [
            writer.handle_locations(&laid_out(&[sample(1, 10, 10, b"CD")], 2)),
            writer.handle_locations(&laid_out(&[sample(1, 15, 10, b"CD")], 2)),
            writer.handle_locations(&laid_out(&[sample(2, 5, 10, b"EF")], 4)),
        ];

        assert_eq!(
            next,
            [
                Err(MuxError::DecodingTimeMismatch {
                    track_id: 1,
                    expected_decoding_time: 15,
                    decoding_time: 10
                }),
                Ok(()),
                Err(MuxError::DecodingTimeMismatch {
                    track_id: 2,
                    expected_decoding_time: 0,
                    decoding_time: 5
                }),
            ]
        );
        let trak = writer.finish().trak.first().unwrap();
        let media_times: Vec<_> = crate::timeline::edits(trak)
            .iter()
            .map(|edit| edit.media_time)
            .collect();
        assert_eq!(
            (trak.mdia.mdhd.duration, media_times),
            (20, Vec::from([-1, 3]))
        );
    }

    #[test]
    fn a_track_with_an_edit_before_its_first_sample_is_refused() {
        let mut writer = SampleTableWriter::new(edited(&[(0, 3)])).unwrap();

        assert_eq!(
            writer.handle_locations(&laid_out(&[sample(1, 5, 10, b"AB")], 0)),
            Err(MuxError::MediaTimeOutOfRange {
                track_id: 1,
                media_time: 3,
                decoding_time: 5
            })
        );
        assert_eq!(
            writer.handle_locations(&laid_out(
                &[
                    sample(1, 3, 10, b"AB"),
                    sample(2, 0, 10, b"CD"),
                    sample(1, 13, 10, b"EF")
                ],
                0
            )),
            Ok(())
        );
    }

    /// Returns the edit durations of the edit list of track 1, and the durations of the `tkhd` of each track and of the `mvhd`, of `movie` written with `samples`
    fn durations(movie: MovieBox, samples: &[Sample]) -> (Vec<u64>, Vec<u64>, u64) {
        let mut writer = SampleTableWriter::new(movie).unwrap();
        writer.handle_locations(&laid_out(samples, 0)).unwrap();
        let movie = writer.finish();
        let edits = movie
            .trak
            .first()
            .map(|trak| {
                crate::timeline::edits(trak)
                    .iter()
                    .map(|edit| edit.edit_duration)
                    .collect()
            })
            .unwrap_or_default();
        let tracks = movie.trak.iter().map(|trak| trak.tkhd.duration).collect();
        (edits, tracks, movie.mvhd.duration)
    }

    #[test]
    fn a_last_edit_lasting_to_the_end_of_the_media_of_a_fragmented_movie_ends_with_the_samples() {
        let fragmented = |media_timescale| {
            let mut movie = edited(&[(0, 8)]);
            movie.mvex = Some(MovieExtendsBox::default());
            if let Some(trak) = movie.trak.first_mut() {
                trak.mdia.mdhd.timescale = media_timescale;
            }
            movie
        };
        let samples = [
            sample(1, 0, 10, b"A"),
            sample(1, 10, 10, b"B"),
            sample(1, 20, 10, b"C"),
            sample(2, 0, 20, b"D"),
        ];

        assert_eq!(
            [
                durations(fragmented(100), &samples),
                durations(fragmented(300), &samples),
                durations(edited(&[(0, 8)]), &samples),
            ],
            [
                (Vec::from([220]), Vec::from([220, 200]), 220),
                (Vec::from([74]), Vec::from([74, 200]), 200),
                (Vec::from([0]), Vec::from([0, 200]), 200),
            ]
        );
    }

    #[test]
    fn a_last_edit_lasting_to_the_end_of_the_media_closes_where_the_last_sample_composed_ends() {
        let mut movie = edited(&[(0, 0)]);
        movie.mvex = Some(MovieExtendsBox::default());
        if let Some(trak) = movie.trak.first_mut() {
            trak.mdia.mdhd.timescale = 1000;
        }
        let mut late = sample(1, 0, 10, b"A");
        let mut properties = *late.properties();
        properties.sample_composition_time_offset = 5;
        late = Sample::new(properties, late.into_data());

        assert_eq!(
            durations(movie, &[late, sample(2, 0, 10, b"B")]),
            (Vec::from([15]), Vec::from([15, 100]), 100)
        );
    }

    #[test]
    fn a_repeated_edit_list_whose_last_edit_lasts_to_the_end_of_the_media_is_presented_once() {
        let mut movie = edited(&[(0, 0)]);
        movie.mvex = Some(MovieExtendsBox::default());
        if let Some(trak) = movie.trak.first_mut() {
            trak.tkhd.duration = 777;
            if let Some(elst) = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut()) {
                elst.repeat_edits = true;
            }
        }
        let mut writer = SampleTableWriter::new(movie).unwrap();
        writer
            .handle_locations(&laid_out(&[sample(1, 0, 10, b"A")], 0))
            .unwrap();

        let trak = writer.finish().trak.first().unwrap();
        let repeat_edits = trak
            .edts
            .as_ref()
            .and_then(|edts| edts.elst.as_ref())
            .map(|elst| elst.repeat_edits);
        assert_eq!((repeat_edits, trak.tkhd.duration), (Some(false), 100));
    }

    #[test]
    fn a_track_with_an_edit_list_lasts_as_long_as_its_edits_or_as_its_tkhd_states_where_they_repeat()
     {
        let samples = [sample(1, 0, 10, b"A"), sample(2, 0, 10, b"B")];
        let mut repeated = edited(&[(30, 0)]);
        if let Some(trak) = repeated.trak.first_mut() {
            trak.tkhd.duration = 777;
            if let Some(elst) = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut()) {
                elst.repeat_edits = true;
            }
        }

        assert_eq!(
            [
                durations(edited(&[(500, -1), (300, 0)]), &samples),
                durations(edited(&[(0, 0)]), &samples),
                durations(repeated, &samples),
            ],
            [
                (Vec::from([500, 300]), Vec::from([800, 100]), 800),
                (Vec::from([0]), Vec::from([0, 100]), 100),
                (Vec::from([30]), Vec::from([777, 100]), 777),
            ]
        );
    }

    /// Returns [`two_tracks`] with an edit list on track 1 of `edits` (edit duration, media time)
    fn edited(edits: &[(u64, i64)]) -> MovieBox {
        let mut movie = two_tracks();
        let mut elst = isobmff_boxes::EditListBox::default();
        elst.entries = edits
            .iter()
            .map(|&(edit_duration, media_time)| {
                let mut edit = isobmff_boxes::EditListEntry::default();
                edit.edit_duration = edit_duration;
                edit.media_time = media_time;
                edit.media_rate_integer = 1;
                edit
            })
            .collect();
        let mut edts = isobmff_boxes::EditBox::default();
        edts.elst = Some(elst);
        if let Some(trak) = movie.trak.first_mut() {
            trak.edts = Some(edts);
        }
        movie
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
            writer.handle_locations(&laid_out(&samples, 100))?;
            Ok::<_, MuxError>(writer.finish().clone())
        };
        let fragments_of = |mut movie: MovieBox| {
            movie.mvex = two_fragmented_tracks().mvex;
            MovieFragmentWriter::new(movie, 1).map(|writer| writer.movie().clone())
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
        let written_mvex =
            |movie| MovieFragmentWriter::new(movie, 1).map(|writer| writer.movie().mvex.clone());

        assert_eq!(
            [written_mvex(two_tracks()), written_mvex(given)],
            [Ok(two_fragmented_tracks().mvex), Ok(expected.mvex)]
        );
    }
}
