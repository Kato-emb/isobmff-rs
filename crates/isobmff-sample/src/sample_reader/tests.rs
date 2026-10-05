use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::SampleFlags;

use super::SampleReader;
use crate::error::Error;
use crate::sample::{Sample, SampleExtent};

/// Extent of a sample of track 1 as the tests here declare it
fn extent(decode_time: u64, data: Range<u64>) -> SampleExtent {
    SampleExtent::new(1, decode_time, 1_024, 0, SampleFlags::ZERO, 1, 1, data)
}

/// Sample of track 1 as [`extent`] declares it
fn sample(decode_time: u64, data: &[u8]) -> Sample {
    Sample::new(
        1,
        decode_time,
        1_024,
        0,
        SampleFlags::ZERO,
        1,
        data.to_vec(),
    )
}

/// Reader holding the extents given, none of them met
fn holding(extents: impl IntoIterator<Item = SampleExtent>) -> SampleReader {
    let mut reader = SampleReader::new();
    for extent in extents {
        reader.handle_sample_extent(extent).unwrap();
    }

    reader
}

/// Takes every sample the reader has made whole
fn drained(reader: &mut SampleReader) -> Vec<Sample> {
    let mut samples = Vec::new();
    while let Some(sample) = reader.poll_sample() {
        samples.push(sample);
    }

    samples
}

#[test]
fn a_cleared_reader_holds_nothing_and_reads_the_next_stretch_under_the_same_limit() {
    let mut reader = SampleReader::with_sample_size_limit(8);
    reader
        .handle_sample_extents([Ok(extent(0, 100..104)), Ok(extent(1_024, 104..108))])
        .unwrap();
    reader.handle_data(100, b"ABCD").unwrap();

    reader.clear();

    assert_eq!(reader.wanted_extent(), None);
    assert_eq!(reader.poll_sample(), None);

    reader
        .handle_sample_extents([Ok(extent(8_192, 500..504))])
        .unwrap();
    reader.handle_data(500, b"WXYZ").unwrap();

    assert_eq!(drained(&mut reader), [sample(8_192, b"WXYZ")]);
    assert_eq!(
        reader.handle_sample_extent(extent(9_216, 504..513)),
        Err(Error::sample_size_limit_exceeded(1, 9, 8))
    );
}

#[test]
fn a_reader_declared_over_takes_extents_again_once_cleared() {
    let mut finished = SampleReader::new();
    finished.finish().unwrap();

    finished.clear();

    assert_eq!(finished.handle_sample_extent(extent(0, 100..104)), Ok(()));
}

#[test]
fn a_failed_reader_stays_failed_once_cleared() {
    let mut failed = SampleReader::new();
    failed.handle_sample_extent(extent(0, 100..104)).unwrap();
    let unfinished = failed.finish().unwrap_err();

    failed.clear();

    assert_eq!(failed.handle_data(100, b"ABCD"), Err(unfinished));
}

#[test]
fn a_sample_is_reported_once_the_bytes_its_extent_names_have_arrived() {
    let mut reader = holding([extent(0, 100..104)]);

    reader.handle_data(100, b"AB").unwrap();
    assert_eq!(reader.poll_sample(), None);

    reader.handle_data(102, b"CD").unwrap();
    assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
}

#[test]
fn input_read_in_order_meets_every_extent_without_a_want_being_asked() {
    let mut reader = holding([extent(0, 100..104), extent(1_024, 104..108)]);

    reader.handle_data(100, b"ABCDEF").unwrap();
    reader.handle_data(106, b"GH").unwrap();

    assert_eq!(
        drained(&mut reader),
        [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
    );
    assert_eq!(reader.wanted_extent(), None);
}

#[test]
fn extents_handed_over_together_are_held_in_the_order_of_their_bytes() {
    let mut reader = SampleReader::new();

    reader
        .handle_sample_extents([Ok(extent(1_024, 104..108)), Ok(extent(0, 100..104))])
        .unwrap();
    assert_eq!(reader.wanted_extent(), Some(100..104));

    reader.handle_data(100, b"ABCDEFGH").unwrap();
    assert_eq!(
        drained(&mut reader),
        [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
    );
}

#[test]
fn extents_handed_over_together_starting_at_the_same_byte_keep_the_order_they_came() {
    let mut reader = SampleReader::new();

    reader
        .handle_sample_extents([Ok(extent(1_024, 100..104)), Ok(extent(0, 100..104))])
        .unwrap();
    reader.handle_data(100, b"ABCD").unwrap();

    assert_eq!(
        drained(&mut reader),
        [sample(1_024, b"ABCD"), sample(0, b"ABCD")]
    );
}

#[test]
fn extents_handed_over_together_behind_one_lying_past_them_are_filled_all_the_same() {
    let mut reader = holding([extent(2_048, 200..204)]);

    reader
        .handle_sample_extents([Ok(extent(1_024, 104..108)), Ok(extent(0, 100..104))])
        .unwrap();
    reader.handle_data(100, b"ABCDEFGH").unwrap();

    assert_eq!(
        drained(&mut reader),
        [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
    );
    assert_eq!(reader.wanted_extent(), Some(200..204));
}

#[test]
fn the_failure_a_resolver_stopped_at_fails_the_reader_after_the_extents_before_it_held_in_order() {
    let mut reader = SampleReader::new();
    let stopped_at = Error::data_offset_overflow(1);

    assert_eq!(
        reader.handle_sample_extents([
            Ok(extent(1_024, 104..108)),
            Ok(extent(0, 100..100)),
            Err(stopped_at)
        ]),
        Err(stopped_at)
    );
    assert_eq!(drained(&mut reader), [sample(0, b"")]);
    assert_eq!(reader.wanted_extent(), Some(104..108));
    assert_eq!(reader.handle_data(104, b"ABCD"), Err(stopped_at));
}

#[test]
fn an_extent_held_behind_one_lying_past_it_is_filled_all_the_same() {
    let mut reader = holding([extent(0, 200..204), extent(1_024, 100..104)]);

    reader.handle_data(100, b"ABCD").unwrap();

    assert_eq!(drained(&mut reader), [sample(1_024, b"ABCD")]);
    assert_eq!(reader.wanted_extent(), Some(200..204));
}

#[test]
fn an_extent_naming_no_bytes_held_behind_a_short_one_does_not_hide_the_extents_held_after_it() {
    let mut reader = holding([extent(0, 0..100), extent(1_024, 10..10)]);
    reader.handle_data(0, &[0xab; 30]).unwrap();

    reader.handle_sample_extent(extent(2_048, 20..25)).unwrap();
    reader.handle_data(20, b"ABCDE").unwrap();

    assert_eq!(drained(&mut reader), [sample(2_048, b"ABCDE")]);
    assert_eq!(reader.wanted_extent(), Some(30..100));
}

#[test]
fn extents_overlapping_each_take_what_they_lack_from_the_same_input() {
    let mut reader = holding([extent(0, 100..110), extent(1_024, 104..108)]);

    reader.handle_data(100, b"ABCDEF").unwrap();
    assert_eq!(reader.poll_sample(), None);

    reader.handle_data(106, b"GHIJ").unwrap();
    assert_eq!(
        drained(&mut reader),
        [sample(0, b"ABCDEFGHIJ"), sample(1_024, b"EFGH")]
    );
}

#[test]
fn bytes_arriving_before_the_extent_that_names_them_are_dropped_and_wanted_again() {
    let mut reader = SampleReader::new();
    reader.handle_data(100, b"ABCD").unwrap();

    reader.handle_sample_extent(extent(0, 100..104)).unwrap();
    assert_eq!(reader.poll_sample(), None);
    assert_eq!(reader.wanted_extent(), Some(100..104));

    reader.handle_data(100, b"ABCD").unwrap();
    assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
}

#[test]
fn bytes_no_extent_names_are_passed_over() {
    let mut reader = holding([extent(0, 100..104)]);

    reader.handle_data(200, b"XXXX").unwrap();

    assert_eq!(reader.poll_sample(), None);
    assert_eq!(reader.wanted_extent(), Some(100..104));
}

#[test]
fn the_reader_holds_no_more_than_the_bytes_its_extents_name() {
    let mut reader = holding([
        extent(0, 100..104),
        extent(1_024, 104..108),
        extent(2_048, 1_000..1_004),
    ]);
    assert_eq!(reader.held_bytes(), 0);

    reader.handle_data(2_000, &[0xab; 4_096]).unwrap();
    assert_eq!(reader.held_bytes(), 0);

    reader.handle_data(0, &[0xab; 106]).unwrap();
    assert_eq!(reader.held_bytes(), 4 + 4);

    reader.handle_data(100, &[0xab; 2_048]).unwrap();
    assert_eq!(reader.held_bytes(), 4 + 4 + 4);

    assert_eq!(drained(&mut reader).len(), 3);
    assert_eq!(reader.held_bytes(), 0);
}

#[test]
fn the_want_is_what_the_earliest_extent_held_still_lacks() {
    let mut reader = holding([extent(0, 100..104), extent(1_024, 200..204)]);
    assert_eq!(reader.wanted_extent(), Some(100..104));

    reader.handle_data(100, b"AB").unwrap();
    assert_eq!(reader.wanted_extent(), Some(102..104));

    reader.handle_data(102, b"CD").unwrap();
    assert_eq!(reader.wanted_extent(), Some(200..204));
}

#[test]
fn bytes_that_arrived_already_leave_the_sample_as_it_stands() {
    let mut reader = holding([extent(0, 100..104)]);

    reader.handle_data(100, b"AB").unwrap();
    reader.handle_data(100, b"XXCD").unwrap();

    assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
}

#[test]
fn a_sample_fills_from_its_start() {
    let mut reader = holding([extent(0, 100..104)]);

    reader.handle_data(102, b"CD").unwrap();
    assert_eq!(reader.poll_sample(), None);

    reader.handle_data(100, b"ABCD").unwrap();
    assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
}

#[test]
fn samples_come_out_in_the_order_their_bytes_arrived_whole() {
    let mut reader = holding([extent(0, 100..104), extent(1_024, 104..108)]);

    reader.handle_data(104, b"EFGH").unwrap();
    reader.handle_data(100, b"ABCD").unwrap();

    assert_eq!(
        drained(&mut reader),
        [sample(1_024, b"EFGH"), sample(0, b"ABCD")]
    );
}

#[test]
fn two_extents_naming_the_same_bytes_are_each_met_by_them() {
    let mut reader = holding([extent(0, 100..104), extent(1_024, 100..104)]);

    reader.handle_data(100, b"ABCD").unwrap();

    assert_eq!(
        drained(&mut reader),
        [sample(0, b"ABCD"), sample(1_024, b"ABCD")]
    );
}

#[test]
fn an_extent_naming_no_bytes_is_whole_as_soon_as_it_is_held() {
    let mut reader = holding([extent(0, 100..100)]);

    assert_eq!(drained(&mut reader), [sample(0, b"")]);
    assert_eq!(reader.wanted_extent(), None);
    reader.finish().unwrap();
}

#[test]
fn an_extent_naming_no_bytes_waits_on_the_short_ones_held_before_it() {
    let mut reader = holding([extent(0, 100..104), extent(1_024, 104..104)]);
    assert_eq!(reader.poll_sample(), None);

    reader.handle_data(100, b"ABCD").unwrap();

    assert_eq!(
        drained(&mut reader),
        [sample(0, b"ABCD"), sample(1_024, b"")]
    );
}

#[test]
fn input_making_a_sample_whole_hands_over_every_whole_sample_held_that_carries_bytes() {
    let mut reader = holding([
        extent(0, 100..104),
        extent(1_024, 104..104),
        extent(2_048, 104..108),
    ]);

    reader.handle_data(104, b"EFGH").unwrap();

    assert_eq!(drained(&mut reader), [sample(2_048, b"EFGH")]);
    assert_eq!(reader.wanted_extent(), Some(100..104));
}

#[test]
fn an_extent_naming_no_bytes_is_not_handed_over_ahead_of_a_short_one_held_before_it() {
    let mut reader = holding([
        extent(0, 100..104),
        extent(1_024, 104..108),
        extent(2_048, 108..108),
    ]);

    reader.handle_data(100, b"ABCD").unwrap();
    assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);

    reader.handle_data(104, b"EFGH").unwrap();
    assert_eq!(
        drained(&mut reader),
        [sample(1_024, b"EFGH"), sample(2_048, b"")]
    );
}

#[test]
fn an_extent_naming_more_bytes_than_the_limit_is_refused() {
    let mut reader = SampleReader::with_sample_size_limit(3);

    assert_eq!(
        reader.handle_sample_extent(extent(0, 100..104)),
        Err(Error::sample_size_limit_exceeded(1, 4, 3))
    );
}

#[test]
fn a_sample_short_of_its_bytes_is_refused_when_the_samples_are_declared_over() {
    let mut reader = holding([extent(0, 100..104)]);
    reader.handle_data(100, b"AB").unwrap();

    assert_eq!(reader.finish(), Err(Error::unfinished_sample(1, 4, 2)));
}

#[test]
fn a_failure_is_reported_again_for_every_call_after_it() {
    let mut reader = SampleReader::with_sample_size_limit(3);
    let refused = reader.handle_sample_extent(extent(0, 100..104));

    assert_eq!(reader.handle_data(100, b"ABCD"), refused);
    assert_eq!(reader.finish(), refused);
}

#[test]
fn samples_made_whole_before_a_failure_are_still_taken() {
    let mut reader = holding([extent(0, 100..104), extent(1_024, 104..108)]);
    reader.handle_data(100, b"ABCD").unwrap();

    assert_eq!(reader.finish(), Err(Error::unfinished_sample(1, 4, 0)));
    assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
}

#[test]
fn anything_handed_over_after_the_samples_were_declared_over_is_refused() {
    let mut reader = SampleReader::new();
    reader.finish().unwrap();

    assert_eq!(
        reader.handle_sample_extent(extent(0, 100..104)),
        Err(Error::already_finished())
    );
    assert_eq!(
        reader.handle_data(100, b"ABCD"),
        Err(Error::already_finished())
    );
    assert_eq!(reader.finish(), Err(Error::already_finished()));
}
