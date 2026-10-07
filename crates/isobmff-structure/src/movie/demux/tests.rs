use alloc::vec::Vec;

use isobmff_boxes::{MovieBox, SampleFlags, TrackExtendsBox};
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

    assert!(matches!(
        demux_fsm
            .handle_input(0, &file_of_one_sample())
            .map_err(Error::kind),
        Err(ErrorKind::Sample(
            isobmff_sample::Error::SampleCountLimitExceeded {
                declared_samples: 1,
                limit_samples: 0,
                ..
            }
        ))
    ));
    assert_eq!(demux_fsm.poll_sample(), None);
}

#[test]
fn a_movie_declaring_more_samples_than_the_limit_lays_out_none() {
    let mut demux_fsm = MovieDemuxFsm::with_limits(DemuxLimits::new().with_resolved_samples(0));

    assert!(matches!(
        demux_fsm
            .handle_input(0, &non_fragmented_file(&[&[b"SAMP"]], true))
            .map_err(Error::kind),
        Err(ErrorKind::Sample(
            isobmff_sample::Error::SampleCountLimitExceeded {
                declared_samples: 1,
                limit_samples: 0,
                ..
            }
        ))
    ));
    assert_eq!(demux_fsm.poll_sample(), None);
}

#[test]
fn the_sample_reader_is_held_to_the_limits_it_is_given() {
    let mut demux_fsm = MovieDemuxFsm::with_limits(
        DemuxLimits::new().with_sample_reader(SampleReaderLimits::new().with_held_extents(0)),
    );

    assert!(matches!(
        demux_fsm
            .handle_input(0, &file_of_one_sample())
            .map_err(Error::kind),
        Err(ErrorKind::Sample(
            isobmff_sample::Error::HeldExtentLimitExceeded {
                needed_extents: 1,
                limit_extents: 0,
                ..
            }
        ))
    ));
}

#[test]
fn a_box_read_into_a_value_declaring_no_total_is_read_to_the_end_of_the_file() {
    let mut file = written(&movie());
    file.splice(..4, [0x00, 0x00, 0x00, 0x00]);

    let demux_fsm = read(&file).unwrap();

    assert_eq!(demux_fsm.movie(), Some(&movie()));
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
fn the_file_is_wanted_from_the_offset_the_reading_resumed_at() {
    let mut demux_fsm = read(&file_of_one_sample()).unwrap();

    demux_fsm.resume_at(100).unwrap();

    assert_eq!(demux_fsm.wanted_input(), Some(WantedInput::new(100, None)));
}
