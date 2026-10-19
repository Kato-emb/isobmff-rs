//! The extents a movie file declares, yielded by a demuxer gathering none of its samples, against the samples one gathering them reads

#[cfg(test)]
mod tests {
    use isobmff::sample::{Sample, SampleExtent};
    use isobmff::{DemuxError, DemuxLimits, MovieDemuxer, SampleExtents, WantedInput};
    use isobmff_boxes::MovieFragmentBox;
    use isobmff_core::BoxDefinition;
    use isobmff_test_support::non_fragmented_file;

    use crate::presentations::{
        SAMPLE_CHUNKS, fragmented_file_samples, fragmented_file_with_movie_samples,
        fragmented_file_with_samples, segment_file_with_samples,
    };
    use crate::reading::samples_of;

    /// Hands `file` over in order in cuts of seven bytes, taking every extent as it is resolved
    fn extents_of(file: &[u8]) -> Result<Vec<SampleExtent>, DemuxError> {
        let mut demuxer = MovieDemuxer::with_gathering(SampleExtents::new(), DemuxLimits::new());
        let mut extents = Vec::new();
        for (offset, arriving) in (0..).step_by(7).zip(file.chunks(7)) {
            demuxer.handle_input(offset, arriving)?;
            extents.extend(core::iter::from_fn(|| demuxer.poll_output()));
        }
        demuxer.finish()?;
        extents.extend(core::iter::from_fn(|| demuxer.poll_output()));

        Ok(extents)
    }

    /// The samples `extents` name in `file`, each built from what its extent states and the bytes it spans
    fn named_in(file: &[u8], extents: &[SampleExtent]) -> Vec<Sample> {
        extents
            .iter()
            .map(|extent| {
                let bytes = extent.extent();
                let span =
                    usize::try_from(bytes.start).unwrap()..usize::try_from(bytes.end).unwrap();
                Sample::new(*extent.properties(), file.get(span).unwrap().to_vec())
            })
            .collect()
    }

    #[test]
    fn the_extents_of_a_movie_name_the_samples_a_demuxer_reads_wherever_its_media_data_lies() {
        for movie_first in [true, false] {
            let file = non_fragmented_file(&SAMPLE_CHUNKS, movie_first);

            let extents = extents_of(&file).unwrap();

            assert_eq!(named_in(&file, &extents), samples_of(&file, usize::MAX));
        }
    }

    #[test]
    fn a_movie_lying_after_its_media_data_wants_nothing_but_the_continuation() {
        let file = non_fragmented_file(&SAMPLE_CHUNKS, false);
        let mut demuxer = MovieDemuxer::with_gathering(SampleExtents::new(), DemuxLimits::new());

        demuxer.handle_input(0, &file).unwrap();

        assert_eq!(
            demuxer.wanted_input(),
            Some(WantedInput::new(u64::try_from(file.len()).unwrap(), None))
        );
    }

    #[test]
    fn the_extents_of_a_fragment_name_its_samples() {
        let file = fragmented_file_with_samples();

        let extents = extents_of(&file).unwrap();

        assert_eq!(named_in(&file, &extents), fragmented_file_samples());
    }

    #[test]
    fn the_extents_of_each_fragment_follow_those_of_the_movie_wherever_its_media_data_lies() {
        for movie_first in [true, false] {
            let file = fragmented_file_with_movie_samples(movie_first, true);

            let extents = extents_of(&file.bytes).unwrap();

            assert_eq!(
                named_in(&file.bytes, &extents),
                [file.movie_samples, file.fragment_samples].concat()
            );
        }
    }

    #[test]
    fn a_media_segment_on_its_own_is_refused_as_a_fragment_before_the_movie() {
        let refused = extents_of(&segment_file_with_samples());

        assert!(matches!(
            refused,
            Err(DemuxError::BoxOutOfOrder { box_type, .. }) if box_type == MovieFragmentBox::BOX_TYPE
        ));
    }
}
