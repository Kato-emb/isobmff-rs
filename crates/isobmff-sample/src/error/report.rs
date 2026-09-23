//! [`Error`] as a caller reads it: what its representation carries, and how it prints

use core::error;
use core::fmt;

use crate::error::Error;
use crate::error::representation::Representation;

impl Error {
    /// Returns the failure of one box carried through, when it holds one
    ///
    /// The values that failure carries, and the boxes it was reached through,
    /// are read off the [`isobmff_core::Error`] itself.
    #[must_use]
    pub const fn box_error(self) -> Option<isobmff_core::Error> {
        self.representation.fields().box_error
    }

    /// Returns the track the failure is about, for the kinds that name one
    #[must_use]
    pub const fn track_id(self) -> Option<u32> {
        self.representation.fields().track_id
    }

    /// Returns the `stsd` entry the failure names, for the kinds that name one
    #[must_use]
    pub const fn sample_description_index(self) -> Option<u32> {
        self.representation.fields().sample_description_index
    }

    /// Returns the `stsd` entry a fragment or a chunk describes its track by, for the kinds that compare one
    #[must_use]
    pub const fn established_sample_description_index(self) -> Option<u32> {
        self.representation
            .fields()
            .established_sample_description_index
    }

    /// Returns the `dref` entry the failure names, for the kinds that name one
    #[must_use]
    pub const fn data_reference_index(self) -> Option<u16> {
        self.representation.fields().data_reference_index
    }

    /// Returns the chunk a run of chunks starts at, counted from one, for the kinds that name one
    #[must_use]
    pub const fn first_chunk(self) -> Option<u32> {
        self.representation.fields().first_chunk
    }

    /// Returns the sample number the failure names, counted from one, for the kinds that name one
    #[must_use]
    pub const fn sample_number(self) -> Option<u32> {
        self.representation.fields().sample_number
    }

    /// Returns the bytes the failure required, for the kinds that count bytes
    #[must_use]
    pub const fn needed_bytes(self) -> Option<u64> {
        self.representation.fields().needed_bytes
    }

    /// Returns the bytes the failure had to hand, for the kinds that count bytes
    #[must_use]
    pub const fn available_bytes(self) -> Option<u64> {
        self.representation.fields().available_bytes
    }

    /// Returns the decode time a sample or a fragment states, for the kinds that compare one
    #[must_use]
    pub const fn stated_decode_time(self) -> Option<u64> {
        self.representation.fields().stated_decode_time
    }

    /// Returns the decode time the samples before it reach, for the kinds that compare one
    #[must_use]
    pub const fn reached_decode_time(self) -> Option<u64> {
        self.representation.fields().reached_decode_time
    }

    /// Returns how far into its fragment a sample lies, in bytes, for the kinds that place one
    #[must_use]
    pub const fn data_offset(self) -> Option<u64> {
        self.representation.fields().data_offset
    }

    /// Returns the composition time offset a sample states, for the kinds that name one
    #[must_use]
    pub const fn composition_time_offset(self) -> Option<i64> {
        self.representation.fields().composition_time_offset
    }

    /// Returns the flags a sample states, for the kinds that name them
    #[must_use]
    pub const fn sample_flags(self) -> Option<u32> {
        self.representation.fields().sample_flags
    }

    /// Returns the track a chunk holds, for the kinds that set one against the track a sample belongs to
    #[must_use]
    pub const fn established_track_id(self) -> Option<u32> {
        self.representation.fields().established_track_id
    }
}

impl fmt::Display for Error {
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
            Representation::SyncSampleOutOfRange {
                track_id,
                sample_number,
            } => write!(
                formatter,
                "sync sample {sample_number} of track {track_id} is listed out of order or past its samples"
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
                "track {track_id} states a sample of {declared} bytes, past the {} a trun row or an stsz entry carries",
                u32::MAX
            ),
            Representation::DataOffsetOutOfRange { track_id, offset } => write!(
                formatter,
                "sample data of track {track_id} lies {offset} bytes into the fragment, past the {} a trun carries",
                i32::MAX
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
                "track {track_id} describes a sample by stsd entry {stated} in a fragment or a chunk describing it by {established}"
            ),
            Representation::NoChunkOpen => {
                formatter.write_str("no chunk is open to carry a sample")
            }
            Representation::TrackIdMismatch {
                stated,
                established,
            } => write!(
                formatter,
                "sample of track {stated} handed over to a chunk of track {established}"
            ),
            Representation::UnsupportedCompositionTimeOffset { track_id, offset } => write!(
                formatter,
                "track {track_id} states a composition time offset of {offset}, which no sample table written here carries"
            ),
            Representation::UnsupportedSampleFlags {
                track_id,
                sample_flags,
            } => write!(
                formatter,
                "track {track_id} states sample flags {sample_flags:#010x}, which no sample table written here carries"
            ),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let values = self.representation.fields();
        let mut fields = formatter.debug_struct("Error");
        fields.field("kind", &self.kind());
        fields.field("category", &self.category());

        if let Some(box_error) = values.box_error {
            fields.field("box_error", &box_error);
        }
        if let Some(track_id) = values.track_id {
            fields.field("track_id", &track_id);
        }
        if let Some(established_track_id) = values.established_track_id {
            fields.field("established_track_id", &established_track_id);
        }
        if let Some(sample_description_index) = values.sample_description_index {
            fields.field("sample_description_index", &sample_description_index);
        }
        if let Some(established) = values.established_sample_description_index {
            fields.field("established_sample_description_index", &established);
        }
        if let Some(data_reference_index) = values.data_reference_index {
            fields.field("data_reference_index", &data_reference_index);
        }
        if let Some(first_chunk) = values.first_chunk {
            fields.field("first_chunk", &first_chunk);
        }
        if let Some(sample_number) = values.sample_number {
            fields.field("sample_number", &sample_number);
        }
        if let Some(needed) = values.needed_bytes {
            fields.field("needed_bytes", &needed);
        }
        if let Some(available) = values.available_bytes {
            fields.field("available_bytes", &available);
        }
        if let Some(stated) = values.stated_decode_time {
            fields.field("stated_decode_time", &stated);
        }
        if let Some(reached) = values.reached_decode_time {
            fields.field("reached_decode_time", &reached);
        }
        if let Some(data_offset) = values.data_offset {
            fields.field("data_offset", &data_offset);
        }
        if let Some(composition_time_offset) = values.composition_time_offset {
            fields.field("composition_time_offset", &composition_time_offset);
        }
        if let Some(sample_flags) = values.sample_flags {
            fields.field("sample_flags", &sample_flags);
        }

        fields.finish()
    }
}

impl error::Error for Error {
    /// Returns the failure of one box carried through, when it holds one
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        if let Representation::Box(box_error) = &self.representation {
            Some(box_error)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString as _;

    use isobmff_core::BoxType;

    use crate::error::Error;

    #[test]
    fn a_failure_carries_only_the_values_its_kind_names() {
        let error = Error::unknown_sample_description_index(2, 7);

        assert_eq!(error.track_id(), Some(2));
        assert_eq!(error.sample_description_index(), Some(7));
        assert_eq!(error.box_error(), None);
        assert_eq!(error.needed_bytes(), None);
        assert_eq!(error.established_sample_description_index(), None);

        let unfinished = Error::unfinished_sample(2, 1_024, 512);

        assert_eq!(unfinished.needed_bytes(), Some(1_024));
        assert_eq!(unfinished.available_bytes(), Some(512));
        assert_eq!(unfinished.sample_description_index(), None);
        assert_eq!(Error::missing_movie_extends().track_id(), None);
        assert_eq!(Error::unknown_track_id(3).sample_description_index(), None);

        let external = Error::external_data_reference(1, 2);

        assert_eq!(external.track_id(), Some(1));
        assert_eq!(external.data_reference_index(), Some(2));
        assert_eq!(external.first_chunk(), None);
        assert_eq!(Error::first_chunk_out_of_range(1, 3).first_chunk(), Some(3));
        assert_eq!(
            Error::sync_sample_out_of_range(1, 3).sample_number(),
            Some(3)
        );
        assert_eq!(Error::sample_count_mismatch(1).data_reference_index(), None);

        let too_long = Error::sample_size_out_of_range(1, 1 << 40);

        assert_eq!(too_long.track_id(), Some(1));
        assert_eq!(too_long.needed_bytes(), Some(1 << 40));
        assert_eq!(too_long.available_bytes(), None);

        let too_far = Error::data_offset_out_of_range(1, 1 << 40);

        assert_eq!(too_far.needed_bytes(), None);
        assert_eq!(too_far.data_offset(), Some(1 << 40));
        assert_eq!(too_far.composition_time_offset(), None);
        assert_eq!(
            Error::composition_time_offset_out_of_range(1, -(1 << 40)).composition_time_offset(),
            Some(-(1 << 40))
        );

        let mismatch = Error::sample_description_index_mismatch(1, 2, 1);

        assert_eq!(mismatch.sample_description_index(), Some(2));
        assert_eq!(mismatch.established_sample_description_index(), Some(1));
        assert_eq!(mismatch.stated_decode_time(), None);

        let gap = Error::decode_time_mismatch(1, 512, 1_024);

        assert_eq!(gap.stated_decode_time(), Some(512));
        assert_eq!(gap.reached_decode_time(), Some(1_024));
        assert_eq!(gap.data_offset(), None);
        assert_eq!(Error::no_fragment_open().track_id(), None);
    }

    #[test]
    fn a_failure_of_one_box_keeps_its_values_and_the_boxes_it_was_reached_through() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"trex"))
            .in_container(BoxType::compact(*b"mvex"));
        let carried = Error::from(box_error);

        assert_eq!(carried.box_error(), Some(box_error));
        assert_eq!(carried.track_id(), None);

        let mismatched = Error::track_id_mismatch(2, 1);

        assert_eq!(mismatched.track_id(), Some(2));
        assert_eq!(mismatched.established_track_id(), Some(1));
        assert_eq!(mismatched.sample_flags(), None);
        assert_eq!(
            Error::unsupported_sample_flags(1, 0x0200_0000).sample_flags(),
            Some(0x0200_0000)
        );
    }

    #[test]
    fn display_of_a_failure_of_the_samples_states_the_reason() {
        assert_eq!(
            Error::decode_time_overflow(1).to_string(),
            "decode time of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::data_offset_overflow(1).to_string(),
            "data offset of track 1 runs past what 64 bits carry"
        );
        assert_eq!(
            Error::unknown_track_id(3).to_string(),
            "movie declares no track 3"
        );
        assert_eq!(
            Error::unknown_sample_description_index(2, 7).to_string(),
            "track 2 has no stsd entry 7"
        );
        assert_eq!(
            Error::missing_movie_extends().to_string(),
            "the movie carries no mvex"
        );
        assert_eq!(
            Error::unknown_data_reference_index(2, 3).to_string(),
            "track 2 has no dref entry 3"
        );
        assert_eq!(
            Error::external_data_reference(2, 3).to_string(),
            "dref entry 3 of track 2 names an external file"
        );
        assert_eq!(
            Error::sample_count_mismatch(2).to_string(),
            "sample tables of track 2 count different numbers of samples"
        );
        assert_eq!(
            Error::first_chunk_out_of_range(2, 5).to_string(),
            "run of chunks of track 2 starts at chunk 5, out of the range open to it"
        );
        assert_eq!(
            Error::sync_sample_out_of_range(2, 5).to_string(),
            "sync sample 5 of track 2 is listed out of order or past its samples"
        );
        assert_eq!(
            Error::sample_size_limit_exceeded(1, 32, 16).to_string(),
            "track 1 declares a sample of 32 bytes, past the 16-byte limit"
        );
        assert_eq!(
            Error::unfinished_sample(2, 1_024, 512).to_string(),
            "sample of track 2 takes 1024 bytes, and 512 arrived"
        );
        assert_eq!(
            Error::already_finished().to_string(),
            "samples were declared over and take nothing more"
        );
        assert_eq!(
            Error::no_fragment_open().to_string(),
            "no fragment is open to carry a sample or be closed"
        );
        assert_eq!(
            Error::fragment_still_open().to_string(),
            "fragment is still open"
        );
        assert_eq!(
            Error::sample_size_out_of_range(1, 1 << 40).to_string(),
            "track 1 states a sample of 1099511627776 bytes, past the 4294967295 a trun row or an stsz entry carries"
        );
        assert_eq!(
            Error::data_offset_out_of_range(1, 1 << 40).to_string(),
            "sample data of track 1 lies 1099511627776 bytes into the fragment, past the 2147483647 a trun carries"
        );
        assert_eq!(
            Error::composition_time_offset_out_of_range(1, 1 << 40).to_string(),
            "track 1 states a composition time offset of 1099511627776, which neither version of a trun writes"
        );
        assert_eq!(
            Error::decode_time_mismatch(1, 512, 1_024).to_string(),
            "track 1 states decode time 512 where the samples before it reach 1024"
        );
        assert_eq!(
            Error::backward_decode_time(1, 512, 1_024).to_string(),
            "track 1 goes back to decode time 512 after reaching 1024"
        );
        assert_eq!(
            Error::sample_description_index_mismatch(1, 2, 1).to_string(),
            "track 1 describes a sample by stsd entry 2 in a fragment or a chunk describing it by 1"
        );
        assert_eq!(
            Error::no_chunk_open().to_string(),
            "no chunk is open to carry a sample"
        );
        assert_eq!(
            Error::track_id_mismatch(2, 1).to_string(),
            "sample of track 2 handed over to a chunk of track 1"
        );
        assert_eq!(
            Error::unsupported_composition_time_offset(1, -8).to_string(),
            "track 1 states a composition time offset of -8, which no sample table written here carries"
        );
        assert_eq!(
            Error::unsupported_sample_flags(1, 0x0200_0000).to_string(),
            "track 1 states sample flags 0x02000000, which no sample table written here carries"
        );
    }

    #[test]
    fn display_of_a_failure_of_one_box_reads_as_that_failure() {
        let box_error = isobmff_core::Error::missing_mandatory_box(BoxType::compact(*b"mvex"));

        assert_eq!(Error::from(box_error).to_string(), box_error.to_string());
    }

    #[test]
    fn debug_names_the_values_a_kind_carries_and_leaves_out_the_rest() {
        assert_eq!(
            format!("{:?}", Error::decode_time_overflow(1)),
            "Error { kind: DecodeTimeOverflow, category: Malformed, track_id: 1 }"
        );
        assert_eq!(
            format!("{:?}", Error::missing_movie_extends()),
            "Error { kind: MissingMovieExtends, category: Malformed }"
        );
        assert_eq!(
            format!("{:?}", Error::sample_size_limit_exceeded(1, 32, 16)),
            "Error { kind: SampleSizeLimitExceeded, category: Unsupported, track_id: 1, needed_bytes: 32, available_bytes: 16 }"
        );
        assert_eq!(
            format!("{:?}", Error::external_data_reference(1, 2)),
            "Error { kind: ExternalDataReference, category: Unsupported, track_id: 1, data_reference_index: 2 }"
        );
        assert_eq!(
            format!("{:?}", Error::first_chunk_out_of_range(1, 5)),
            "Error { kind: FirstChunkOutOfRange, category: Malformed, track_id: 1, first_chunk: 5 }"
        );
        assert_eq!(
            format!("{:?}", Error::sync_sample_out_of_range(1, 5)),
            "Error { kind: SyncSampleOutOfRange, category: Malformed, track_id: 1, sample_number: 5 }"
        );
        assert_eq!(
            format!("{:?}", Error::backward_decode_time(1, 512, 1_024)),
            "Error { kind: BackwardDecodeTime, category: Malformed, track_id: 1, stated_decode_time: 512, reached_decode_time: 1024 }"
        );
        assert_eq!(
            format!("{:?}", Error::data_offset_out_of_range(1, 1 << 40)),
            "Error { kind: DataOffsetOutOfRange, category: Unsupported, track_id: 1, data_offset: 1099511627776 }"
        );
        assert_eq!(
            format!(
                "{:?}",
                Error::composition_time_offset_out_of_range(1, -(1 << 40))
            ),
            "Error { kind: CompositionTimeOffsetOutOfRange, category: Unsupported, track_id: 1, composition_time_offset: -1099511627776 }"
        );
        assert_eq!(
            format!("{:?}", Error::sample_description_index_mismatch(1, 2, 1)),
            "Error { kind: SampleDescriptionIndexMismatch, category: Malformed, track_id: 1, sample_description_index: 2, established_sample_description_index: 1 }"
        );
        assert_eq!(
            format!("{:?}", Error::track_id_mismatch(2, 1)),
            "Error { kind: TrackIdMismatch, category: Malformed, track_id: 2, established_track_id: 1 }"
        );
        assert_eq!(
            format!("{:?}", Error::unsupported_sample_flags(1, 0x0200_0000)),
            "Error { kind: UnsupportedSampleFlags, category: Unsupported, track_id: 1, sample_flags: 33554432 }"
        );
    }
}
