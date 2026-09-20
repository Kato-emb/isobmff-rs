//! [`SampleError`] as a caller reads it: the values a failure carries, and how it prints

use core::error;
use core::fmt;

use crate::error::kind::Representation;
use crate::error::{FURTHEST_DATA_OFFSET, LARGEST_SAMPLE_SIZE, SampleError};

impl SampleError {
    /// Returns the failure of one box carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        match self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the track the failure is about, for the kinds that name one
    #[must_use]
    pub const fn track_id(self) -> Option<u32> {
        match self.representation {
            Representation::DecodeTimeOverflow { track_id }
            | Representation::DataOffsetOverflow { track_id }
            | Representation::UnknownTrackId { track_id }
            | Representation::UnknownSampleDescriptionIndex { track_id, .. }
            | Representation::UnknownDataReferenceIndex { track_id, .. }
            | Representation::ExternalDataReference { track_id, .. }
            | Representation::SampleCountMismatch { track_id }
            | Representation::FirstChunkOutOfRange { track_id, .. }
            | Representation::SampleSizeLimitExceeded { track_id, .. }
            | Representation::UnfinishedSample { track_id, .. }
            | Representation::SampleSizeOutOfRange { track_id, .. }
            | Representation::DataOffsetOutOfRange { track_id, .. }
            | Representation::CompositionTimeOffsetOutOfRange { track_id, .. }
            | Representation::DecodeTimeMismatch { track_id, .. }
            | Representation::BackwardDecodeTime { track_id, .. }
            | Representation::SampleDescriptionIndexMismatch { track_id, .. } => Some(track_id),
            Representation::Box(_)
            | Representation::MissingMovieExtends
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the `stsd` entry the failure names, for the kinds that name one
    #[must_use]
    pub const fn sample_description_index(self) -> Option<u32> {
        match self.representation {
            Representation::UnknownSampleDescriptionIndex {
                sample_description_index,
                ..
            }
            | Representation::SampleDescriptionIndexMismatch {
                stated: sample_description_index,
                ..
            } => Some(sample_description_index),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the `dref` entry the failure names, for the kinds that name one
    #[must_use]
    pub const fn data_reference_index(self) -> Option<u16> {
        match self.representation {
            Representation::UnknownDataReferenceIndex {
                data_reference_index,
                ..
            }
            | Representation::ExternalDataReference {
                data_reference_index,
                ..
            } => Some(data_reference_index),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the chunk a run of chunks starts at, counted from one, for the kinds that name one
    #[must_use]
    pub const fn first_chunk(self) -> Option<u32> {
        match self.representation {
            Representation::FirstChunkOutOfRange { first_chunk, .. } => Some(first_chunk),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::SampleSizeLimitExceeded { declared, .. }
            | Representation::SampleSizeOutOfRange { declared, .. } => Some(declared),
            Representation::UnfinishedSample { needed, .. } => Some(needed),
            Representation::DataOffsetOutOfRange { offset, .. } => Some(offset),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        match self.representation {
            Representation::SampleSizeLimitExceeded { limit, .. } => Some(limit),
            Representation::UnfinishedSample { available, .. } => Some(available),
            Representation::SampleSizeOutOfRange { .. } => Some(LARGEST_SAMPLE_SIZE),
            Representation::DataOffsetOutOfRange { .. } => Some(FURTHEST_DATA_OFFSET),
            Representation::Box(_)
            | Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }
}

impl fmt::Display for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.representation {
            Representation::Box(box_error) => write!(formatter, "{box_error}"),
            Representation::DecodeTimeOverflow { track_id } => write!(
                formatter,
                "decode time of track {track_id} runs past what 64 bits carry"
            ),
            Representation::DataOffsetOverflow { track_id } => write!(
                formatter,
                "data offset of track {track_id} runs past what 64 bits carry"
            ),
            Representation::UnknownTrackId { track_id } => {
                write!(formatter, "movie declares no track {track_id}")
            }
            Representation::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => write!(
                formatter,
                "track {track_id} has no stsd entry {sample_description_index}"
            ),
            Representation::MissingMovieExtends => formatter.write_str("the movie carries no mvex"),
            Representation::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "track {track_id} has no dref entry {data_reference_index}"
            ),
            Representation::ExternalDataReference {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "dref entry {data_reference_index} of track {track_id} names an external file"
            ),
            Representation::SampleCountMismatch { track_id } => write!(
                formatter,
                "sample tables of track {track_id} count different numbers of samples"
            ),
            Representation::FirstChunkOutOfRange {
                track_id,
                first_chunk,
            } => write!(
                formatter,
                "run of chunks of track {track_id} starts at chunk {first_chunk}, out of the range open to it"
            ),
            Representation::SampleSizeLimitExceeded {
                track_id,
                declared,
                limit,
            } => write!(
                formatter,
                "track {track_id} declares a sample of {declared} bytes, past the {limit}-byte limit"
            ),
            Representation::UnfinishedSample {
                track_id,
                needed,
                available,
            } => write!(
                formatter,
                "sample of track {track_id} takes {needed} bytes, and {available} arrived"
            ),
            Representation::AlreadyFinished => {
                formatter.write_str("samples were declared over and take nothing more")
            }
            Representation::NoFragmentOpen => {
                formatter.write_str("no fragment is open to carry a sample or be closed")
            }
            Representation::FragmentStillOpen => formatter.write_str("fragment is still open"),
            Representation::SampleSizeOutOfRange { track_id, declared } => write!(
                formatter,
                "track {track_id} states a sample of {declared} bytes, past the {LARGEST_SAMPLE_SIZE} a trun row carries"
            ),
            Representation::DataOffsetOutOfRange { track_id, offset } => write!(
                formatter,
                "sample data of track {track_id} lies {offset} bytes into the fragment, past the {FURTHEST_DATA_OFFSET} a trun carries"
            ),
            Representation::CompositionTimeOffsetOutOfRange { track_id, offset } => write!(
                formatter,
                "track {track_id} states a composition time offset of {offset}, which neither version of a trun writes"
            ),
            Representation::DecodeTimeMismatch {
                track_id,
                stated,
                reached,
            } => write!(
                formatter,
                "track {track_id} states decode time {stated} where the samples before it reach {reached}"
            ),
            Representation::BackwardDecodeTime {
                track_id,
                stated,
                reached,
            } => write!(
                formatter,
                "track {track_id} goes back to decode time {stated} after reaching {reached}"
            ),
            Representation::SampleDescriptionIndexMismatch {
                track_id,
                stated,
                established,
            } => write!(
                formatter,
                "track {track_id} describes a sample by stsd entry {stated} in a fragment describing it by {established}"
            ),
            Representation::FragmentNotRepresentable => {
                formatter.write_str("samples do not build the boxes of a fragment")
            }
        }
    }
}

impl fmt::Debug for SampleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = formatter.debug_struct("SampleError");
        fields.field("kind", &self.kind());
        fields.field("category", &self.category());

        if let Some(box_error) = self.box_error() {
            fields.field("box_error", &box_error);
        }
        if let Some(track_id) = self.track_id() {
            fields.field("track_id", &track_id);
        }
        if let Some(sample_description_index) = self.sample_description_index() {
            fields.field("sample_description_index", &sample_description_index);
        }
        if let Some(data_reference_index) = self.data_reference_index() {
            fields.field("data_reference_index", &data_reference_index);
        }
        if let Some(first_chunk) = self.first_chunk() {
            fields.field("first_chunk", &first_chunk);
        }
        if let Some(needed) = self.needed_bytes() {
            fields.field("needed_bytes", &needed);
        }
        if let Some(available) = self.available_bytes() {
            fields.field("available_bytes", &available);
        }

        fields.finish()
    }
}

impl error::Error for SampleError {
    /// Returns the failure of one box carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.representation {
            Representation::Box(box_error) => Some(box_error),
            Representation::DecodeTimeOverflow { .. }
            | Representation::DataOffsetOverflow { .. }
            | Representation::UnknownTrackId { .. }
            | Representation::UnknownSampleDescriptionIndex { .. }
            | Representation::MissingMovieExtends
            | Representation::UnknownDataReferenceIndex { .. }
            | Representation::ExternalDataReference { .. }
            | Representation::SampleCountMismatch { .. }
            | Representation::FirstChunkOutOfRange { .. }
            | Representation::SampleSizeLimitExceeded { .. }
            | Representation::UnfinishedSample { .. }
            | Representation::AlreadyFinished
            | Representation::NoFragmentOpen
            | Representation::FragmentStillOpen
            | Representation::SampleSizeOutOfRange { .. }
            | Representation::DataOffsetOutOfRange { .. }
            | Representation::CompositionTimeOffsetOutOfRange { .. }
            | Representation::DecodeTimeMismatch { .. }
            | Representation::BackwardDecodeTime { .. }
            | Representation::SampleDescriptionIndexMismatch { .. }
            | Representation::FragmentNotRepresentable => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use isobmff_core::BoxType;

    use crate::error::SampleError;

    #[test]
    fn a_failure_carries_only_the_values_its_kind_names() {
        let error = SampleError::unknown_sample_description_index(2, 7);

        assert_eq!(error.track_id(), Some(2));
        assert_eq!(error.sample_description_index(), Some(7));
        assert_eq!(error.box_error(), None);
        assert_eq!(error.needed_bytes(), None);

        let unfinished = SampleError::unfinished_sample(2, 1_024, 512);

        assert_eq!(unfinished.needed_bytes(), Some(1_024));
        assert_eq!(unfinished.available_bytes(), Some(512));
        assert_eq!(unfinished.sample_description_index(), None);
        assert_eq!(SampleError::missing_movie_extends().track_id(), None);
        assert_eq!(
            SampleError::unknown_track_id(3).sample_description_index(),
            None
        );

        let external = SampleError::external_data_reference(1, 2);

        assert_eq!(external.track_id(), Some(1));
        assert_eq!(external.data_reference_index(), Some(2));
        assert_eq!(external.first_chunk(), None);
        assert_eq!(
            SampleError::first_chunk_out_of_range(1, 3).first_chunk(),
            Some(3)
        );
        assert_eq!(
            SampleError::sample_count_mismatch(1).data_reference_index(),
            None
        );

        let too_long = SampleError::sample_size_out_of_range(1, 1 << 40);

        assert_eq!(too_long.track_id(), Some(1));
        assert_eq!(too_long.needed_bytes(), Some(1 << 40));
        assert_eq!(too_long.available_bytes(), Some(u64::from(u32::MAX)));
        assert_eq!(
            SampleError::sample_description_index_mismatch(1, 2, 1).sample_description_index(),
            Some(2)
        );
        assert_eq!(SampleError::no_fragment_open().track_id(), None);
    }

    #[test]
    fn a_failure_of_one_box_keeps_its_values_and_the_boxes_it_was_reached_through() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"))
            .in_container(BoxType::compact(*b"mvex"));
        let carried = SampleError::from(box_error);

        assert_eq!(carried.box_error(), Some(box_error));
        assert_eq!(carried.track_id(), None);
    }

    #[test]
    fn display_of_a_failure_of_the_samples_states_the_reason() {
        assert_eq!(
            SampleError::decode_time_overflow(1).to_string(),
            "decode time of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            SampleError::data_offset_overflow(1).to_string(),
            "data offset of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            SampleError::unknown_track_id(3).to_string(),
            "movie declares no track 3"
        );
        assert_eq!(
            SampleError::unknown_sample_description_index(2, 7).to_string(),
            "track 2 has no stsd entry 7"
        );
        assert_eq!(
            SampleError::missing_movie_extends().to_string(),
            "the movie carries no mvex"
        );
        assert_eq!(
            SampleError::unknown_data_reference_index(2, 3).to_string(),
            "track 2 has no dref entry 3"
        );
        assert_eq!(
            SampleError::external_data_reference(2, 3).to_string(),
            "dref entry 3 of track 2 names an external file"
        );
        assert_eq!(
            SampleError::sample_count_mismatch(2).to_string(),
            "sample tables of track 2 count different numbers of samples"
        );
        assert_eq!(
            SampleError::first_chunk_out_of_range(2, 5).to_string(),
            "run of chunks of track 2 starts at chunk 5, out of the range open to it"
        );
        assert_eq!(
            SampleError::sample_size_limit_exceeded(1, 32, 16).to_string(),
            "track 1 declares a sample of 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            SampleError::unfinished_sample(2, 1_024, 512).to_string(),
            "sample of track 2 takes 1024 bytes, and 512 arrived"
        );
        assert_eq!(
            SampleError::already_finished().to_string(),
            "samples were declared over and take nothing more"
        );
        assert_eq!(
            SampleError::no_fragment_open().to_string(),
            "no fragment is open to carry a sample or be closed"
        );
        assert_eq!(
            SampleError::fragment_still_open().to_string(),
            "fragment is still open"
        );
        assert_eq!(
            SampleError::sample_size_out_of_range(1, 1 << 40).to_string(),
            "track 1 states a sample of 1099511627776 bytes, past the 4294967295 a trun row carries"
        );
        assert_eq!(
            SampleError::data_offset_out_of_range(1, 1 << 40).to_string(),
            "sample data of track 1 lies 1099511627776 bytes into the fragment, past the 2147483647 a trun carries"
        );
        assert_eq!(
            SampleError::composition_time_offset_out_of_range(1, 1 << 40).to_string(),
            "track 1 states a composition time offset of 1099511627776, which neither version of a trun writes"
        );
        assert_eq!(
            SampleError::decode_time_mismatch(1, 512, 1_024).to_string(),
            "track 1 states decode time 512 where the samples before it reach 1024"
        );
        assert_eq!(
            SampleError::backward_decode_time(1, 512, 1_024).to_string(),
            "track 1 goes back to decode time 512 after reaching 1024"
        );
        assert_eq!(
            SampleError::sample_description_index_mismatch(1, 2, 1).to_string(),
            "track 1 describes a sample by stsd entry 2 in a fragment describing it by 1"
        );
        assert_eq!(
            SampleError::fragment_not_representable().to_string(),
            "samples do not build the boxes of a fragment"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"mvex"));

        assert_eq!(
            SampleError::from(box_error).to_string(),
            box_error.to_string()
        );
    }

    #[test]
    fn debug_names_the_values_a_kind_carries_and_leaves_out_the_rest() {
        assert_eq!(
            format!("{:?}", SampleError::decode_time_overflow(1)),
            "SampleError { kind: DecodeTimeOverflow, category: Malformed, track_id: 1 }"
        );
        assert_eq!(
            format!("{:?}", SampleError::missing_movie_extends()),
            "SampleError { kind: MissingMovieExtends, category: Malformed }"
        );
        assert_eq!(
            format!("{:?}", SampleError::sample_size_limit_exceeded(1, 32, 16)),
            "SampleError { kind: SampleSizeLimitExceeded, category: Unsupported, track_id: 1, needed_bytes: 32, available_bytes: 16 }"
        );
        assert_eq!(
            format!("{:?}", SampleError::external_data_reference(1, 2)),
            "SampleError { kind: ExternalDataReference, category: Unsupported, track_id: 1, data_reference_index: 2 }"
        );
        assert_eq!(
            format!("{:?}", SampleError::first_chunk_out_of_range(1, 5)),
            "SampleError { kind: FirstChunkOutOfRange, category: Malformed, track_id: 1, first_chunk: 5 }"
        );
    }
}
