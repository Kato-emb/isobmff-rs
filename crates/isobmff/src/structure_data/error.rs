//! Why a [`StructureDataReader`](super::StructureDataReader) refuses a box, a writer a sample, or [`subsegments`](super::subsegments) a segment index

use core::fmt;

use isobmff_core::BoxType;

/// Why a [`StructureDataReader`](super::StructureDataReader) refuses a box, a [`SampleTableWriter`](super::SampleTableWriter) or a [`MovieFragmentWriter`](super::MovieFragmentWriter) a sample, or [`subsegments`](super::subsegments) a segment index
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// The tracks declare more samples than
    /// [`StructureDataReader::new`](super::StructureDataReader::new) allows,
    /// and nothing is allocated for them; or a sample is one more than
    /// [`SampleTableWriter::new`](super::SampleTableWriter::new) or
    /// [`MovieFragmentWriter::new`](super::MovieFragmentWriter::new) allows,
    /// and it is not taken
    SampleCountOverLimit {
        /// Number of samples the tracks declare, or the writer would hold
        sample_count: u64,
        /// The most samples the reader resolves from one box, or the writer
        /// holds of a track or of a fragment
        limit: u64,
    },
    /// A table the sample table of a track needs is missing
    MissingTable {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box missing
        box_type: BoxType,
    },
    /// A table of a track counts another number of samples than the sample
    /// size table of the track does, which gives the number of samples in
    /// the track, ISO/IEC 14496-12 §8.7.3.2
    ///
    /// The specification recommends against such tables, and gives no value
    /// to a sample past the shorter of them, so no location can be made for
    /// it as the specification reads them.
    SampleCountMismatch {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box whose count differs
        box_type: BoxType,
        /// Number of samples the sample size table counts
        sample_count: u64,
        /// Number of samples the box counts
        table_sample_count: u64,
    },
    /// An entry of a table of a track holds a value the table does not allow,
    /// such as a `first_chunk` that does not increase, or a sync sample
    /// number that names no sample of the track or is not after the one
    /// before it, ISO/IEC 14496-12 §8.6.2.1
    InvalidEntry {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box
        box_type: BoxType,
        /// Index, from 0, of the entry
        entry: u64,
    },
    /// The samples of a track lie in another file than the one the box is in;
    /// or, for a writer, in an `IdentifiedMediaDataBox`
    MediaDataElsewhere {
        /// `track_ID` of the track
        track_id: u32,
        /// `data_reference_index`, from 1, of the sample entry of the samples
        data_reference_index: u16,
    },
    /// The samples of a track lie in the `IdentifiedMediaDataBox` the
    /// sequence number of their movie fragment names, and are not in a
    /// movie fragment, which ISO/IEC 14496-12 §8.7.2.1 requires them to be
    SequenceNumberOutsideMovieFragment {
        /// `track_ID` of the track
        track_id: u32,
        /// `data_reference_index`, from 1, of the sample entry of the samples
        data_reference_index: u16,
    },
    /// An `imda` has the `imda_identifier` of an `imda` read before, which
    /// ISO/IEC 14496-12 §8.1.4.3 forbids; the one read before is kept
    DuplicateImdaIdentifier {
        /// `imda_identifier` of the box
        imda_identifier: u32,
    },
    /// An `imda` is one more than
    /// [`StructureDataReader::new`](super::StructureDataReader::new) allows
    /// to remember, and it is not remembered
    IdentifiedMediaDataOverLimit {
        /// `imda_identifier` of the box
        imda_identifier: u32,
        /// The most `imda` the reader remembers
        limit: u64,
    },
    /// Samples count their offsets from an `imda` the file does not hold:
    /// none of the identifier was read when the input ended, ISO/IEC 14496-12
    /// §8.7.2.1
    IdentifiedMediaDataNotFound {
        /// The least `imda_identifier` the samples wait for
        imda_identifier: u32,
        /// Number of the samples that wait for it
        sample_count: u64,
    },
    /// The bytes of a sample of a track lie past the last offset a file can
    /// have, or its decoding time past the last a track can have
    Overflow {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// The `sample_composition_time_offset` of a sample does not fit the 32
    /// bits of a `ctts` entry, signed, ISO/IEC 14496-12 §8.6.1.3
    CompositionOffsetOutOfRange {
        /// `sample_composition_time_offset` of the sample
        sample_composition_time_offset: i64,
    },
    /// The `sample_description_index` of a sample names no entry of the
    /// `stsd` the sample table is written with
    SampleDescriptionIndexOutOfRange {
        /// `sample_description_index` of the sample, from 1
        sample_description_index: u32,
    },
    /// A movie fragment comes before any `moov`, which says what its tracks
    /// are, ISO/IEC 14496-12 §8.8.1
    MovieFragmentWithoutMovie,
    /// A track fragment, or a sample a
    /// [`MovieFragmentWriter`](super::MovieFragmentWriter) takes, is of a
    /// track the `moov` does not have
    UnknownTrack {
        /// `track_ID` the `tfhd` or the sample names
        track_id: u32,
    },
    /// A sample of a track fragment takes a value that neither the `tfhd` nor
    /// a `trex` of its track gives, ISO/IEC 14496-12 §8.8.3, §8.8.7
    MissingDefault {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// The `tfdt` of a track fragment gives a decoding time less than the
    /// `sample_duration`s of the samples before it in the track add up to,
    /// which ISO/IEC 14496-12 §8.8.12.1 forbids
    DecodingTimeMismatch {
        /// `track_ID` of the track
        track_id: u32,
        /// Decoding time the durations of the samples before add up to
        expected: u64,
        /// Decoding time the `tfdt` gives
        found: u64,
    },
    /// A sample is of another track than the samples a
    /// [`SampleTableWriter`](super::SampleTableWriter) has taken
    OtherTrack {
        /// `track_ID` of the samples taken
        track_id: u32,
        /// `track_ID` of the sample
        found: u32,
    },
    /// The decoding time of a sample is not the one the `sample_duration`s
    /// of the samples before it in the track add up to, from 0, which a
    /// time-to-sample table declares, ISO/IEC 14496-12 §8.6.1.2, or from the
    /// first sample of the track, which a track fragment run declares, §8.8.8
    DecodingTimeGap {
        /// `track_ID` of the track
        track_id: u32,
        /// Decoding time the table declares for the sample
        expected: u64,
        /// Decoding time of the sample
        found: u64,
    },
    /// A box does not write
    Encode(isobmff_core::Error),
    /// The items a `sidx` references reach past the last offset a file can
    /// have, or past the last presentation time 64 bits count, ISO/IEC
    /// 14496-12 §8.14.3
    SegmentIndexOverflow {
        /// `reference_ID` of the box
        reference_id: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleCountOverLimit {
                sample_count,
                limit,
            } => write!(
                formatter,
                "{sample_count} samples, over the limit of {limit}"
            ),
            Self::MissingTable { track_id, box_type } => write!(
                formatter,
                "the sample table of track {track_id} has no `{box_type}`"
            ),
            Self::SampleCountMismatch {
                track_id,
                box_type,
                sample_count,
                table_sample_count,
            } => write!(
                formatter,
                "the `{box_type}` of track {track_id} counts {table_sample_count} samples, and the sample size table {sample_count}"
            ),
            Self::InvalidEntry {
                track_id,
                box_type,
                entry,
            } => write!(
                formatter,
                "entry {entry} of the `{box_type}` of track {track_id} is not allowed"
            ),
            Self::MediaDataElsewhere {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "the samples of track {track_id} lie where data reference {data_reference_index} names, not in this file"
            ),
            Self::SequenceNumberOutsideMovieFragment {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "the samples of track {track_id} lie in the imda of the sequence number data reference {data_reference_index} names, outside any movie fragment"
            ),
            Self::DuplicateImdaIdentifier { imda_identifier } => write!(
                formatter,
                "imda_identifier {imda_identifier} is that of an imda read before"
            ),
            Self::IdentifiedMediaDataOverLimit {
                imda_identifier,
                limit,
            } => write!(
                formatter,
                "the imda of imda_identifier {imda_identifier} is over the limit of {limit}"
            ),
            Self::IdentifiedMediaDataNotFound {
                imda_identifier,
                sample_count,
            } => write!(
                formatter,
                "{sample_count} samples lie in the imda of imda_identifier {imda_identifier}, and the file has none"
            ),
            Self::Overflow { track_id } => write!(
                formatter,
                "a sample of track {track_id} lies past the last offset or decoding time there can be"
            ),
            Self::CompositionOffsetOutOfRange {
                sample_composition_time_offset,
            } => write!(
                formatter,
                "sample_composition_time_offset {sample_composition_time_offset} does not fit 32 bits, signed"
            ),
            Self::SampleDescriptionIndexOutOfRange {
                sample_description_index,
            } => write!(
                formatter,
                "sample_description_index {sample_description_index} names no entry of the `stsd`"
            ),
            Self::MovieFragmentWithoutMovie => {
                write!(formatter, "a movie fragment comes before any moov")
            }
            Self::UnknownTrack { track_id } => {
                write!(formatter, "the moov has no track {track_id}")
            }
            Self::MissingDefault { track_id } => write!(
                formatter,
                "a sample of track {track_id} takes a value neither its tfhd nor a trex gives"
            ),
            Self::DecodingTimeMismatch {
                track_id,
                expected,
                found,
            } => write!(
                formatter,
                "the tfdt of track {track_id} gives decoding time {found}, less than the {expected} the samples before it add up to"
            ),
            Self::OtherTrack { track_id, found } => write!(
                formatter,
                "a sample of track {found} follows those of track {track_id}"
            ),
            Self::DecodingTimeGap {
                track_id,
                expected,
                found,
            } => write!(
                formatter,
                "a sample of track {track_id} has decoding_time {found}, and the samples before it add up to {expected}"
            ),
            Self::Encode(_) => write!(formatter, "a box does not write"),
            Self::SegmentIndexOverflow { reference_id } => write!(
                formatter,
                "the items the segment index of stream {reference_id} references reach past the last offset or presentation time there can be"
            ),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Encode(source) => Some(source),
            Self::SampleCountOverLimit { .. }
            | Self::SegmentIndexOverflow { .. }
            | Self::MissingTable { .. }
            | Self::SampleCountMismatch { .. }
            | Self::InvalidEntry { .. }
            | Self::MediaDataElsewhere { .. }
            | Self::SequenceNumberOutsideMovieFragment { .. }
            | Self::DuplicateImdaIdentifier { .. }
            | Self::IdentifiedMediaDataOverLimit { .. }
            | Self::IdentifiedMediaDataNotFound { .. }
            | Self::Overflow { .. }
            | Self::CompositionOffsetOutOfRange { .. }
            | Self::SampleDescriptionIndexOutOfRange { .. }
            | Self::MovieFragmentWithoutMovie
            | Self::UnknownTrack { .. }
            | Self::MissingDefault { .. }
            | Self::DecodingTimeMismatch { .. }
            | Self::OtherTrack { .. }
            | Self::DecodingTimeGap { .. } => None,
        }
    }
}
