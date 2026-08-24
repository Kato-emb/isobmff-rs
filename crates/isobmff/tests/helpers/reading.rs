//! Reading the samples of a file through the box layer beside this one
//!
//! The wiring a caller writes to put the two layers together: a `BoxReader`
//! frames the file, and the two events the sample layer takes are handed on with
//! the extent the box layer reports for them.

use isobmff::{Sample, SampleReader};
use isobmff_sequence::{BoxEvent, BoxReader};

/// The samples `file` carries, read off it `cut_length` bytes at a time
pub(crate) fn samples_of(file: &[u8], cut_length: usize) -> Vec<Sample> {
    let mut box_reader = BoxReader::new();
    let mut sample_reader = None;
    let mut samples = Vec::new();

    for arriving in file.chunks(cut_length) {
        box_reader.handle_input(arriving).unwrap();

        while let Some(event) = box_reader.poll_event() {
            let extent = box_reader.event_extent().unwrap();

            match event {
                BoxEvent::Movie(movie) => {
                    sample_reader = Some(SampleReader::new(&movie).unwrap());
                }
                BoxEvent::MovieFragment(movie_fragment) => sample_reader
                    .as_mut()
                    .unwrap()
                    .handle_movie_fragment(movie_fragment, extent)
                    .unwrap(),
                BoxEvent::RawPayload(payload) => sample_reader
                    .as_mut()
                    .unwrap()
                    .handle_media_data(&payload, extent)
                    .unwrap(),
                // Why not the wildcard on its own: `BoxEvent` is
                // `#[non_exhaustive]`, so the arm cannot go, and
                // `wildcard_enum_match_arm` refuses one that stands for variants
                // this match could have named.
                BoxEvent::FileType(_)
                | BoxEvent::SegmentType(_)
                | BoxEvent::RawStart(_)
                | BoxEvent::RawEnd
                | _ => {}
            }

            while let Some(sample) = sample_reader.as_mut().and_then(SampleReader::poll_sample) {
                samples.push(sample);
            }
        }
    }

    box_reader.finish().unwrap();
    sample_reader.unwrap().finish().unwrap();

    samples
}
