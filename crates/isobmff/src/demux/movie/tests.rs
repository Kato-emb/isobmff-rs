use alloc::vec::Vec;

use isobmff_boxes::{MovieBox, SampleFlags, TrackExtendsBox};
use isobmff_core::{BoxDefinition, BoxType};
use isobmff_test_support::{
    file_type, fragmented_movie, framed, movie_fragment, non_fragmented_file, written,
};

use super::{DemuxError, MovieDemuxer};
use crate::sample::{Sample, SampleExtent, SampleProperties, SampleReaderLimits};
use crate::{DemuxLimits, FragmentedMuxer, SampleExtents, WantedInput};

/// Movie of one track continued in fragments, whose defaults a `trex` states
fn movie() -> MovieBox {
    fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))
}

/// One sample of track 1, the first of its fragment
fn sample() -> Sample {
    Sample::new(
        SampleProperties {
            track_id: 1,
            decode_time: 0,
            sample_duration: 1_024,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::ZERO,
            sample_description_index: 1,
        },
        b"SAMP".to_vec(),
    )
}

/// A file of one fragment carrying [`sample`], handed no brands
fn file_of_one_sample() -> Vec<u8> {
    let mut muxer = FragmentedMuxer::new();
    let mut file = Vec::new();

    muxer.handle_movie(movie()).unwrap();
    muxer.begin_fragment(1).unwrap();
    muxer.handle_sample(sample()).unwrap();
    muxer.finish_fragment().unwrap();
    muxer.finish().unwrap();
    while let Some(written) = muxer.poll_output() {
        file.extend_from_slice(&written);
    }

    file
}

/// What the demuxer makes of `file` handed over whole, then declared over
fn read(file: &[u8]) -> Result<MovieDemuxer, DemuxError> {
    let mut demuxer = MovieDemuxer::new();

    demuxer.handle_input(0, file)?;
    demuxer.finish()?;

    Ok(demuxer)
}

#[test]
fn a_file_declaring_no_brands_is_read_all_the_same() {
    let file = [written(&movie()), written(&movie_fragment())].concat();
    let demuxer = read(&file).unwrap();

    assert_eq!(demuxer.file_type(), None);
    assert_eq!(demuxer.movie(), Some(&movie()));
}

#[test]
fn a_file_declared_over_without_a_movie_is_rejected() {
    assert_eq!(
        read(&written(&file_type())).map(drop),
        Err(DemuxError::MissingMandatoryBox {
            box_type: MovieBox::BOX_TYPE
        })
    );
}

#[test]
fn a_box_read_into_a_value_declaring_a_payload_past_the_limit_is_rejected() {
    let mut demuxer = MovieDemuxer::with_limits(DemuxLimits::new().with_payload(4));

    assert_eq!(
        demuxer.handle_input(0, &written(&file_type())),
        Err(DemuxError::PayloadLimitExceeded {
            box_type: BoxType::compact(*b"ftyp"),
            reached_bytes: 16,
            limit_bytes: 4,
        })
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
    let mut demuxer =
        MovieDemuxer::with_limits(DemuxLimits::new().with_payload(movie.len() as u64));

    demuxer.handle_input(0, &file).unwrap();

    assert_eq!(demuxer.finish(), Ok(()));
}

#[test]
fn a_fragment_declaring_more_samples_than_the_limit_lays_out_none() {
    let mut demuxer = MovieDemuxer::with_limits(DemuxLimits::new().with_resolved_samples(0));

    assert!(matches!(
        demuxer.handle_input(0, &file_of_one_sample()),
        Err(DemuxError::Sample {
            error: crate::sample::Error::SampleCountLimitExceeded {
                declared_samples: 1,
                limit_samples: 0,
                ..
            }
        })
    ));
    assert_eq!(demuxer.poll_output(), None);
}

#[test]
fn a_movie_declaring_more_samples_than_the_limit_lays_out_none() {
    let mut demuxer = MovieDemuxer::with_limits(DemuxLimits::new().with_resolved_samples(0));

    assert!(matches!(
        demuxer.handle_input(0, &non_fragmented_file(&[&[b"SAMP"]], true)),
        Err(DemuxError::Sample {
            error: crate::sample::Error::SampleCountLimitExceeded {
                declared_samples: 1,
                limit_samples: 0,
                ..
            }
        })
    ));
    assert_eq!(demuxer.poll_output(), None);
}

#[test]
fn the_sample_reader_is_held_to_the_limits_it_is_given() {
    let mut demuxer = MovieDemuxer::with_limits(
        DemuxLimits::new().with_sample_reader(SampleReaderLimits::new().with_held_extents(0)),
    );

    assert!(matches!(
        demuxer.handle_input(0, &file_of_one_sample()),
        Err(DemuxError::Sample {
            error: crate::sample::Error::HeldExtentLimitExceeded {
                held_extents: 1,
                limit_extents: 0,
                ..
            }
        })
    ));
}

#[test]
fn a_box_read_into_a_value_declaring_no_total_is_read_to_the_end_of_the_file() {
    let mut file = written(&movie());
    file.splice(..4, [0x00, 0x00, 0x00, 0x00]);

    let demuxer = read(&file).unwrap();

    assert_eq!(demuxer.movie(), Some(&movie()));
}

#[test]
fn media_data_the_input_is_still_to_bring_is_not_wanted_and_completes_the_sample_as_it_arrives() {
    let mut file = file_of_one_sample();
    let media_data = file.split_off(file.len().saturating_sub(4));

    let mut demuxer = MovieDemuxer::new();
    demuxer.handle_input(0, &file).unwrap();
    let wanted = demuxer.wanted_input();
    demuxer
        .handle_input(file.len() as u64, &media_data)
        .unwrap();

    assert_eq!(wanted, Some(WantedInput::new(file.len() as u64, None)));
    assert_eq!(demuxer.poll_output(), Some(sample()));
}

#[test]
fn the_file_is_wanted_from_the_offset_the_reading_resumed_at() {
    let mut demuxer = read(&file_of_one_sample()).unwrap();

    demuxer.resume_at(100).unwrap();

    assert_eq!(demuxer.wanted_input(), Some(WantedInput::new(100, None)));
}

#[test]
fn the_extents_held_are_dropped_when_the_reading_resumes() {
    let file = file_of_one_sample();
    let media_data_start = file.len().saturating_sub(4) as u64;
    let mut read_on = MovieDemuxer::with_gathering(SampleExtents::new(), DemuxLimits::new());
    let mut resumed = MovieDemuxer::with_gathering(SampleExtents::new(), DemuxLimits::new());
    read_on.handle_input(0, &file).unwrap();
    resumed.handle_input(0, &file).unwrap();

    resumed.resume_at(100).unwrap();

    assert_eq!(
        (read_on.poll_output(), resumed.poll_output()),
        (
            Some(SampleExtent::new(
                *sample().properties(),
                1,
                media_data_start..media_data_start.saturating_add(4),
            )),
            None
        )
    );
}
