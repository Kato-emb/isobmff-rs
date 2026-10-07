//! [`Error`] as it is reported: the line it prints and the failure of one box it carries

use core::error;
use core::fmt;

use crate::error::Error;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Box { error } => write!(formatter, "{error}"),
            Self::DecodeTimeOverflow { track_id } => write!(
                formatter,
                "decode time of track {track_id} runs past what 64 bits carry"
            ),
            Self::PresentationTimeOverflow { track_id } => write!(
                formatter,
                "presentation time of track {track_id} runs past what 64 bits carry"
            ),
            Self::MissingDecodeTime { track_id } => write!(
                formatter,
                "fragment of track {track_id} states no decode time, and where the track stands is not known"
            ),
            Self::DataOffsetOverflow { track_id } => write!(
                formatter,
                "data offset of track {track_id} runs past what 64 bits carry"
            ),
            Self::UnknownTrackId { track_id } => {
                write!(formatter, "movie declares no track {track_id}")
            }
            Self::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => write!(
                formatter,
                "track {track_id} has no stsd entry {sample_description_index}"
            ),
            Self::MissingMovieExtends => formatter.write_str("the movie carries no mvex"),
            Self::SampleTableNotEmpty { track_id } => write!(
                formatter,
                "sample table of track {track_id} lays samples out in a movie continued in fragments"
            ),
            Self::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "track {track_id} has no dref entry {data_reference_index}"
            ),
            Self::ExternalDataReference {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "dref entry {data_reference_index} of track {track_id} names an external file"
            ),
            Self::SampleCountMismatch { track_id } => write!(
                formatter,
                "sample tables of track {track_id} count different numbers of samples"
            ),
            Self::FirstChunkOutOfRange {
                track_id,
                first_chunk,
            } => write!(
                formatter,
                "run of chunks of track {track_id} starts at chunk {first_chunk}, out of the range open to it"
            ),
            Self::SyncSampleOutOfRange {
                track_id,
                sample_number,
            } => write!(
                formatter,
                "sync sample {sample_number} of track {track_id} is listed out of order or past its samples"
            ),
            Self::SampleSizeLimitExceeded {
                track_id,
                declared_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "track {track_id} declares a sample of {declared_bytes} bytes, past the {limit_bytes}-byte limit"
            ),
            Self::SampleCountLimitExceeded {
                declared_samples,
                limit_samples,
            } => write!(
                formatter,
                "boxes declare {declared_samples} samples, past the {limit_samples}-sample limit"
            ),
            Self::HeldExtentLimitExceeded {
                needed_extents,
                limit_extents,
            } => write!(
                formatter,
                "reader would hold {needed_extents} extents, past the {limit_extents}-extent limit"
            ),
            Self::HeldBytesLimitExceeded {
                needed_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "reader would hold {needed_bytes} bytes, past the {limit_bytes}-byte limit"
            ),
            Self::UnfinishedSample {
                track_id,
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "sample of track {track_id} takes {needed_bytes} bytes, and {available_bytes} arrived"
            ),
            Self::AlreadyFinished => {
                formatter.write_str("samples were declared over and take nothing more")
            }
            Self::NoFragmentOpen => {
                formatter.write_str("no fragment is open to carry a sample or be closed")
            }
            Self::FragmentStillOpen => formatter.write_str("fragment is still open"),
            Self::SampleSizeOutOfRange {
                track_id,
                declared_bytes,
            } => write!(
                formatter,
                "track {track_id} states a sample of {declared_bytes} bytes, past the {} a trun row or an stsz entry carries",
                u32::MAX
            ),
            Self::DataOffsetOutOfRange {
                track_id,
                data_offset,
            } => write!(
                formatter,
                "sample data of track {track_id} lies {data_offset} bytes into the fragment, past the {} a trun carries",
                i32::MAX
            ),
            Self::CompositionTimeOffsetOutOfRange {
                track_id,
                composition_time_offset,
            } => write!(
                formatter,
                "track {track_id} states a composition time offset of {composition_time_offset}, outside what 32 signed bits hold"
            ),
            Self::DecodeTimeMismatch {
                track_id,
                stated_decode_time,
                reached_decode_time,
            } => write!(
                formatter,
                "track {track_id} states decode time {stated_decode_time} where the samples before it reach {reached_decode_time}"
            ),
            Self::BackwardDecodeTime {
                track_id,
                stated_decode_time,
                reached_decode_time,
            } => write!(
                formatter,
                "track {track_id} goes back to decode time {stated_decode_time} after reaching {reached_decode_time}"
            ),
            Self::SampleDescriptionIndexMismatch {
                track_id,
                stated_sample_description_index,
                established_sample_description_index,
            } => write!(
                formatter,
                "track {track_id} describes a sample by stsd entry {stated_sample_description_index} in a fragment or a chunk describing it by {established_sample_description_index}"
            ),
            Self::NoChunkOpen => formatter.write_str("no chunk is open to carry a sample"),
            Self::TrackIdMismatch {
                stated_track_id,
                established_track_id,
            } => write!(
                formatter,
                "sample of track {stated_track_id} handed over to a chunk of track {established_track_id}"
            ),
        }
    }
}

impl error::Error for Error {
    /// Returns the failure of one box carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Box { error } => Some(error),
            Self::DecodeTimeOverflow { .. }
            | Self::PresentationTimeOverflow { .. }
            | Self::MissingDecodeTime { .. }
            | Self::DataOffsetOverflow { .. }
            | Self::UnknownTrackId { .. }
            | Self::UnknownSampleDescriptionIndex { .. }
            | Self::MissingMovieExtends
            | Self::SampleTableNotEmpty { .. }
            | Self::UnknownDataReferenceIndex { .. }
            | Self::ExternalDataReference { .. }
            | Self::SampleCountMismatch { .. }
            | Self::FirstChunkOutOfRange { .. }
            | Self::SyncSampleOutOfRange { .. }
            | Self::SampleSizeLimitExceeded { .. }
            | Self::SampleCountLimitExceeded { .. }
            | Self::HeldExtentLimitExceeded { .. }
            | Self::HeldBytesLimitExceeded { .. }
            | Self::UnfinishedSample { .. }
            | Self::AlreadyFinished
            | Self::NoFragmentOpen
            | Self::FragmentStillOpen
            | Self::SampleSizeOutOfRange { .. }
            | Self::DataOffsetOutOfRange { .. }
            | Self::CompositionTimeOffsetOutOfRange { .. }
            | Self::DecodeTimeMismatch { .. }
            | Self::BackwardDecodeTime { .. }
            | Self::SampleDescriptionIndexMismatch { .. }
            | Self::NoChunkOpen
            | Self::TrackIdMismatch { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use isobmff_core::BoxType;

    use crate::error::Error;

    #[test]
    fn display_of_a_failure_of_the_samples_states_the_reason() {
        assert_eq!(
            Error::DecodeTimeOverflow { track_id: 1 }.to_string(),
            "decode time of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::PresentationTimeOverflow { track_id: 1 }.to_string(),
            "presentation time of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::MissingDecodeTime { track_id: 1 }.to_string(),
            "fragment of track 1 states no decode time, and where the track stands is not known"
        );
        assert_eq!(
            Error::DataOffsetOverflow { track_id: 1 }.to_string(),
            "data offset of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::UnknownTrackId { track_id: 3 }.to_string(),
            "movie declares no track 3"
        );
        assert_eq!(
            Error::UnknownSampleDescriptionIndex {
                track_id: 2,
                sample_description_index: 7,
            }
            .to_string(),
            "track 2 has no stsd entry 7"
        );
        assert_eq!(
            Error::MissingMovieExtends.to_string(),
            "the movie carries no mvex"
        );
        assert_eq!(
            Error::SampleTableNotEmpty { track_id: 4 }.to_string(),
            "sample table of track 4 lays samples out in a movie continued in fragments"
        );
        assert_eq!(
            Error::UnknownDataReferenceIndex {
                track_id: 2,
                data_reference_index: 3,
            }
            .to_string(),
            "track 2 has no dref entry 3"
        );
        assert_eq!(
            Error::ExternalDataReference {
                track_id: 2,
                data_reference_index: 3,
            }
            .to_string(),
            "dref entry 3 of track 2 names an external file"
        );
        assert_eq!(
            Error::SampleCountMismatch { track_id: 2 }.to_string(),
            "sample tables of track 2 count different numbers of samples"
        );
        assert_eq!(
            Error::FirstChunkOutOfRange {
                track_id: 2,
                first_chunk: 5,
            }
            .to_string(),
            "run of chunks of track 2 starts at chunk 5, out of the range open to it"
        );
        assert_eq!(
            Error::SyncSampleOutOfRange {
                track_id: 2,
                sample_number: 5,
            }
            .to_string(),
            "sync sample 5 of track 2 is listed out of order or past its samples"
        );
        assert_eq!(
            Error::SampleSizeLimitExceeded {
                track_id: 1,
                declared_bytes: 32,
                limit_bytes: 16,
            }
            .to_string(),
            "track 1 declares a sample of 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            Error::SampleCountLimitExceeded {
                declared_samples: 32,
                limit_samples: 16,
            }
            .to_string(),
            "boxes declare 32 samples, past the 16-sample limit"
        );
        assert_eq!(
            Error::HeldExtentLimitExceeded {
                needed_extents: 17,
                limit_extents: 16,
            }
            .to_string(),
            "reader would hold 17 extents, past the 16-extent limit"
        );
        assert_eq!(
            Error::HeldBytesLimitExceeded {
                needed_bytes: 32,
                limit_bytes: 16,
            }
            .to_string(),
            "reader would hold 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            Error::UnfinishedSample {
                track_id: 2,
                needed_bytes: 1_024,
                available_bytes: 512,
            }
            .to_string(),
            "sample of track 2 takes 1024 bytes, and 512 arrived"
        );
        assert_eq!(
            Error::AlreadyFinished.to_string(),
            "samples were declared over and take nothing more"
        );
        assert_eq!(
            Error::NoFragmentOpen.to_string(),
            "no fragment is open to carry a sample or be closed"
        );
        assert_eq!(
            Error::FragmentStillOpen.to_string(),
            "fragment is still open"
        );
        assert_eq!(
            Error::SampleSizeOutOfRange {
                track_id: 1,
                declared_bytes: 1 << 40,
            }
            .to_string(),
            "track 1 states a sample of 1099511627776 bytes, past the 4294967295 a trun row or an stsz entry carries"
        );
        assert_eq!(
            Error::DataOffsetOutOfRange {
                track_id: 1,
                data_offset: 1 << 40,
            }
            .to_string(),
            "sample data of track 1 lies 1099511627776 bytes into the fragment, past the 2147483647 a trun carries"
        );
        assert_eq!(
            Error::CompositionTimeOffsetOutOfRange {
                track_id: 1,
                composition_time_offset: 1 << 40,
            }
            .to_string(),
            "track 1 states a composition time offset of 1099511627776, outside what 32 signed bits hold"
        );
        assert_eq!(
            Error::DecodeTimeMismatch {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024,
            }
            .to_string(),
            "track 1 states decode time 512 where the samples before it reach 1024"
        );
        assert_eq!(
            Error::BackwardDecodeTime {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024,
            }
            .to_string(),
            "track 1 goes back to decode time 512 after reaching 1024"
        );
        assert_eq!(
            Error::SampleDescriptionIndexMismatch {
                track_id: 1,
                stated_sample_description_index: 2,
                established_sample_description_index: 1,
            }
            .to_string(),
            "track 1 describes a sample by stsd entry 2 in a fragment or a chunk describing it by 1"
        );
        assert_eq!(
            Error::NoChunkOpen.to_string(),
            "no chunk is open to carry a sample"
        );
        assert_eq!(
            Error::TrackIdMismatch {
                stated_track_id: 2,
                established_track_id: 1,
            }
            .to_string(),
            "sample of track 2 handed over to a chunk of track 1"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"mvex"));

        assert_eq!(Error::from(box_error).to_string(), box_error.to_string());
    }
}
