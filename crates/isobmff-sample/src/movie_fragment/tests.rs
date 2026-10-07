use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{
    CompositionTimeOffset, DegradationPriorityEntry, HeaderDuration, IsLeading, MovieBox,
    MovieExtendsBox, MovieFragmentBox, MovieFragmentHeaderBox, MovieHeaderBox, PaddingBitsEntry,
    SampleDependencyTypeEntry, SampleDependsOn, SampleFlags, SampleHasRedundancy,
    SampleIsDependedOn, TrackBox, TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox,
    TrackRunSample,
};
use isobmff_core::{BoxDecode as _, BoxEncode as _, Mp4EpochSeconds};
use isobmff_test_support::{
    external_data_reference, fragmented_movie, track, track_reading_from, unfragmented_movie,
    written,
};

use super::sample_extents;
use crate::error::Error;
use crate::sample::{SampleExtent, SampleProperties};
use crate::track_decode_times::TrackDecodeTimes;

/// Movie of the given tracks, each fragmented with samples that last 1024 units and occupy 4 bytes
fn movie(trak: Vec<TrackBox>) -> MovieBox {
    let trex = trak
        .iter()
        .map(|trak| TrackExtendsBox::new(trak.tkhd().track_id(), 1, 1_024, 4, SampleFlags::ZERO))
        .collect();

    MovieBox::new(
        MovieHeaderBox::new(
            Mp4EpochSeconds::from_seconds(0),
            Mp4EpochSeconds::from_seconds(0),
            1_000,
            HeaderDuration::ZERO,
            2,
        ),
        trak,
        MovieExtendsBox::new(trex),
    )
    .unwrap()
}

/// Index just past the `occurrence`-th box type `four_cc` names in `written`, counting from 0
fn past_type(written: &[u8], four_cc: &[u8; 4], occurrence: usize) -> usize {
    written
        .windows(4)
        .enumerate()
        .filter(|(_, window)| window == four_cc)
        .nth(occurrence)
        .and_then(|(at, _)| at.checked_add(4))
        .unwrap()
}

/// The movie `written` holds, read with its second `mdhd` stating version 2 so the track holding it is kept unread
fn read_keeping_second_track_unread(mut written: Vec<u8>) -> MovieBox {
    let version_at = past_type(&written, b"mdhd", 1);
    *written.get_mut(version_at).unwrap() = 2;

    MovieBox::decode(&written).unwrap().0
}

/// The movie `written` holds with its second `trex` naming track 9, which leaves track 2 with no `trex`
fn with_no_trex_for_track_2(mut written: Vec<u8>) -> Vec<u8> {
    let track_id_at = past_type(&written, b"trex", 1).checked_add(4).unwrap();
    written
        .get_mut(track_id_at..track_id_at.checked_add(4).unwrap())
        .unwrap()
        .copy_from_slice(&9_u32.to_be_bytes());

    written
}

/// `movie(vec![track(1), track(2)])` read with track 2 kept unread
fn movie_keeping_track_2_unread() -> MovieBox {
    read_keeping_second_track_unread(written(&movie(vec![track(1), track(2)])))
}

/// Movie of one track whose samples last 1024 units and occupy 4 bytes each
fn one_track_movie() -> MovieBox {
    fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 4, SampleFlags::ZERO))
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

/// Flags of a sync sample stating `sample_depends_on`, every other field 0
fn depending_on(sample_depends_on: SampleDependsOn) -> SampleFlags {
    SampleFlags::new(
        SampleDependencyTypeEntry::new(
            IsLeading::Unknown,
            sample_depends_on,
            SampleIsDependedOn::Unknown,
            SampleHasRedundancy::Unknown,
        ),
        PaddingBitsEntry::default(),
        false,
        DegradationPriorityEntry::default(),
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
        trun,
    )
    .unwrap()
}

/// Fragment of `track_id` stating no anchor, of one run of `sample_count` samples
fn stating_no_anchor(
    track_id: u32,
    data_offset: Option<i32>,
    sample_count: u32,
) -> TrackFragmentBox {
    TrackFragmentBox::new(
        track_fragment_header(TrackFragmentHeaderFlags::ZERO, track_id, None, None, None),
        vec![run(data_offset, sample_count)],
    )
    .unwrap()
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
    SampleExtent::new(
        SampleProperties {
            track_id,
            decode_time,
            sample_duration: 1_024,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::ZERO,
            sample_description_index: 1,
        },
        1,
        data,
    )
}

/// Resolves `movie_fragment` at the start of the file against a movie no fragment was resolved for
fn resolved(
    movie_fragment: &MovieFragmentBox,
    movie: &MovieBox,
) -> Result<Vec<SampleExtent>, Error> {
    resolved_from(
        movie_fragment,
        movie,
        0,
        &mut TrackDecodeTimes::new(movie).unwrap(),
    )
}

/// Resolves `movie_fragment` at `moof_start` against a movie whose tracks stand at `decode_times`
fn resolved_from(
    movie_fragment: &MovieFragmentBox,
    movie: &MovieBox,
    moof_start: u64,
    decode_times: &mut TrackDecodeTimes,
) -> Result<Vec<SampleExtent>, Error> {
    sample_extents(movie_fragment, movie, moof_start, decode_times, u64::MAX)?.collect()
}

/// Times of a movie whose track 1 stands at `decode_time`
fn track_1_at(decode_time: u64) -> TrackDecodeTimes {
    let mut decode_times = TrackDecodeTimes::new(&one_track_movie()).unwrap();
    decode_times.set_decode_time(1, decode_time);

    decode_times
}

#[test]
fn a_sample_takes_what_its_row_states_over_the_defaults_of_the_fragment_and_the_track() {
    let rows = vec![TrackRunSample::new(
        Some(512),
        Some(2),
        Some(depending_on(SampleDependsOn::DependsOnOthers)),
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
        vec![TrackRunBox::new(Some(100), None, rows).unwrap()],
    )
    .unwrap();

    assert_eq!(
        resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
        Ok(vec![SampleExtent::new(
            SampleProperties {
                track_id: 1,
                decode_time: 0,
                sample_duration: 512,
                sample_composition_time_offset: -8,
                sample_flags: depending_on(SampleDependsOn::DependsOnOthers),
                sample_description_index: 1
            },
            1,
            100..102
        )])
    );
}

#[test]
fn an_offset_a_version_0_run_states_past_the_signed_range_is_read_as_negative() {
    let rows = vec![TrackRunSample::new(
        None,
        None,
        None,
        Some(CompositionTimeOffset::new(0xFFFF_FC00).unwrap()),
    )];
    let written_signed = TrackRunBox::new(Some(100), None, rows).unwrap();

    assert_eq!(
        resolved(
            &movie_fragment(vec![track_fragment(1, vec![written_signed])]),
            &one_track_movie()
        ),
        Ok(vec![SampleExtent::new(
            SampleProperties {
                track_id: 1,
                decode_time: 0,
                sample_duration: 1_024,
                sample_composition_time_offset: -1_024,
                sample_flags: SampleFlags::ZERO,
                sample_description_index: 1
            },
            1,
            100..104
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
        vec![run(Some(100), 2)],
    )
    .unwrap();

    assert_eq!(
        resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
        Ok(vec![
            SampleExtent::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 0,
                    sample_duration: 256,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 1
                },
                1,
                100..102
            ),
            SampleExtent::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 256,
                    sample_duration: 256,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 1
                },
                1,
                102..104
            ),
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
        vec![
            TrackRunBox::new(
                Some(100),
                Some(depending_on(SampleDependsOn::DoesNotDependOnOthers)),
                rows,
            )
            .unwrap(),
        ],
    );

    assert_eq!(
        resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
        Ok(vec![
            SampleExtent::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 0,
                    sample_duration: 1_024,
                    sample_composition_time_offset: 0,
                    sample_flags: depending_on(SampleDependsOn::DoesNotDependOnOthers),
                    sample_description_index: 1
                },
                1,
                100..104
            ),
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
        vec![run(Some(100), 1)],
    )
    .unwrap()
    .with_tfdt(TrackFragmentBaseMediaDecodeTimeBox::new(4_096));
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
        vec![run(Some(8), 1)],
    )
    .unwrap();

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
            &mut TrackDecodeTimes::new(&one_track_movie()).unwrap()
        ),
        Ok(vec![extent(1, 0, 1_100..1_104)])
    );
}

#[test]
fn offsets_of_a_fragment_stating_no_anchor_at_all_are_anchored_at_the_movie_fragment() {
    let track_fragment = TrackFragmentBox::new(
        track_fragment_header(TrackFragmentHeaderFlags::ZERO, 1, None, None, None),
        vec![run(Some(100), 1)],
    )
    .unwrap();

    assert_eq!(
        resolved(&movie_fragment(vec![track_fragment]), &one_track_movie()),
        Ok(vec![extent(1, 0, 100..104)])
    );
}

#[test]
fn offsets_of_a_later_track_fragment_stating_no_anchor_follow_the_data_before_it() {
    assert_eq!(
        resolved(
            &movie_fragment(vec![
                stating_no_anchor(1, Some(100), 1),
                stating_no_anchor(2, None, 1),
            ]),
            &movie(vec![track(1), track(2)])
        ),
        Ok(vec![extent(1, 0, 100..104), extent(2, 0, 104..108)])
    );
}

#[test]
fn a_track_fragment_after_one_carrying_no_run_is_anchored_where_that_one_was() {
    let carrying_no_run = TrackFragmentBox::new_empty_duration(track_fragment_header(
        TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
        1,
        None,
        Some(4_096),
        None,
    ));
    let stating_no_anchor = TrackFragmentBox::new(
        track_fragment_header(TrackFragmentHeaderFlags::ZERO, 2, None, None, None),
        vec![run(Some(100), 1)],
    )
    .unwrap();

    assert_eq!(
        resolved_from(
            &movie_fragment(vec![carrying_no_run, stating_no_anchor]),
            &movie(vec![track(1), track(2)]),
            1_000,
            &mut TrackDecodeTimes::new(&movie(vec![track(1), track(2)])).unwrap(),
        ),
        Ok(vec![extent(2, 0, 1_100..1_104)])
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
        Err(Error::MissingMovieExtends)
    );
}

#[test]
fn a_fragment_of_a_track_the_movie_never_declared_is_refused() {
    let of_an_unknown_track = movie_fragment(vec![track_fragment(3, vec![run(Some(100), 1)])]);

    assert_eq!(
        resolved(&of_an_unknown_track, &one_track_movie()),
        Err(Error::UnknownTrackId { track_id: 3 })
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
        vec![run(Some(100), 1)],
    )
    .unwrap();

    assert_eq!(
        resolved(&movie_fragment(vec![by_a_second_entry]), &one_track_movie()),
        Err(Error::UnknownSampleDescriptionIndex {
            track_id: 1,
            sample_description_index: 2
        })
    );
}

#[test]
fn a_fragment_of_a_track_reading_from_an_external_file_is_refused() {
    let external = track_reading_from(1, external_data_reference());

    assert_eq!(
        resolved(&one_sample_movie_fragment(), &movie(vec![external])),
        Err(Error::ExternalDataReference {
            track_id: 1,
            data_reference_index: 1
        })
    );
}

#[test]
fn an_empty_duration_moves_the_timeline_on_without_a_sample() {
    let empty = TrackFragmentBox::new_empty_duration(track_fragment_header(
        TrackFragmentHeaderFlags::ZERO,
        1,
        None,
        Some(4_096),
        None,
    ));

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
    assert_eq!(decode_times.decode_time(1), Some(5_120));
}

#[test]
fn a_track_is_moved_past_the_samples_of_each_of_its_fragments_in_turn() {
    let mut decode_times = TrackDecodeTimes::new(&movie(vec![track(1), track(2)])).unwrap();
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
    assert_eq!(decode_times.decode_time(1), Some(3_072));
    assert_eq!(decode_times.decode_time(2), Some(0));
}

#[test]
fn a_failure_returned_outright_leaves_every_track_where_it_stood() {
    let mut decode_times = TrackDecodeTimes::new(&one_track_movie()).unwrap();
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
        Err(Error::UnknownTrackId { track_id: 3 })
    );
    assert_eq!(decode_times.decode_time(1), Some(0));
}

#[test]
fn where_no_track_stands_anywhere_known_only_a_stated_decode_time_resolves() {
    let mut decode_times = TrackDecodeTimes::unknown();
    let stating_none = one_sample_movie_fragment();
    let stating_one = movie_fragment(vec![
        track_fragment(1, vec![run(Some(100), 1)])
            .with_tfdt(TrackFragmentBaseMediaDecodeTimeBox::new(8_192)),
    ]);

    assert_eq!(
        resolved_from(&stating_none, &one_track_movie(), 0, &mut decode_times),
        Err(Error::MissingDecodeTime { track_id: 1 })
    );
    assert_eq!(decode_times, TrackDecodeTimes::unknown());
    assert_eq!(
        resolved_from(&stating_one, &one_track_movie(), 0, &mut decode_times),
        Ok(vec![extent(1, 8_192, 100..104)])
    );
    assert_eq!(
        resolved_from(&stating_none, &one_track_movie(), 0, &mut decode_times),
        Ok(vec![extent(1, 9_216, 100..104)])
    );
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
    let mut decode_times = TrackDecodeTimes::new(&movie(vec![track(1), track(2)])).unwrap();
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
            vec![run(None, 1)],
        )
        .unwrap(),
    ]);

    assert_eq!(
        sample_extents(
            &then_past_the_end_of_the_file,
            &movie(vec![track(1), track(2)]),
            0,
            &mut decode_times,
            u64::MAX
        )
        .unwrap()
        .collect::<Vec<_>>(),
        [
            Ok(extent(1, 0, 100..104)),
            Err(Error::DataOffsetOverflow { track_id: 2 })
        ]
    );
    assert_eq!(decode_times.decode_time(1), Some(1_024));
    assert_eq!(decode_times.decode_time(2), Some(1_024));
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
        vec![run(Some(100), 1)],
    )
    .unwrap()
    .with_tfdt(TrackFragmentBaseMediaDecodeTimeBox::new(u64::MAX));

    assert_eq!(
        resolved(
            &movie_fragment(vec![at_the_end_of_time]),
            &one_track_movie()
        ),
        Err(Error::DecodeTimeOverflow { track_id: 1 })
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
        vec![run(None, 1)],
    )
    .unwrap();

    assert_eq!(
        resolved(
            &movie_fragment(vec![past_the_end_of_the_file]),
            &one_track_movie()
        ),
        Err(Error::DataOffsetOverflow { track_id: 1 })
    );
}

#[test]
fn a_fragment_counting_more_samples_than_the_limit_settles_none() {
    let two_tracks = movie(vec![track(1), track(2)]);
    let two_samples = movie_fragment(vec![
        track_fragment(1, vec![run(Some(100), 1)]),
        track_fragment(2, vec![run(Some(104), 1)]),
    ]);
    let mut decode_times = TrackDecodeTimes::new(&two_tracks).unwrap();

    assert_eq!(
        sample_extents(&two_samples, &two_tracks, 0, &mut decode_times, 1).map(|_| ()),
        Err(Error::SampleCountLimitExceeded {
            declared_samples: 2,
            limit_samples: 1
        })
    );
    assert_eq!(decode_times, TrackDecodeTimes::new(&two_tracks).unwrap());
    assert_eq!(
        sample_extents(&two_samples, &two_tracks, 0, &mut decode_times, 2)
            .unwrap()
            .collect::<Vec<_>>(),
        [Ok(extent(1, 0, 100..104)), Ok(extent(2, 0, 104..108))]
    );
}

#[test]
fn runs_holding_only_a_count_are_counted_before_a_sample_is_settled() {
    let empty_rows = || TrackRunBox::from_sample_count(None, None, u32::MAX);
    let many_runs = movie_fragment(vec![track_fragment(
        1,
        vec![empty_rows(), empty_rows(), empty_rows()],
    )]);

    assert_eq!(
        sample_extents(
            &many_runs,
            &one_track_movie(),
            0,
            &mut TrackDecodeTimes::new(&one_track_movie()).unwrap(),
            1_048_576
        )
        .map(|_| ()),
        Err(Error::SampleCountLimitExceeded {
            declared_samples: 3 * u64::from(u32::MAX),
            limit_samples: 1_048_576
        })
    );
}

#[test]
fn a_fragment_of_a_track_the_movie_kept_unread_gives_no_sample_and_the_others_resolve() {
    let movie_fragment = movie_fragment(vec![
        track_fragment(2, vec![run(Some(100), 1)]),
        track_fragment(1, vec![run(Some(200), 1)]),
    ]);

    assert_eq!(
        resolved(&movie_fragment, &movie_keeping_track_2_unread()),
        Ok(vec![extent(1, 0, 200..204)])
    );
}

#[test]
fn samples_after_a_fragment_kept_unread_lie_where_they_would_were_its_track_read() {
    let movie_fragment = movie_fragment(vec![
        stating_no_anchor(2, Some(100), 2),
        stating_no_anchor(1, None, 1),
    ]);
    let with_every_track_read = resolved(&movie_fragment, &movie(vec![track(1), track(2)]))
        .unwrap()
        .into_iter()
        .filter(|extent| extent.properties().track_id != 2)
        .collect();

    assert_eq!(
        resolved(&movie_fragment, &movie_keeping_track_2_unread()),
        Ok(with_every_track_read)
    );
}

#[test]
fn a_fragment_anchored_after_one_kept_unread_whose_sample_sizes_nothing_states_is_refused() {
    let movie = read_keeping_second_track_unread(with_no_trex_for_track_2(written(&movie(vec![
        track(1),
        track(2),
    ]))));
    let movie_fragment = movie_fragment(vec![
        track_fragment(1, vec![run(Some(50), 1)]),
        stating_no_anchor(2, Some(100), 1),
        stating_no_anchor(1, None, 1),
    ]);

    let extents: Vec<_> = sample_extents(
        &movie_fragment,
        &movie,
        0,
        &mut TrackDecodeTimes::new(&movie).unwrap(),
        u64::MAX,
    )
    .unwrap()
    .collect();

    assert_eq!(
        extents,
        [
            Ok(extent(1, 0, 50..54)),
            Err(Error::UnknownTrackId { track_id: 2 })
        ]
    );
}

#[test]
fn a_fragment_kept_unread_is_walked_by_the_sample_size_its_header_states() {
    let movie = read_keeping_second_track_unread(with_no_trex_for_track_2(written(&movie(vec![
        track(1),
        track(2),
    ]))));
    let kept_unread = TrackFragmentBox::new(
        track_fragment_header(TrackFragmentHeaderFlags::ZERO, 2, None, None, Some(6)),
        vec![run(Some(100), 2)],
    )
    .unwrap();

    assert_eq!(
        resolved(
            &movie_fragment(vec![kept_unread, stating_no_anchor(1, None, 1)]),
            &movie
        ),
        Ok(vec![extent(1, 0, 112..116)])
    );
}

#[test]
fn a_run_stating_its_offset_places_the_end_of_a_fragment_kept_unread_whatever_ran_before_it() {
    let movie = read_keeping_second_track_unread(with_no_trex_for_track_2(written(&movie(vec![
        track(1),
        track(2),
    ]))));
    let sized_run = TrackRunBox::new(
        Some(200),
        None,
        vec![TrackRunSample::new(None, Some(6), None, None)],
    )
    .unwrap();
    let kept_unread = TrackFragmentBox::new(
        track_fragment_header(TrackFragmentHeaderFlags::ZERO, 2, None, None, None),
        vec![run(None, 1), sized_run],
    )
    .unwrap();

    assert_eq!(
        resolved(
            &movie_fragment(vec![kept_unread, stating_no_anchor(1, None, 1)]),
            &movie
        ),
        Ok(vec![extent(1, 0, 206..210)])
    );
}
