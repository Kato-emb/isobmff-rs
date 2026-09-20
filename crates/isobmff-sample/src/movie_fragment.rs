//! [`sample_extents`], the samples a movie fragment declares resolved against the movie, ISO/IEC 14496-12 §8.8

use alloc::vec::Vec;

use isobmff_boxes::{
    CompositionTimeOffset, MovieBox, MovieFragmentBox, TrackFragmentBox, TrackRunBox,
};

use crate::error::SampleError;
use crate::sample::SampleExtent;
use crate::sample_description::SampleDescriptions;
use crate::track_decode_times::TrackDecodeTimes;

/// Resolves the samples `movie_fragment` declares against `movie`, in the order it declares them
///
/// Each track fragment of the `moof` is resolved by the three-step fallback of
/// ISO/IEC 14496-12 §8.8.7 and §8.8.8: what a `trun` row states, then what
/// the `tfhd` of the fragment states, then what the `trex` of the track states.
/// Where the data of a run lies follows §8.8.7.1: a `tfhd` stating a
/// `base_data_offset` anchors its runs there, one setting
/// `default-base-is-moof` anchors them at `moof_start`, where the `moof`
/// begins in the file, and one stating neither anchors them at the `moof` for
/// the first track fragment and at the end of the data of the track fragment
/// before it for those that follow. A run stating no `data_offset` starts
/// where the run before it ended, and the first run of a track fragment at
/// its anchor.
///
/// When the samples are decoded follows §8.8.12: a track fragment carrying a
/// `tfdt` starts its samples there, and one carrying none carries on from where
/// `decode_times` has its track. Before the extents are returned,
/// `decode_times` is moved to where every track fragment leaves its track —
/// the end of its last sample, or the default duration on from where it
/// started for a fragment declaring an empty duration, which carries no
/// samples (§8.8.7.1) — so the fragment resolved next carries on from there
/// whether or not the extents are taken. A failure returned outright leaves
/// `decode_times` as it was. A row stating no composition time offset has one
/// of zero. The `data_reference_index` of each sample is read off the `stsd`
/// entry that describes it (§8.5.2.3), which has to name the file itself.
///
/// The extents come out in the order the fragment declares them, and stop at
/// the first failure, which is the last item. They borrow nothing: the boxes
/// and `decode_times` are the caller's again once the call returns.
///
/// # Errors
///
/// Returned outright, before `decode_times` moves:
///
/// * [`MissingMovieExtends`](crate::SampleErrorKind::MissingMovieExtends): a
///   `traf` continues a movie that carries no `mvex`, and so no fragments.
/// * [`UnknownTrackId`](crate::SampleErrorKind::UnknownTrackId): a `traf`
///   carries samples of a track the movie declares no `trak` or `trex` for.
/// * [`UnknownSampleDescriptionIndex`](crate::SampleErrorKind::UnknownSampleDescriptionIndex):
///   a `traf` describes its samples by an `stsd` entry its track has none of.
/// * The failures of [`SampleEntry::try_from`](isobmff_boxes::SampleEntry),
///   carried on [`Box`](crate::SampleErrorKind::Box): the `stsd` entry does
///   not read as a sample entry, with `stsd` added to the containers.
/// * [`UnknownDataReferenceIndex`](crate::SampleErrorKind::UnknownDataReferenceIndex):
///   the `stsd` entry names a `dref` entry its track has none of.
/// * [`ExternalDataReference`](crate::SampleErrorKind::ExternalDataReference):
///   the `dref` entry names a resource other than the file itself.
/// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow): the
///   decode times of a track run past what 64 bits carry.
///
/// Returned as the last of the extents:
///
/// * [`DataOffsetOverflow`](crate::SampleErrorKind::DataOffsetOverflow): the
///   offsets a fragment states run past what 64 bits carry.
pub fn sample_extents(
    movie_fragment: &MovieFragmentBox,
    movie: &MovieBox,
    moof_start: u64,
    decode_times: &mut TrackDecodeTimes,
) -> Result<impl Iterator<Item = Result<SampleExtent, SampleError>> + use<>, SampleError> {
    let mut reached = decode_times.clone();
    let track_fragments = movie_fragment
        .traf()
        .iter()
        .map(|traf| TrackFragment::settle(traf, movie, &mut reached))
        .collect::<Result<Vec<_>, _>>()?;
    *decode_times = reached;

    let mut extents = Vec::new();
    let outcome = resolve_data(movie_fragment, &track_fragments, moof_start, &mut extents);

    Ok(extents.into_iter().map(Ok).chain(outcome.err().map(Err)))
}

/// What one track fragment settles for its samples before their data is placed
///
/// The defaults are what a `tfhd` states for the fragment where it carries one
/// and the `trex` of the track states where it does not (ISO/IEC 14496-12
/// §8.8.7); `decode_time` is when the first sample of the fragment is decoded.
struct TrackFragment {
    track_id: u32,
    sample_description_index: u32,
    sample_duration: u32,
    sample_size: u32,
    sample_flags: u32,
    data_reference_index: u16,
    decode_time: u64,
}

impl TrackFragment {
    /// Settles `traf` against `movie`, and moves `reached` past its samples, ISO/IEC 14496-12 §8.8.12
    fn settle(
        traf: &TrackFragmentBox,
        movie: &MovieBox,
        reached: &mut TrackDecodeTimes,
    ) -> Result<Self, SampleError> {
        let Some(mvex) = movie.mvex() else {
            return Err(SampleError::missing_movie_extends());
        };
        let tfhd = traf.tfhd();
        let track_id = tfhd.track_id();
        let trak = movie
            .trak()
            .iter()
            .find(|trak| trak.tkhd().track_id() == track_id);
        let trex = mvex.trex().iter().find(|trex| trex.track_id() == track_id);
        let (Some(trak), Some(trex)) = (trak, trex) else {
            return Err(SampleError::unknown_track_id(track_id));
        };

        let sample_description_index = tfhd
            .sample_description_index()
            .unwrap_or(trex.default_sample_description_index());
        let sample_duration = tfhd
            .default_sample_duration()
            .unwrap_or(trex.default_sample_duration());
        let data_reference_index =
            SampleDescriptions::new(trak).data_reference_index(sample_description_index)?;
        let decode_time = traf.tfdt().map_or(reached.decode_time(track_id), |tfdt| {
            tfdt.base_media_decode_time()
        });

        let overflow = || SampleError::decode_time_overflow(track_id);
        let mut end = decode_time;
        if tfhd.duration_is_empty() {
            end = end
                .checked_add(u64::from(sample_duration))
                .ok_or_else(overflow)?;
        }
        for row in traf.trun().iter().flat_map(TrackRunBox::samples) {
            let lasts = u64::from(row.sample_duration().unwrap_or(sample_duration));
            end = end.checked_add(lasts).ok_or_else(overflow)?;
        }
        reached.reach(track_id, end);

        Ok(Self {
            track_id,
            sample_description_index,
            sample_duration,
            sample_size: tfhd
                .default_sample_size()
                .unwrap_or(trex.default_sample_size()),
            sample_flags: tfhd
                .default_sample_flags()
                .unwrap_or(trex.default_sample_flags()),
            data_reference_index,
            decode_time,
        })
    }
}

/// Where the samples of a track fragment settle as its runs are walked
///
/// `base` is where the offsets of the track fragment are anchored, which the
/// offset a run states is counted from. `data_offset` is where the sample
/// resolved next starts, and `decode_time` when it is decoded.
struct Cursor {
    base: u64,
    data_offset: u64,
    decode_time: u64,
}

/// Places the data of every track fragment of `movie_fragment` into `extents`, stopping at the first failure
fn resolve_data(
    movie_fragment: &MovieFragmentBox,
    track_fragments: &[TrackFragment],
    moof_start: u64,
    extents: &mut Vec<SampleExtent>,
) -> Result<(), SampleError> {
    let mut data_before = None;

    for (traf, settled) in movie_fragment.traf().iter().zip(track_fragments) {
        let tfhd = traf.tfhd();
        let base = tfhd
            .base_data_offset()
            .unwrap_or(if tfhd.default_base_is_moof() {
                moof_start
            } else {
                data_before.unwrap_or(moof_start)
            });
        let mut cursor = Cursor {
            base,
            data_offset: base,
            decode_time: settled.decode_time,
        };

        for trun in traf.trun() {
            resolve_run(trun, settled, &mut cursor, extents)?;
        }

        data_before = Some(cursor.data_offset);
    }

    Ok(())
}

/// Resolves the samples `trun` declares into `extents`, and moves `cursor` past them
fn resolve_run(
    trun: &TrackRunBox,
    settled: &TrackFragment,
    cursor: &mut Cursor,
    extents: &mut Vec<SampleExtent>,
) -> Result<(), SampleError> {
    let track_id = settled.track_id;
    if let Some(stated) = trun.data_offset() {
        cursor.data_offset = cursor
            .base
            .checked_add_signed(i64::from(stated))
            .ok_or(SampleError::data_offset_overflow(track_id))?;
    }

    let mut first_sample_flags = trun.first_sample_flags();
    for row in trun.samples() {
        let declared = u64::from(row.sample_size().unwrap_or(settled.sample_size));
        let data_end = cursor
            .data_offset
            .checked_add(declared)
            .ok_or(SampleError::data_offset_overflow(track_id))?;
        let sample_duration = row.sample_duration().unwrap_or(settled.sample_duration);

        extents.push(SampleExtent::new(
            track_id,
            cursor.decode_time,
            sample_duration,
            row.sample_composition_time_offset()
                .map_or(0, CompositionTimeOffset::get),
            first_sample_flags
                .take()
                .or(row.sample_flags())
                .unwrap_or(settled.sample_flags),
            settled.sample_description_index,
            settled.data_reference_index,
            cursor.data_offset..data_end,
        ));

        // Why not checked_add: TrackFragment::settle summed these same durations
        // from the same start and refused the fragment on overflow, so this
        // cannot wrap.
        cursor.decode_time = cursor.decode_time.wrapping_add(u64::from(sample_duration));
        cursor.data_offset = data_end;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::{
        CompositionTimeOffset, MovieBox, MovieExtendsBox, MovieFragmentBox, MovieFragmentHeaderBox,
        MovieHeaderBox, TrackBox, TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox,
        TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox,
        TrackRunSample,
    };
    use isobmff_core::{BoxDecode as _, BoxEncode as _, Mp4EpochSeconds};
    use isobmff_test_support::{
        external_data_reference, fragmented_movie, track, track_reading_from, unfragmented_movie,
        written,
    };

    use super::sample_extents;
    use crate::error::SampleError;
    use crate::sample::SampleExtent;
    use crate::track_decode_times::TrackDecodeTimes;

    /// Movie of the given tracks, each fragmented with samples that last 1024 units and occupy 4 bytes
    fn movie(trak: Vec<TrackBox>) -> MovieBox {
        let trex = trak
            .iter()
            .map(|trak| TrackExtendsBox::new(trak.tkhd().track_id(), 1, 1_024, 4, 0))
            .collect();

        MovieBox::new(
            MovieHeaderBox::new(
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                1_000,
                0,
                2,
            ),
            trak,
            MovieExtendsBox::new(trex),
        )
        .unwrap()
    }

    /// Movie of one track whose samples last 1024 units and occupy 4 bytes each
    fn one_track_movie() -> MovieBox {
        fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 4, 0))
    }

    /// Fragment header of one track, carrying the flags and defaults given
    fn track_fragment_header(
        flags: TrackFragmentHeaderFlags,
        track_id: u32,
        base_data_offset: Option<u64>,
        default_sample_duration: Option<u32>,
        default_sample_size: Option<u32>,
    ) -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox::new(
            flags,
            track_id,
            base_data_offset,
            None,
            default_sample_duration,
            default_sample_size,
            None,
        )
    }

    /// Run of samples that take the size and duration of their defaults
    fn run(data_offset: Option<i32>, sample_count: u32) -> TrackRunBox {
        let rows = (0..sample_count)
            .map(|_| TrackRunSample::new(None, None, None, None))
            .collect();

        TrackRunBox::new(data_offset, None, rows).unwrap()
    }

    /// Fragment of `track_id`, anchored at the movie fragment, of the runs given
    fn track_fragment(track_id: u32, trun: Vec<TrackRunBox>) -> TrackFragmentBox {
        TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                track_id,
                None,
                None,
                None,
            ),
            None,
            trun,
        )
    }

    /// Movie fragment carrying the given track fragments
    fn movie_fragment(traf: Vec<TrackFragmentBox>) -> MovieFragmentBox {
        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), traf)
    }

    /// Movie fragment claiming one four-byte sample of track 1, just past itself
    fn one_sample_movie_fragment() -> MovieFragmentBox {
        movie_fragment(vec![track_fragment(1, vec![run(Some(100), 1)])])
    }

    /// Extent of a sample of `track_id` as the defaults of the movies here settle it
    fn extent(track_id: u32, decode_time: u64, data: Range<u64>) -> SampleExtent {
        SampleExtent::new(track_id, decode_time, 1_024, 0, 0, 1, 1, data)
    }

    /// Resolves `movie_fragment` at the start of the file against a movie no fragment was resolved for
    fn resolved(
        movie_fragment: &MovieFragmentBox,
        movie: &MovieBox,
    ) -> Result<Vec<SampleExtent>, SampleError> {
        resolved_from(movie_fragment, movie, 0, &mut TrackDecodeTimes::new())
    }

    /// Resolves `movie_fragment` at `moof_start` against a movie whose tracks stand at `decode_times`
    fn resolved_from(
        movie_fragment: &MovieFragmentBox,
        movie: &MovieBox,
        moof_start: u64,
        decode_times: &mut TrackDecodeTimes,
    ) -> Result<Vec<SampleExtent>, SampleError> {
        sample_extents(movie_fragment, movie, moof_start, decode_times)?.collect()
    }

    /// Times of a movie whose track 1 stands at `decode_time`
    fn track_1_at(decode_time: u64) -> TrackDecodeTimes {
        let mut decode_times = TrackDecodeTimes::new();
        decode_times.reach(1, decode_time);

        decode_times
    }

    #[test]
    fn a_sample_takes_what_its_row_states_over_the_defaults_of_the_fragment_and_the_track() {
        let rows = vec![TrackRunSample::new(
            Some(512),
            Some(2),
            Some(0x0100_0000),
            Some(CompositionTimeOffset::new(-8).unwrap()),
        )];
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(256),
                Some(8),
            ),
            None,
            vec![TrackRunBox::new(Some(100), None, rows).unwrap()],
        );

        assert_eq!(
            resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
            Ok(vec![SampleExtent::new(
                1,
                0,
                512,
                -8,
                0x0100_0000,
                1,
                1,
                100..102
            )])
        );
    }

    #[test]
    fn a_sample_takes_what_its_fragment_states_over_the_defaults_of_its_track() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(256),
                Some(2),
            ),
            None,
            vec![run(Some(100), 2)],
        );

        assert_eq!(
            resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
            Ok(vec![
                SampleExtent::new(1, 0, 256, 0, 0, 1, 1, 100..102),
                SampleExtent::new(1, 256, 256, 0, 0, 1, 1, 102..104),
            ])
        );
    }

    #[test]
    fn the_flags_of_the_first_sample_of_a_run_stand_in_for_the_defaults() {
        let rows = vec![
            TrackRunSample::new(None, None, None, None),
            TrackRunSample::new(None, None, None, None),
        ];
        let track_fragment = track_fragment(
            1,
            vec![TrackRunBox::new(Some(100), Some(0x0200_0000), rows).unwrap()],
        );

        assert_eq!(
            resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
            Ok(vec![
                SampleExtent::new(1, 0, 1_024, 0, 0x0200_0000, 1, 1, 100..104),
                extent(1, 1_024, 104..108),
            ])
        );
    }

    #[test]
    fn a_fragment_stating_no_decode_time_carries_on_from_where_the_track_stands() {
        assert_eq!(
            resolved_from(
                &one_sample_movie_fragment(),
                &one_track_movie(),
                200,
                &mut track_1_at(5_120)
            ),
            Ok(vec![extent(1, 5_120, 300..304)])
        );
    }

    #[test]
    fn a_decode_time_is_taken_as_stated_however_the_track_stands() {
        let stated = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                None,
            ),
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(4_096)),
            vec![run(Some(100), 1)],
        );
        assert_eq!(
            resolved_from(
                &movie_fragment(vec![stated]),
                &one_track_movie(),
                0,
                &mut track_1_at(91_024)
            ),
            Ok(vec![extent(1, 4_096, 100..104)])
        );
    }

    #[test]
    fn offsets_are_anchored_at_the_base_the_fragment_states() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(TrackFragmentHeaderFlags::ZERO, 1, Some(400), None, None),
            None,
            vec![run(Some(8), 1)],
        );

        assert_eq!(
            resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
            Ok(vec![extent(1, 0, 408..412)])
        );
    }

    #[test]
    fn offsets_anchored_at_the_movie_fragment_count_from_where_it_lies() {
        assert_eq!(
            resolved_from(
                &one_sample_movie_fragment(),
                &one_track_movie(),
                1_000,
                &mut TrackDecodeTimes::new()
            ),
            Ok(vec![extent(1, 0, 1_100..1_104)])
        );
    }

    #[test]
    fn offsets_of_a_fragment_stating_no_anchor_at_all_are_anchored_at_the_movie_fragment() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(TrackFragmentHeaderFlags::ZERO, 1, None, None, None),
            None,
            vec![run(Some(100), 1)],
        );

        assert_eq!(
            resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
            Ok(vec![extent(1, 0, 100..104)])
        );
    }

    #[test]
    fn offsets_of_a_later_track_fragment_stating_no_anchor_follow_the_data_before_it() {
        let stating_no_anchor = |track_id, data_offset| {
            TrackFragmentBox::new(
                track_fragment_header(TrackFragmentHeaderFlags::ZERO, track_id, None, None, None),
                None,
                vec![run(data_offset, 1)],
            )
        };

        assert_eq!(
            resolved(
                &movie_fragment(vec![
                    stating_no_anchor(1, Some(100)),
                    stating_no_anchor(2, None),
                ]),
                &movie(vec![track(1), track(2)])
            ),
            Ok(vec![extent(1, 0, 100..104), extent(2, 0, 104..108)])
        );
    }

    #[test]
    fn a_run_stating_no_offset_starts_where_the_run_before_it_ended() {
        let two_runs = movie_fragment(vec![track_fragment(
            1,
            vec![run(Some(100), 1), run(None, 1)],
        )]);

        assert_eq!(
            resolved(&two_runs, &one_track_movie()),
            Ok(vec![extent(1, 0, 100..104), extent(1, 1_024, 104..108)])
        );
    }

    #[test]
    fn a_movie_carrying_no_extends_box_is_not_fragmented_at_all() {
        assert_eq!(
            resolved(&one_sample_movie_fragment(), &unfragmented_movie()),
            Err(SampleError::missing_movie_extends())
        );
    }

    #[test]
    fn a_fragment_of_a_track_the_movie_never_declared_is_refused() {
        let of_an_unknown_track = movie_fragment(vec![track_fragment(3, vec![run(Some(100), 1)])]);

        assert_eq!(
            resolved(&of_an_unknown_track, &one_track_movie()),
            Err(SampleError::unknown_track_id(3))
        );
    }

    #[test]
    fn two_tracks_declaring_one_track_id_leave_the_first_of_them_standing() {
        // Why not MovieBox::new: it refuses a movie declaring one track_id twice,
        // so the resolver only ever meets the collision through a decode.
        let declared = one_track_movie();
        let mut payload = vec![0; usize::try_from(declared.payload_len()).unwrap()];
        declared.encode_payload(&mut payload).unwrap();
        let colliding = MovieBox::decode_payload(&[payload, written(&track(1))].concat()).unwrap();

        assert_eq!(
            resolved(&one_sample_movie_fragment(), &colliding),
            Ok(vec![extent(1, 0, 100..104)])
        );
    }

    #[test]
    fn a_fragment_described_by_an_entry_its_track_has_none_of_is_refused() {
        let by_a_second_entry = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(2),
                None,
                None,
                None,
            ),
            None,
            vec![run(Some(100), 1)],
        );

        assert_eq!(
            resolved(&movie_fragment(vec![by_a_second_entry]), &one_track_movie()),
            Err(SampleError::unknown_sample_description_index(1, 2))
        );
    }

    #[test]
    fn a_fragment_of_a_track_reading_from_an_external_file_is_refused() {
        let external = track_reading_from(1, external_data_reference());

        assert_eq!(
            resolved(&one_sample_movie_fragment(), &movie(vec![external])),
            Err(SampleError::external_data_reference(1, 1))
        );
    }

    #[test]
    fn an_empty_duration_moves_the_timeline_on_without_a_sample() {
        let empty = TrackFragmentBox::with_empty_duration(
            track_fragment_header(TrackFragmentHeaderFlags::ZERO, 1, None, Some(4_096), None),
            None,
        );

        let mut decode_times = track_1_at(1_024);

        assert_eq!(
            resolved_from(
                &movie_fragment(vec![empty]),
                &one_track_movie(),
                0,
                &mut decode_times
            ),
            Ok(vec![])
        );
        assert_eq!(decode_times.decode_time(1), 5_120);
    }

    #[test]
    fn a_track_is_moved_past_the_samples_of_each_of_its_fragments_in_turn() {
        let mut decode_times = TrackDecodeTimes::new();
        let two_of_the_same_track = movie_fragment(vec![
            track_fragment(1, vec![run(Some(100), 2)]),
            track_fragment(1, vec![run(Some(108), 1)]),
        ]);

        assert_eq!(
            resolved_from(
                &two_of_the_same_track,
                &movie(vec![track(1), track(2)]),
                0,
                &mut decode_times
            ),
            Ok(vec![
                extent(1, 0, 100..104),
                extent(1, 1_024, 104..108),
                extent(1, 2_048, 108..112),
            ])
        );
        assert_eq!(decode_times.decode_time(1), 3_072);
        assert_eq!(decode_times.decode_time(2), 0);
    }

    #[test]
    fn a_failure_returned_outright_leaves_every_track_where_it_stood() {
        let mut decode_times = TrackDecodeTimes::new();
        let then_an_unknown_track = movie_fragment(vec![
            track_fragment(1, vec![run(Some(100), 1)]),
            track_fragment(3, vec![run(Some(104), 1)]),
        ]);

        assert_eq!(
            resolved_from(
                &then_an_unknown_track,
                &one_track_movie(),
                0,
                &mut decode_times
            ),
            Err(SampleError::unknown_track_id(3))
        );
        assert_eq!(decode_times.decode_time(1), 0);
    }

    #[test]
    fn the_sequence_number_a_fragment_carries_is_neither_checked_nor_reported() {
        let out_of_order = MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(9),
            vec![track_fragment(1, vec![run(Some(100), 1)])],
        );

        assert_eq!(
            resolved(&out_of_order, &one_track_movie()),
            Ok(vec![extent(1, 0, 100..104)])
        );
    }

    #[test]
    fn the_extents_placed_before_a_failure_come_out_ahead_of_it_and_the_tracks_have_moved() {
        let mut decode_times = TrackDecodeTimes::new();
        let then_past_the_end_of_the_file = movie_fragment(vec![
            track_fragment(1, vec![run(Some(100), 1)]),
            TrackFragmentBox::new(
                track_fragment_header(
                    TrackFragmentHeaderFlags::ZERO,
                    2,
                    Some(u64::MAX),
                    None,
                    None,
                ),
                None,
                vec![run(None, 1)],
            ),
        ]);

        assert_eq!(
            sample_extents(
                &then_past_the_end_of_the_file,
                &movie(vec![track(1), track(2)]),
                0,
                &mut decode_times
            )
            .unwrap()
            .collect::<Vec<_>>(),
            [
                Ok(extent(1, 0, 100..104)),
                Err(SampleError::data_offset_overflow(2))
            ]
        );
        assert_eq!(decode_times.decode_time(1), 1_024);
        assert_eq!(decode_times.decode_time(2), 1_024);
    }

    #[test]
    fn decode_times_running_past_what_64_bits_carry_are_refused() {
        let at_the_end_of_time = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(u32::MAX),
                None,
            ),
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(u64::MAX)),
            vec![run(Some(100), 1)],
        );

        assert_eq!(
            resolved(
                &movie_fragment(vec![at_the_end_of_time]),
                &one_track_movie()
            ),
            Err(SampleError::decode_time_overflow(1))
        );
    }

    #[test]
    fn data_offsets_running_past_what_64_bits_carry_are_refused() {
        let past_the_end_of_the_file = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderFlags::ZERO,
                1,
                Some(u64::MAX),
                None,
                None,
            ),
            None,
            vec![run(None, 1)],
        );

        assert_eq!(
            resolved(
                &movie_fragment(vec![past_the_end_of_the_file]),
                &one_track_movie()
            ),
            Err(SampleError::data_offset_overflow(1))
        );
    }
}
