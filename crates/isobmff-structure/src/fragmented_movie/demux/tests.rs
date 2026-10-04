use isobmff_boxes::{FileTypeBox, MovieBox, SampleFlags, TrackExtendsBox};
use isobmff_core::{BoxDefinition, BoxType};
use isobmff_sample::{Sample, SampleReader};
use isobmff_test_support::{file_type, fragmented_movie, framed, movie_fragment, written};

use super::super::tests::{file_of_one_sample, sample};
use super::{Error, FragmentedDemuxFsm};
use crate::ErrorKind;
use crate::WantedInput;

/// Movie of one track continued in fragments, whose defaults a `trex` states
fn movie() -> MovieBox {
    fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
}

/// What the demux FSM makes of `file` handed over whole, then declared over
fn read(file: &[u8]) -> Result<FragmentedDemuxFsm, Error> {
    let mut demux_fsm = FragmentedDemuxFsm::new();

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
    let mut demux_fsm = FragmentedDemuxFsm::with_limits(4, SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT);

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
    let mut demux_fsm = FragmentedDemuxFsm::with_limits(
        movie.len() as u64,
        SampleReader::DEFAULT_SAMPLE_SIZE_LIMIT,
    );

    demux_fsm.handle_input(0, &file).unwrap();

    assert_eq!(demux_fsm.finish(), Ok(()));
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

    let mut demux_fsm = FragmentedDemuxFsm::new();

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

    let mut demux_fsm = FragmentedDemuxFsm::new();
    demux_fsm.handle_input(0, &file).unwrap();
    let wanted = demux_fsm.wanted_input();
    demux_fsm
        .handle_input(file.len() as u64, &media_data)
        .unwrap();

    assert_eq!(wanted, Some(WantedInput::new(file.len() as u64, None)));
    assert_eq!(demux_fsm.poll_sample(), Some(sample()));
}

#[test]
fn a_failed_demux_fsm_reports_the_same_failure_for_every_call_after_it() {
    let mut demux_fsm = FragmentedDemuxFsm::new();
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

#[test]
fn input_at_an_offset_neither_in_order_nor_wanted_is_refused_and_the_file_reads_on() {
    let file = file_of_one_sample();
    let mut demux_fsm = FragmentedDemuxFsm::new();

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
    let mut demux_fsm = FragmentedDemuxFsm::new();

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
