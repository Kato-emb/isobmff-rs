use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, MovieBox, SampleFlags, TrackExtendsBox};
use isobmff_core::{BoxDefinition, BoxType};
use isobmff_sample::{Sample, SampleReaderLimits};
use isobmff_test_support::{
    file_type, fragmented_movie, framed, movie_fragment, non_fragmented_file, written,
};

use super::{Error, MovieDemuxFsm};
use crate::{DemuxLimits, ErrorKind, FragmentedMuxFsm, WantedInput};

/// Movie of one track continued in fragments, whose defaults a `trex` states
fn movie() -> MovieBox {
    fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
}

/// One sample of track 1, the first of its fragment
fn sample() -> Sample {
    Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())
}

/// A file of one fragment carrying [`sample`], handed no brands
fn file_of_one_sample() -> Vec<u8> {
    let mut mux_fsm = FragmentedMuxFsm::new();
    let mut file = Vec::new();

    mux_fsm.handle_movie(movie()).unwrap();
    mux_fsm.begin_fragment(1).unwrap();
    mux_fsm.handle_sample(sample()).unwrap();
    mux_fsm.finish_fragment().unwrap();
    mux_fsm.finish().unwrap();
    while let Some(written) = mux_fsm.poll_output() {
        file.extend_from_slice(&written);
    }

    file
}

/// What the demux FSM makes of `file` handed over whole, then declared over
fn read(file: &[u8]) -> Result<MovieDemuxFsm, Error> {
    let mut demux_fsm = MovieDemuxFsm::new();

    demux_fsm.handle_input(0, file)?;
    demux_fsm.finish()?;

    Ok(demux_fsm)
}

#[test]
fn a_file_declaring_no_brands_is_read_all_the_same() {
    let file = [written(&movie()), written(&movie_fragment())].concat();
    let demux_fsm = read(&file).unwrap();

    assert_eq!(demux_fsm.file_type(), None);
    assert_eq!(demux_fsm.movie(), Some(&movie()));
}

#[test]
fn a_file_declared_over_without_a_movie_is_rejected() {
    assert_eq!(
        read(&written(&file_type())).map(drop),
        Err(Error::missing_mandatory_box(MovieBox::BOX_TYPE))
    );
}

#[test]
fn a_box_read_into_a_value_declaring_a_payload_past_the_limit_is_rejected() {
    let mut demux_fsm = MovieDemuxFsm::with_limits(DemuxLimits::new().with_payload(4));

    assert_eq!(
        demux_fsm
            .handle_input(0, &written(&file_type()))
            .map_err(Error::kind),
        Err(ErrorKind::PayloadLimitExceeded)
    );
}

#[test]
fn a_box_passed_over_is_not_bounded_by_the_limit() {
    let movie = written(&movie());
    let file = [
        movie.clone(),
        framed(BoxType::compact(*b"free"), &[0x11; 4_096]),
    ]
    .concat();
    let mut demux_fsm =
        MovieDemuxFsm::with_limits(DemuxLimits::new().with_payload(movie.len() as u64));

    demux_fsm.handle_input(0, &file).unwrap();

    assert_eq!(demux_fsm.finish(), Ok(()));
}

#[test]
fn a_fragment_declaring_more_samples_than_the_limit_lays_out_none() {
    let mut demux_fsm = MovieDemuxFsm::with_limits(DemuxLimits::new().with_resolved_samples(0));

    assert_eq!(
        demux_fsm.handle_input(0, &file_of_one_sample()),
        Err(Error::from(
            isobmff_sample::Error::sample_count_limit_exceeded(1, 0)
        ))
    );
    assert_eq!(demux_fsm.poll_sample(), None);
}

#[test]
fn a_movie_declaring_more_samples_than_the_limit_lays_out_none() {
    let mut demux_fsm = MovieDemuxFsm::with_limits(DemuxLimits::new().with_resolved_samples(0));

    assert_eq!(
        demux_fsm.handle_input(0, &non_fragmented_file(&[&[b"SAMP"]], true)),
        Err(Error::from(
            isobmff_sample::Error::sample_count_limit_exceeded(1, 0)
        ))
    );
    assert_eq!(demux_fsm.poll_sample(), None);
}

#[test]
fn the_sample_reader_is_held_to_the_limits_it_is_given() {
    let mut demux_fsm = MovieDemuxFsm::with_limits(
        DemuxLimits::new().with_sample_reader(SampleReaderLimits::new().with_held_extents(0)),
    );

    assert_eq!(
        demux_fsm.handle_input(0, &file_of_one_sample()),
        Err(Error::from(
            isobmff_sample::Error::held_extent_limit_exceeded(1, 0)
        ))
    );
}

#[test]
fn a_box_read_into_a_value_declaring_no_total_is_read_to_the_end_of_the_file() {
    let mut file = written(&movie());
    file.splice(..4, [0x00, 0x00, 0x00, 0x00]);

    let demux_fsm = read(&file).unwrap();

    assert_eq!(demux_fsm.movie(), Some(&movie()));
}

#[test]
fn the_samples_completed_before_a_framing_failure_are_still_taken() {
    let mut file = file_of_one_sample();
    file.extend_from_slice(b"\0\0\0\x04free");

    let mut demux_fsm = MovieDemuxFsm::new();

    assert_eq!(
        demux_fsm.handle_input(0, &file).map_err(Error::kind),
        Err(ErrorKind::Sequence(isobmff_sequence::ErrorKind::Box(
            isobmff_core::ErrorKind::SizeBelowHeader
        )))
    );
    assert_eq!(
        demux_fsm.poll_sample().map(Sample::into_data),
        Some(b"SAMP".to_vec())
    );
    assert_eq!(demux_fsm.wanted_input(), None);
}

#[test]
fn media_data_the_input_is_still_to_bring_is_not_wanted_and_completes_the_sample_as_it_arrives() {
    let mut file = file_of_one_sample();
    let media_data = file.split_off(file.len().saturating_sub(4));

    let mut demux_fsm = MovieDemuxFsm::new();
    demux_fsm.handle_input(0, &file).unwrap();
    let wanted = demux_fsm.wanted_input();
    demux_fsm
        .handle_input(file.len() as u64, &media_data)
        .unwrap();

    assert_eq!(wanted, Some(WantedInput::new(file.len() as u64, None)));
    assert_eq!(demux_fsm.poll_sample(), Some(sample()));
}

#[test]
fn a_file_declared_over_with_a_sample_short_of_its_bytes_is_rejected() {
    let file = non_fragmented_file(&[&[b"SAMP"]], false);
    let mut demux_fsm = MovieDemuxFsm::new();

    demux_fsm.handle_input(0, &file).unwrap();

    assert_eq!(
        demux_fsm.finish().map_err(Error::kind),
        Err(ErrorKind::Sample(
            isobmff_sample::ErrorKind::UnfinishedSample
        ))
    );
}

#[test]
fn a_failed_demux_fsm_reports_the_same_failure_for_every_call_after_it() {
    let mut demux_fsm = MovieDemuxFsm::new();
    let failure = Error::box_out_of_order(FileTypeBox::BOX_TYPE);
    let file = [written(&file_type()), written(&file_type())].concat();

    assert_eq!(demux_fsm.handle_input(0, &file), Err(failure));
    assert_eq!(demux_fsm.handle_input(0, &written(&movie())), Err(failure));
    assert_eq!(demux_fsm.finish(), Err(failure));
}

#[test]
fn input_handed_over_after_finishing_is_rejected() {
    let mut demux_fsm = read(&written(&movie())).unwrap();

    assert_eq!(
        demux_fsm.handle_input(0, &written(&file_type())),
        Err(Error::already_finished())
    );
    assert_eq!(demux_fsm.finish(), Err(Error::already_finished()));
}

/// The demux FSM handed whole a file whose movie lies after its media data, that file, and the offset of its one sample
fn movie_after_its_media_data() -> (MovieDemuxFsm, Vec<u8>, u64) {
    let file = non_fragmented_file(&[&[b"SAMP"]], false);
    let lacking = file.windows(4).position(|bytes| bytes == b"SAMP").unwrap() as u64;
    let mut demux_fsm = MovieDemuxFsm::new();

    demux_fsm.handle_input(0, &file).unwrap();

    (demux_fsm, file, lacking)
}

#[test]
fn bytes_a_movie_lying_after_its_media_data_lacks_are_wanted_with_their_length() {
    let (demux_fsm, _, lacking) = movie_after_its_media_data();

    assert_eq!(
        demux_fsm.wanted_input(),
        Some(WantedInput::new(lacking, Some(4)))
    );
}

#[test]
fn input_at_the_offset_wanted_completes_the_sample_and_the_continuation_is_wanted_after_it() {
    let (mut demux_fsm, file, lacking) = movie_after_its_media_data();

    demux_fsm.handle_input(lacking, b"SAMP").unwrap();

    assert_eq!(
        demux_fsm.poll_sample().map(Sample::into_data),
        Some(b"SAMP".to_vec())
    );
    assert_eq!(
        demux_fsm.wanted_input(),
        Some(WantedInput::new(file.len() as u64, None))
    );
}

#[test]
fn input_in_order_is_taken_while_bytes_are_wanted() {
    let (mut demux_fsm, file, lacking) = movie_after_its_media_data();

    demux_fsm
        .handle_input(file.len() as u64, &framed(BoxType::compact(*b"free"), &[]))
        .unwrap();

    assert_eq!(
        demux_fsm.wanted_input(),
        Some(WantedInput::new(lacking, Some(4)))
    );
}

#[test]
fn input_at_an_offset_neither_in_order_nor_wanted_is_refused_and_the_file_reads_on() {
    let file = file_of_one_sample();
    let mut demux_fsm = MovieDemuxFsm::new();

    demux_fsm.handle_input(0, file.get(..8).unwrap()).unwrap();

    assert_eq!(
        demux_fsm.handle_input(9, file.get(9..).unwrap()),
        Err(Error::unwanted_input(9))
    );
    assert_eq!(demux_fsm.handle_input(8, file.get(8..).unwrap()), Ok(()));
    assert_eq!(demux_fsm.finish(), Ok(()));
    assert_eq!(demux_fsm.poll_sample(), Some(sample()));
}

#[test]
fn empty_input_is_taken_as_nothing_wherever_it_is_handed_over() {
    let mut demux_fsm = MovieDemuxFsm::new();

    assert_eq!(demux_fsm.handle_input(9, &[]), Ok(()));
    assert_eq!(demux_fsm.wanted_input(), Some(WantedInput::new(0, None)));
}

#[test]
fn the_file_is_wanted_from_the_offset_the_reading_resumed_at() {
    let mut demux_fsm = read(&file_of_one_sample()).unwrap();

    demux_fsm.resume_at(100).unwrap();

    assert_eq!(demux_fsm.wanted_input(), Some(WantedInput::new(100, None)));
}

#[test]
fn nothing_is_wanted_once_the_file_is_declared_over() {
    let demux_fsm = read(&file_of_one_sample()).unwrap();

    assert_eq!(demux_fsm.wanted_input(), None);
}
