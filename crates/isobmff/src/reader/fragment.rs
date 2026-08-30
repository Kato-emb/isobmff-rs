//! [`MovieTracks`], the samples a movie fragment declares resolved against the movie

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{
    MovieBox, MovieFragmentBox, TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox,
};

use crate::error::SampleError;

/// Defaults one track sets for its fragments, and where its timeline stands
///
/// The defaults are the ones the `trex` of the track declares (ISO/IEC 14496-12
/// §8.8.3), which a `tfhd` overrides for one fragment and a `trun` row for one
/// sample. The decode time is the running one: where the samples resolved so
/// far leave the media timeline of this track, and so where a fragment carrying
/// no `tfdt` starts.
#[derive(Clone, Copy, Debug)]
struct Track {
    default_sample_description_index: u32,
    default_sample_duration: u32,
    default_sample_size: u32,
    default_sample_flags: u32,
    decode_time: u64,
}

/// What the samples of one track fragment fall back on
///
/// A `tfhd` states a default for the fragment where it carries one, and the
/// `trex` of the track stands in where it does not (ISO/IEC 14496-12 §8.8.7).
#[derive(Clone, Copy, Debug)]
struct Defaults {
    sample_description_index: u32,
    sample_duration: u32,
    sample_size: u32,
    sample_flags: u32,
}

impl Defaults {
    /// Returns what the samples of the fragment `tfhd` heads fall back on, ISO/IEC
    /// 14496-12 §8.8.7
    fn of(tfhd: &TrackFragmentHeaderBox, track: &Track) -> Self {
        Self {
            sample_description_index: tfhd
                .sample_description_index()
                .unwrap_or(track.default_sample_description_index),
            sample_duration: tfhd
                .default_sample_duration()
                .unwrap_or(track.default_sample_duration),
            sample_size: tfhd
                .default_sample_size()
                .unwrap_or(track.default_sample_size),
            sample_flags: tfhd
                .default_sample_flags()
                .unwrap_or(track.default_sample_flags),
        }
    }
}

/// Where the samples of a track fragment settle as its runs are walked
///
/// `base` is where the offsets of the track fragment are anchored, which the
/// offset a run states is counted from. `data_offset` is where the sample
/// resolved next starts, and `decode_time` when it is decoded.
#[derive(Clone, Copy, Debug)]
struct Cursor {
    base: u64,
    data_offset: u64,
    decode_time: u64,
}

/// Sample a fragment declared, resolved to what it states and where it lies
///
/// `extent` is the bytes of the presentation the sample was resolved to, which
/// the data of it is gathered from.
#[derive(Clone, Debug)]
pub(super) struct SettledSample {
    pub(super) track_id: u32,
    pub(super) decode_time: u64,
    pub(super) sample_duration: u32,
    pub(super) sample_composition_time_offset: Option<i64>,
    pub(super) sample_flags: u32,
    pub(super) sample_description_index: u32,
    pub(super) extent: Range<u64>,
}

impl SettledSample {
    /// Returns the bytes the sample was declared to occupy
    pub(super) fn declared_len(&self) -> u64 {
        self.extent.end.saturating_sub(self.extent.start)
    }
}

/// Tracks a movie declares, and what the fragments of each one are resolved against
///
/// The defaults a `trex` sets and the running decode time of every track are
/// held here, so a fragment is resolved into the samples it declares against
/// the fragments resolved before it.
#[derive(Clone, Debug)]
pub(super) struct MovieTracks {
    tracks: BTreeMap<u32, Track>,
}

impl MovieTracks {
    /// Reads off `movie` the defaults its fragments fall back on
    ///
    /// # Errors
    ///
    /// * [`MissingMovieExtends`](crate::SampleErrorKind::MissingMovieExtends):
    ///   the movie carries no `mvex`, and so continues in no fragments.
    pub(super) fn of(movie: &MovieBox) -> Result<Self, SampleError> {
        let Some(mvex) = movie.mvex() else {
            return Err(SampleError::missing_movie_extends());
        };

        // Why not scanning the `trex` per track: both counts follow from the
        // length of the `moov`, so the scan would cost the product of two figures
        // an input settles.
        let mut defaults = BTreeMap::new();
        for trex in mvex.trex() {
            defaults.entry(trex.track_id()).or_insert(trex);
        }

        let mut tracks = BTreeMap::new();
        for trak in movie.trak() {
            let track_id = trak.tkhd().track_id();
            // Why not reporting the track whose `trex` is missing: MovieBox
            // refuses a fragmented movie that leaves one without it, so what is
            // passed over here is a `trex` of a track the movie never declared.
            if let Some(trex) = defaults.get(&track_id) {
                tracks.entry(track_id).or_insert(Track {
                    default_sample_description_index: trex.default_sample_description_index(),
                    default_sample_duration: trex.default_sample_duration(),
                    default_sample_size: trex.default_sample_size(),
                    default_sample_flags: trex.default_sample_flags(),
                    decode_time: 0,
                });
            }
        }

        Ok(Self { tracks })
    }

    /// Resolves the samples `movie_fragment` declares, in the order it declares them
    ///
    /// `moof_start` is where the fragment begins, which the offsets of its track
    /// fragments are anchored against. The decode time each track is left at is
    /// kept, so the fragment resolved after this one carries on from it.
    ///
    /// A fragment declaring an empty duration carries no samples, and moves the
    /// timeline of its track on by the default duration alone (ISO/IEC 14496-12
    /// §8.8.7.1).
    ///
    /// # Errors
    ///
    /// * [`UnknownTrackId`](crate::SampleErrorKind::UnknownTrackId): a `traf`
    ///   carries samples of a track the movie never declared.
    /// * [`DataOffsetOverflow`](crate::SampleErrorKind::DataOffsetOverflow): the
    ///   offsets a fragment states run past what 64 bits carry.
    /// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow): the
    ///   decode times of a track run past what 64 bits carry.
    pub(super) fn settle(
        &mut self,
        movie_fragment: &MovieFragmentBox,
        moof_start: u64,
    ) -> Result<Vec<SettledSample>, SampleError> {
        let mut settled = Vec::new();
        let mut data_before = None;

        for traf in movie_fragment.traf() {
            let tfhd = traf.tfhd();
            let track_id = tfhd.track_id();
            let Some(track) = self.tracks.get(&track_id).copied() else {
                return Err(SampleError::unknown_track_id(track_id));
            };

            let defaults = Defaults::of(tfhd, &track);
            let base = base_data_offset(tfhd, moof_start, data_before);
            let mut cursor = Cursor {
                base,
                data_offset: base,
                decode_time: decode_time(traf, &track),
            };

            if tfhd.duration_is_empty() {
                cursor.decode_time = cursor
                    .decode_time
                    .checked_add(u64::from(defaults.sample_duration))
                    .ok_or(SampleError::decode_time_overflow(track_id))?;
            }

            for trun in traf.trun() {
                settle_run(trun, track_id, &defaults, &mut cursor, &mut settled)?;
            }

            self.tracks.insert(
                track_id,
                Track {
                    decode_time: cursor.decode_time,
                    ..track
                },
            );
            data_before = Some(cursor.data_offset);
        }

        Ok(settled)
    }
}

/// Returns where the offsets of the track fragment `tfhd` heads are anchored, ISO/IEC
/// 14496-12 §8.8.7.1
///
/// A `tfhd` stating a `base_data_offset` is anchored there, one setting
/// `default-base-is-moof` at the fragment itself, and one stating neither at the
/// fragment for the first track fragment and at the end of the data of the one
/// before it for those that follow — which `data_before` names.
fn base_data_offset(
    tfhd: &TrackFragmentHeaderBox,
    moof_start: u64,
    data_before: Option<u64>,
) -> u64 {
    if let Some(explicit) = tfhd.base_data_offset() {
        explicit
    } else if tfhd.default_base_is_moof() {
        moof_start
    } else {
        data_before.unwrap_or(moof_start)
    }
}

/// Returns where `traf` places its track on the media timeline, ISO/IEC 14496-12
/// §8.8.12
///
/// A `tfdt` states the decode time of the first sample of the track fragment
/// absolutely. Where the fragment carries none, the track carries on from where
/// the samples resolved before it left it.
fn decode_time(traf: &TrackFragmentBox, track: &Track) -> u64 {
    traf.tfdt()
        .map_or(track.decode_time, |tfdt| tfdt.base_media_decode_time())
}

/// Resolves the samples `trun` declares, and moves `cursor` past them, ISO/IEC
/// 14496-12 §8.8.8
///
/// A run states where it starts as an offset from the base the cursor is
/// anchored at, and one stating none starts where the run before it ended. The flags of the first sample of the
/// run are stated by the run itself where it carries them, and every further
/// field a row leaves out is taken from `defaults`.
///
/// # Errors
///
/// * [`DataOffsetOverflow`](crate::SampleErrorKind::DataOffsetOverflow): the
///   offsets the run states run past what 64 bits carry.
/// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow): the
///   decode times of the track run past what 64 bits carry.
fn settle_run(
    trun: &TrackRunBox,
    track_id: u32,
    defaults: &Defaults,
    cursor: &mut Cursor,
    settled: &mut Vec<SettledSample>,
) -> Result<(), SampleError> {
    if let Some(stated) = trun.data_offset() {
        cursor.data_offset = cursor
            .base
            .checked_add_signed(i64::from(stated))
            .ok_or(SampleError::data_offset_overflow(track_id))?;
    }

    let mut first_sample_flags = trun.first_sample_flags();
    for row in trun.samples() {
        let declared = u64::from(row.sample_size().unwrap_or(defaults.sample_size));
        let data_end = cursor
            .data_offset
            .checked_add(declared)
            .ok_or(SampleError::data_offset_overflow(track_id))?;
        let sample_duration = row.sample_duration().unwrap_or(defaults.sample_duration);

        settled.push(SettledSample {
            track_id,
            decode_time: cursor.decode_time,
            sample_duration,
            sample_composition_time_offset: row.sample_composition_time_offset(),
            sample_flags: first_sample_flags
                .take()
                .or(row.sample_flags())
                .unwrap_or(defaults.sample_flags),
            sample_description_index: defaults.sample_description_index,
            extent: cursor.data_offset..data_end,
        });

        cursor.decode_time = cursor
            .decode_time
            .checked_add(u64::from(sample_duration))
            .ok_or(SampleError::decode_time_overflow(track_id))?;
        cursor.data_offset = data_end;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, TrackExtendsBox,
        TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox,
        TrackRunSample,
    };
    use isobmff_core::FullBoxFlags;

    use crate::error::SampleError;
    use crate::reader::SampleReader;
    use crate::reader::tests::{
        MOVIE_FRAGMENT_LEN, drained, movie, movie_fragment, movie_of_one_track_id_twice,
        one_sample_movie_fragment, one_track_movie, read_one_fragment, run, sample, track_fragment,
        track_fragment_header, two_track_movie,
    };
    use crate::sample::Sample;

    #[test]
    fn a_sample_takes_what_its_row_states_over_the_defaults_of_the_fragment_and_the_track() {
        let rows =
            vec![TrackRunSample::new(Some(512), Some(2), Some(0x0100_0000), Some(-8)).unwrap()];
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(256),
                Some(8),
            ),
            None,
            vec![TrackRunBox::new(Some(100), None, rows).unwrap()],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"AB", 100..102).unwrap();

        assert_eq!(
            drained(&mut reader),
            [Sample::new(
                1,
                0,
                512,
                Some(-8),
                0x0100_0000,
                1,
                b"AB".to_vec()
            )]
        );
    }

    #[test]
    fn a_sample_takes_what_its_fragment_states_over_the_defaults_of_its_track() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(256),
                Some(2),
            ),
            None,
            vec![run(Some(100), 2)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(
            drained(&mut reader),
            [
                Sample::new(1, 0, 256, None, 0, 1, b"AB".to_vec()),
                Sample::new(1, 256, 256, None, 0, 1, b"CD".to_vec()),
            ]
        );
    }

    #[test]
    fn the_flags_of_the_first_sample_of_a_run_stand_in_for_the_defaults() {
        let rows = vec![
            TrackRunSample::new(None, None, None, None).unwrap(),
            TrackRunSample::new(None, None, None, None).unwrap(),
        ];
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                None,
            ),
            None,
            vec![TrackRunBox::new(Some(100), Some(0x0200_0000), rows).unwrap()],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCDEFGH", 100..108).unwrap();

        assert_eq!(
            drained(&mut reader),
            [
                Sample::new(1, 0, 1_024, None, 0x0200_0000, 1, b"ABCD".to_vec()),
                sample(1_024, b"EFGH"),
            ]
        );
    }

    #[test]
    fn a_track_starts_at_zero_where_its_first_fragment_states_no_decode_time() {
        assert_eq!(read_one_fragment(1, b"ABCD"), [sample(0, b"ABCD")]);
    }

    #[test]
    fn samples_are_placed_by_the_durations_of_the_samples_before_them() {
        assert_eq!(
            read_one_fragment(2, b"ABCDEFGH"),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
    }

    #[test]
    fn a_fragment_stating_no_decode_time_carries_on_from_the_one_before_it() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();

        for (fragment_start, data_start) in [(0, 100), (200, 300)] {
            reader
                .handle_movie_fragment(
                    one_sample_movie_fragment(),
                    fragment_start..fragment_start + 100,
                )
                .unwrap();
            reader
                .handle_media_data(b"ABCD", data_start..data_start + 4)
                .unwrap();
        }

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"ABCD")]
        );
    }

    #[test]
    fn a_decode_time_is_taken_as_stated_however_the_durations_read_so_far_sum() {
        let stated = |base_media_decode_time| {
            TrackFragmentBox::new(
                track_fragment_header(
                    TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                    1,
                    None,
                    None,
                    None,
                ),
                Some(TrackFragmentBaseMediaDecodeTimeBox::new(
                    base_media_decode_time,
                )),
                vec![run(Some(100), 1)],
            )
            .unwrap()
        };

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        for (fragment_start, data_start, decode_time) in [(0, 100, 90_000), (200, 300, 4_096)] {
            reader
                .handle_movie_fragment(
                    movie_fragment(vec![stated(decode_time)]),
                    fragment_start..fragment_start + 100,
                )
                .unwrap();
            reader
                .handle_media_data(b"ABCD", data_start..data_start + 4)
                .unwrap();
        }

        assert_eq!(
            drained(&mut reader),
            [sample(90_000, b"ABCD"), sample(4_096, b"ABCD")]
        );
    }

    #[test]
    fn offsets_are_anchored_at_the_base_the_fragment_states() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1, Some(400), None, None),
            None,
            vec![run(Some(8), 1)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 408..412).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn offsets_of_a_fragment_stating_no_anchor_at_all_are_anchored_at_the_movie_fragment() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1, None, None, None),
            None,
            vec![run(Some(100), 1)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn offsets_of_a_later_track_fragment_stating_no_anchor_follow_the_data_before_it() {
        let stating_no_anchor = |track_id, data_offset| {
            TrackFragmentBox::new(
                track_fragment_header(FullBoxFlags::ZERO, track_id, None, None, None),
                None,
                vec![run(data_offset, 1)],
            )
            .unwrap()
        };
        let two_tracks = two_track_movie();

        let mut reader = SampleReader::new(&two_tracks).unwrap();
        reader
            .handle_movie_fragment(
                movie_fragment(vec![
                    stating_no_anchor(1, Some(100)),
                    stating_no_anchor(2, None),
                ]),
                0..MOVIE_FRAGMENT_LEN,
            )
            .unwrap();
        reader.handle_media_data(b"ABCDEFGH", 100..108).unwrap();

        assert_eq!(
            drained(&mut reader),
            [
                sample(0, b"ABCD"),
                Sample::new(2, 0, 1_024, None, 0, 1, b"EFGH".to_vec()),
            ]
        );
    }

    #[test]
    fn a_run_stating_no_offset_starts_where_the_run_before_it_ended() {
        let track_fragment = track_fragment(vec![run(Some(100), 1), run(None, 1)]);

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCDEFGH", 100..108).unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
    }

    #[test]
    fn a_movie_carrying_no_extends_box_is_not_fragmented_at_all() {
        assert_eq!(
            SampleReader::new(&movie(&[1], vec![])).unwrap_err(),
            SampleError::missing_movie_extends()
        );
    }

    #[test]
    fn an_extends_box_for_a_track_the_movie_never_declared_is_passed_over() {
        let with_a_spare = movie(
            &[1],
            vec![
                TrackExtendsBox::new(1, 1, 1_024, 4, 0),
                TrackExtendsBox::new(7, 1, 1_024, 4, 0),
            ],
        );

        let mut reader = SampleReader::new(&with_a_spare).unwrap();
        let of_the_spare = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                7,
                None,
                None,
                None,
            ),
            None,
            vec![run(Some(100), 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(movie_fragment(vec![of_the_spare]), 0..MOVIE_FRAGMENT_LEN),
            Err(SampleError::unknown_track_id(7))
        );
    }

    #[test]
    fn two_tracks_declaring_one_id_leave_the_first_of_them_standing() {
        let mut reader = SampleReader::new(&movie_of_one_track_id_twice()).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn a_fragment_of_a_track_the_movie_never_declared_is_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let of_an_unknown_track = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                3,
                None,
                None,
                None,
            ),
            None,
            vec![run(Some(100), 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(
                movie_fragment(vec![of_an_unknown_track]),
                0..MOVIE_FRAGMENT_LEN
            ),
            Err(SampleError::unknown_track_id(3))
        );
    }

    #[test]
    fn an_empty_duration_moves_the_timeline_on_without_a_sample() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let empty = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DURATION_IS_EMPTY,
                1,
                None,
                Some(4_096),
                None,
            ),
            None,
            vec![],
        )
        .unwrap();

        reader
            .handle_movie_fragment(movie_fragment(vec![empty]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 100..200)
            .unwrap();
        reader.handle_media_data(b"ABCD", 200..204).unwrap();

        assert_eq!(drained(&mut reader), [sample(4_096, b"ABCD")]);
    }

    #[test]
    fn the_sequence_number_a_fragment_carries_is_neither_checked_nor_reported() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let out_of_order = MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(9),
            vec![track_fragment(vec![run(Some(100), 1)])],
        );

        reader
            .handle_movie_fragment(out_of_order, 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn decode_times_running_past_what_64_bits_carry_are_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let at_the_end_of_time = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(u32::MAX),
                None,
            ),
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(u64::MAX)),
            vec![run(Some(100), 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(
                movie_fragment(vec![at_the_end_of_time]),
                0..MOVIE_FRAGMENT_LEN
            ),
            Err(SampleError::decode_time_overflow(1))
        );
    }

    #[test]
    fn data_offsets_running_past_what_64_bits_carry_are_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let past_the_end_of_the_file = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1, Some(u64::MAX), None, None),
            None,
            vec![run(None, 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(
                movie_fragment(vec![past_the_end_of_the_file]),
                0..MOVIE_FRAGMENT_LEN
            ),
            Err(SampleError::data_offset_overflow(1))
        );
    }
}
