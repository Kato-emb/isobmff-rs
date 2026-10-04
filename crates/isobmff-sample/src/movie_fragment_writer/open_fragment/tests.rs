use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{
    DegradationPriorityEntry, MovieFragmentBox, MovieFragmentHeaderBox, PaddingBitsEntry,
    SampleDependencyTypeEntry, SampleFlags, TrackFragmentBox, TrackFragmentHeaderBox,
    TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
};
use isobmff_core::BoxEncode as _;

use crate::error::Error;
use crate::movie_fragment_writer::tests::{sample, writer};
use crate::sample::Sample;

/// Bytes the header of the `mdat` beside a fragment occupies
const MEDIA_DATA_HEADER_LEN: u64 = 8;

/// Sample of track 1 at `decode_time` stating `sample_composition_time_offset`
fn offset_by(decode_time: u64, sample_composition_time_offset: i64) -> Sample {
    Sample::new(
        1,
        decode_time,
        1_024,
        sample_composition_time_offset,
        SampleFlags::ZERO,
        1,
        b"AAAA".to_vec(),
    )
}

/// Sample of `track_id` at `decode_time` lasting `sample_duration`, stating `sample_composition_time_offset`
fn timed(
    track_id: u32,
    decode_time: u64,
    sample_duration: u32,
    sample_composition_time_offset: i64,
) -> Sample {
    Sample::new(
        track_id,
        decode_time,
        sample_duration,
        sample_composition_time_offset,
        SampleFlags::ZERO,
        1,
        b"AAAA".to_vec(),
    )
}

/// Flags stating `sample_depends_on` and `sample_is_non_sync_sample`, every other field 0
fn flags(sample_depends_on: u8, sample_is_non_sync_sample: bool) -> SampleFlags {
    SampleFlags::new(
        SampleDependencyTypeEntry::new(0, sample_depends_on, 0, 0).unwrap(),
        PaddingBitsEntry::default(),
        sample_is_non_sync_sample,
        DegradationPriorityEntry::default(),
    )
}

/// Sample of track 1 at `decode_time` stating `sample_flags`
fn flagged(decode_time: u64, sample_flags: SampleFlags) -> Sample {
    Sample::new(1, decode_time, 1_024, 0, sample_flags, 1, b"AAAA".to_vec())
}

/// Writes `samples` as one fragment, and returns the boxes it is written as
fn one_fragment(samples: Vec<Sample>) -> (MovieFragmentBox, Vec<u8>) {
    let mut writer = writer();

    writer.begin_fragment(1).unwrap();
    for sample in samples {
        writer.handle_sample(sample).unwrap();
    }

    writer.finish_fragment().unwrap()
}

/// The fragment `track_id` contributed to `movie_fragment`
fn track_fragment_of(movie_fragment: &MovieFragmentBox, track_id: u32) -> &TrackFragmentBox {
    movie_fragment
        .traf()
        .iter()
        .find(|track_fragment| track_fragment.tfhd().track_id() == track_id)
        .unwrap()
}

/// Offset the first run of `movie_fragment` states, past the fragment and the header of its `mdat`
fn data_offset_of(movie_fragment: &MovieFragmentBox) -> i32 {
    i32::try_from(
        movie_fragment
            .encoded_len()
            .saturating_add(MEDIA_DATA_HEADER_LEN),
    )
    .unwrap()
}

/// Rows each run of the fragment of `track_id` carries
fn rows_of_the_runs(movie_fragment: &MovieFragmentBox, track_id: u32) -> Vec<usize> {
    track_fragment_of(movie_fragment, track_id)
        .trun()
        .iter()
        .map(|track_run| track_run.samples().len())
        .collect()
}

/// Header a `traf` of track 1 is written with, stating the defaults given
fn track_fragment_header(
    default_sample_duration: Option<u32>,
    default_sample_size: Option<u32>,
    default_sample_flags: Option<SampleFlags>,
) -> TrackFragmentHeaderBox {
    TrackFragmentHeaderBox::new(
        TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
        1,
        None,
        Some(1),
        default_sample_duration,
        default_sample_size,
        default_sample_flags,
    )
}

/// Every row the fragment of `track_id` carries, run after run
fn rows_of(movie_fragment: &MovieFragmentBox, track_id: u32) -> Vec<TrackRunSample> {
    track_fragment_of(movie_fragment, track_id)
        .trun()
        .iter()
        .flat_map(|track_run| track_run.samples())
        .cloned()
        .collect()
}

#[test]
fn a_decode_time_is_written_for_every_fragment_of_a_track() {
    let mut writer = writer();
    let mut decode_times = Vec::new();

    for (sequence_number, decode_time) in [(1, 0), (2, 8_192)] {
        writer.begin_fragment(sequence_number).unwrap();
        writer
            .handle_sample(sample(1, decode_time, b"AAAA"))
            .unwrap();
        let (movie_fragment, _media_data) = writer.finish_fragment().unwrap();
        decode_times.push(
            track_fragment_of(&movie_fragment, 1)
                .tfdt()
                .unwrap()
                .base_media_decode_time(),
        );
    }

    assert_eq!(decode_times, [0, 8_192]);
}

#[test]
fn a_fragment_opened_continuing_places_every_track_where_it_reached() {
    let first_fragment = [timed(1, 0, 3_000, 0), timed(2, 0, 1_024, 0)];
    let mut continuing = writer();
    let mut stated = writer();
    for writer in [&mut continuing, &mut stated] {
        writer.begin_fragment(1).unwrap();
        for sample in first_fragment.clone() {
            writer.handle_sample(sample).unwrap();
        }
        writer.finish_fragment().unwrap();
    }

    continuing.begin_fragment_continuing(2).unwrap();
    for sample in [
        timed(1, 90_000, 3_000, 3_000),
        timed(2, 30_720, 1_024, 0),
        timed(1, 93_000, 1_500, 0),
    ] {
        continuing.handle_sample(sample).unwrap();
    }
    stated.begin_fragment(2).unwrap();
    for sample in [
        timed(1, 3_000, 3_000, 3_000),
        timed(2, 1_024, 1_024, 0),
        timed(1, 6_000, 1_500, 0),
    ] {
        stated.handle_sample(sample).unwrap();
    }
    let (movie_fragment, _media_data) = continuing.finish_fragment().unwrap();

    let decode_times: Vec<u64> = movie_fragment
        .traf()
        .iter()
        .map(|track_fragment| track_fragment.tfdt().unwrap().base_media_decode_time())
        .collect();
    assert_eq!(decode_times, [3_000, 1_024]);
    assert_eq!(
        Ok((movie_fragment, b"AAAAAAAAAAAA".to_vec())),
        stated.finish_fragment()
    );
}

#[test]
fn a_first_fragment_opened_continuing_places_its_tracks_at_zero() {
    let mut writer = writer();

    writer.begin_fragment_continuing(1).unwrap();
    writer.handle_sample(sample(1, 90_000, b"AAAA")).unwrap();
    let (movie_fragment, _media_data) = writer.finish_fragment().unwrap();

    assert_eq!(
        track_fragment_of(&movie_fragment, 1)
            .tfdt()
            .unwrap()
            .base_media_decode_time(),
        0
    );
}

#[test]
fn the_media_data_holds_the_samples_in_the_order_they_arrived() {
    let (_movie_fragment, media_data) = one_fragment(vec![
        sample(1, 0, b"AAAA"),
        sample(2, 0, b"BBBB"),
        sample(1, 1_024, b"CCCC"),
    ]);

    assert_eq!(media_data, b"AAAABBBBCCCC");
}

#[test]
fn one_track_fragment_per_track_in_the_order_the_tracks_first_appear() {
    let (movie_fragment, _media_data) = one_fragment(vec![
        sample(2, 0, b"AAAA"),
        sample(1, 0, b"BBBB"),
        sample(2, 1_024, b"CCCC"),
    ]);

    let tracks: Vec<u32> = movie_fragment
        .traf()
        .iter()
        .map(|track_fragment| track_fragment.tfhd().track_id())
        .collect();
    assert_eq!(tracks, [2, 1]);
}

#[test]
fn samples_of_one_track_handed_over_together_are_one_run() {
    let (together, _media_data) = one_fragment(vec![
        sample(1, 0, b"AAAA"),
        sample(1, 1_024, b"BBBB"),
        sample(2, 0, b"CCCC"),
    ]);
    let (apart, _media_data) = one_fragment(vec![
        sample(1, 0, b"AAAA"),
        sample(2, 0, b"CCCC"),
        sample(1, 1_024, b"BBBB"),
    ]);

    assert_eq!(rows_of_the_runs(&together, 1), [2]);
    assert_eq!(rows_of_the_runs(&apart, 1), [1, 1]);
}

#[test]
fn offsets_are_anchored_at_the_fragment_itself() {
    let (movie_fragment, _media_data) =
        one_fragment(vec![sample(1, 0, b"AAAA"), sample(2, 0, b"BBBB")]);

    let past_the_fragment = movie_fragment
        .encoded_len()
        .saturating_add(MEDIA_DATA_HEADER_LEN);
    let offsets: Vec<Option<i32>> = movie_fragment
        .traf()
        .iter()
        .flat_map(TrackFragmentBox::trun)
        .map(TrackRunBox::data_offset)
        .collect();

    assert_eq!(
        offsets,
        [
            Some(i32::try_from(past_the_fragment).unwrap()),
            Some(i32::try_from(past_the_fragment.saturating_add(4)).unwrap()),
        ]
    );
}

#[test]
fn what_the_samples_share_is_stated_once_by_their_track_fragment_header() {
    let (movie_fragment, _media_data) =
        one_fragment(vec![sample(1, 0, b"AAAA"), sample(1, 1_024, b"BBBB")]);
    let header = track_fragment_of(&movie_fragment, 1).tfhd();

    assert_eq!(
        *header,
        track_fragment_header(Some(1_024), Some(4), Some(SampleFlags::ZERO))
    );
    assert_eq!(
        rows_of(&movie_fragment, 1),
        vec![TrackRunSample::new(None, None, None, None); 2]
    );
}

#[test]
fn what_the_samples_do_not_share_is_stated_by_every_row() {
    let shorter = Sample::new(1, 1_024, 512, 0, SampleFlags::ZERO, 1, b"BB".to_vec());
    let (movie_fragment, _media_data) = one_fragment(vec![sample(1, 0, b"AAAA"), shorter]);
    let header = track_fragment_of(&movie_fragment, 1).tfhd();

    assert_eq!(
        *header,
        track_fragment_header(None, None, Some(SampleFlags::ZERO))
    );
    assert_eq!(
        rows_of(&movie_fragment, 1),
        [
            TrackRunSample::new(Some(1_024), Some(4), None, None),
            TrackRunSample::new(Some(512), Some(2), None, None),
        ]
    );
}

#[test]
fn flags_only_the_first_sample_differs_on_are_written_as_its_own() {
    let (movie_fragment, _media_data) = one_fragment(vec![
        flagged(0, flags(2, false)),
        flagged(1_024, flags(1, true)),
        flagged(2_048, flags(1, true)),
    ]);
    let track_fragment = track_fragment_of(&movie_fragment, 1);

    assert_eq!(
        *track_fragment.tfhd(),
        track_fragment_header(Some(1_024), Some(4), Some(flags(1, true)))
    );
    assert_eq!(
        track_fragment.trun(),
        [TrackRunBox::new(
            Some(data_offset_of(&movie_fragment)),
            Some(flags(2, false)),
            vec![TrackRunSample::new(None, None, None, None); 3],
        )
        .unwrap()]
    );
}

#[test]
fn flags_no_two_samples_share_are_written_by_every_row() {
    let (movie_fragment, _media_data) = one_fragment(vec![
        flagged(0, flags(2, false)),
        flagged(1_024, flags(1, true)),
        flagged(2_048, flags(1, false)),
    ]);
    let track_fragment = track_fragment_of(&movie_fragment, 1);

    assert_eq!(
        *track_fragment.tfhd(),
        track_fragment_header(Some(1_024), Some(4), None)
    );
    assert_eq!(
        track_fragment.trun(),
        [TrackRunBox::new(
            Some(data_offset_of(&movie_fragment)),
            None,
            vec![
                TrackRunSample::new(None, None, Some(flags(2, false)), None),
                TrackRunSample::new(None, None, Some(flags(1, true)), None),
                TrackRunSample::new(None, None, Some(flags(1, false)), None),
            ],
        )
        .unwrap()]
    );
}

#[test]
fn a_run_stating_offsets_no_one_trun_version_writes_both_of_is_refused() {
    let mut writer = writer();

    writer.begin_fragment(1).unwrap();
    writer.handle_sample(offset_by(0, -8)).unwrap();
    writer
        .handle_sample(offset_by(1_024, i64::from(u32::MAX)))
        .unwrap();

    assert_eq!(
        writer.finish_fragment(),
        Err(Error::composition_time_offset_out_of_range(
            1,
            i64::from(u32::MAX)
        ))
    );
}

#[test]
fn a_composition_time_offset_no_run_writes_is_refused() {
    let past_the_field = i64::from(u32::MAX).saturating_add(1);
    let mut writer = writer();

    writer.begin_fragment(1).unwrap();

    assert_eq!(
        writer.handle_sample(offset_by(0, past_the_field)),
        Err(Error::composition_time_offset_out_of_range(
            1,
            past_the_field
        ))
    );
}

#[test]
fn a_fragment_of_no_samples_is_written_as_an_empty_pair() {
    let (movie_fragment, media_data) = one_fragment(vec![]);

    assert_eq!(
        movie_fragment,
        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![])
    );
    assert_eq!(media_data, Vec::<u8>::new());
}

#[test]
fn a_sample_that_does_not_start_where_the_one_before_it_ends_is_refused() {
    let mut writer = writer();

    writer.begin_fragment(1).unwrap();
    writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

    assert_eq!(
        writer.handle_sample(sample(1, 512, b"BBBB")),
        Err(Error::decode_time_mismatch(1, 512, 1_024))
    );
}

#[test]
fn a_mismatch_in_a_fragment_opened_continuing_is_reported_in_the_times_the_samples_state() {
    let mut writer = writer();

    writer.begin_fragment(1).unwrap();
    writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
    writer.finish_fragment().unwrap();
    writer.begin_fragment_continuing(2).unwrap();
    writer.handle_sample(sample(1, 90_000, b"BBBB")).unwrap();

    assert_eq!(
        writer.handle_sample(sample(1, 90_512, b"CCCC")),
        Err(Error::decode_time_mismatch(1, 90_512, 91_024))
    );
}

#[test]
fn samples_of_one_fragment_described_by_two_entries_are_refused() {
    let described_by_the_second =
        Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 2, b"BBBB".to_vec());
    let mut writer = writer();

    writer.begin_fragment(1).unwrap();
    writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

    assert_eq!(
        writer.handle_sample(described_by_the_second),
        Err(Error::sample_description_index_mismatch(1, 2, 1))
    );
}

#[test]
fn decode_times_running_past_what_64_bits_carry_are_refused() {
    let at_the_end_of_time = Sample::new(
        1,
        u64::MAX,
        1_024,
        0,
        SampleFlags::ZERO,
        1,
        b"AAAA".to_vec(),
    );
    let mut writer = writer();

    writer.begin_fragment(1).unwrap();

    assert_eq!(
        writer.handle_sample(at_the_end_of_time),
        Err(Error::decode_time_overflow(1))
    );
}
