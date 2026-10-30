//! [`DemuxError`] and [`MuxError`], the reasons the bytes of a file do not read as its boxes, or boxes do not write as a file

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Error};

use crate::byte_range::ByteRange;
use crate::framing::BoxEventKind;

/// Reason the bytes of a file do not read as its boxes, its structure-data as the samples it declares, or its media data as their bytes
///
/// A [`BoxReader`](crate::framing::BoxReader) that fails stays failed: every
/// call after it returns the same failure.
/// [`UnwantedInput`](Self::UnwantedInput),
/// [`UndecidedBox`](Self::UndecidedBox), [`NoOpenBox`](Self::NoOpenBox) and a
/// [`PayloadLimitExceeded`](Self::PayloadLimitExceeded) refused by a gather
/// are the exceptions, which refuse one call and leave the reader as it was.
/// A [`StructureDataReader`](crate::structure_data::StructureDataReader)
/// refuses the one box it is handed, and stays as it was. A
/// [`MediaDataReader`](crate::media_data::MediaDataReader) refuses the
/// locations it is handed, and stays as it was.
///
/// A box whose own bytes are malformed fails with the [`Error`] it reads with,
/// carried whole as [`Box`](Self::Box).
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DemuxError {
    /// Input starts past the file offset the reader wants next
    #[non_exhaustive]
    UnwantedInput {
        /// File offset the input starts at
        offset: u64,
        /// Input the reader wants next
        wanted: ByteRange,
    },
    /// Input arrives before the header just read is answered with a gather, a stream or a skip
    #[non_exhaustive]
    UndecidedBox {
        /// Type of the box the header opens
        box_type: BoxType,
    },
    /// Gather, stream or skip asked for while no header waits for an answer
    #[non_exhaustive]
    NoOpenBox,
    /// Payload of a box being gathered reaches past the limit it was gathered with
    #[non_exhaustive]
    PayloadLimitExceeded {
        /// Type of the box
        box_type: BoxType,
        /// Length of the payload as declared, or, for a payload that runs to the end of the file, one byte past the limit
        reached_bytes: u64,
        /// Length the payload was gathered with as a limit
        limit_bytes: u64,
    },
    /// File ends inside the header of a box
    #[non_exhaustive]
    UnfinishedHeader {
        /// Length the header reaches
        needed_bytes: u64,
        /// Length of the header that arrived
        available_bytes: u64,
    },
    /// File ends inside a box, before the total its header declares
    #[non_exhaustive]
    UnfinishedBox {
        /// Type of the box
        box_type: BoxType,
        /// Total the header declares
        needed_bytes: u64,
        /// Length of the box that arrived
        available_bytes: u64,
    },
    /// Box of which a file holds at most one arrives a second time
    #[non_exhaustive]
    DuplicateBox {
        /// Type of the box
        box_type: BoxType,
    },
    /// Structure-data declares more samples than the limit it is resolved with
    #[non_exhaustive]
    SampleCountLimitExceeded {
        /// Number of samples the `moov` or the `moof` declares
        declared_samples: u64,
        /// Number of samples one `moov` or `moof` is resolved up to
        limit_samples: u64,
    },
    /// Table of a sample table that does not agree with the others on the samples of the track
    #[non_exhaustive]
    SampleTableMismatch {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the box of the table
        box_type: BoxType,
    },
    /// Samples described by a sample entry their track has none of
    #[non_exhaustive]
    UnknownSampleDescriptionIndex {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, of the sample entry
        sample_description_index: u32,
    },
    /// Sample entry that names an entry of a data reference its track has none of
    #[non_exhaustive]
    UnknownDataReferenceIndex {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, of the entry of the `dref`
        data_reference_index: u16,
    },
    /// Media data in a file other than the one the structure-data is in
    #[non_exhaustive]
    ExternalDataReference {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, of the entry of the `dref` that names the file
        data_reference_index: u16,
    },
    /// Media data in a container the reader does not resolve offsets into: an `IdentifiedMediaDataBox`
    #[non_exhaustive]
    UnsupportedDataReference {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, of the entry of the `dref`
        data_reference_index: u16,
    },
    /// Movie fragment that arrives before the movie it extends
    #[non_exhaustive]
    MissingMovie,
    /// Track fragment of a track the movie has none of
    #[non_exhaustive]
    UnknownTrackId {
        /// `track_ID` the `tfhd` gives
        track_id: u32,
    },
    /// Track fragment of a track the `mvex` of the movie gives no `trex` for
    #[non_exhaustive]
    MissingTrackExtends {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// Track fragment without a `tfdt` whose decoding time is not known, as after a reader resumes
    #[non_exhaustive]
    MissingDecodingTime {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// Media data of a sample that lies outside the file offsets 64 bits count
    #[non_exhaustive]
    DataOffsetOutOfRange {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// Decoding time of a sample past what 64 bits carry
    #[non_exhaustive]
    DecodingTimeOverflow {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// Segment index that would take the segment indexes a reader keeps past the limit it keeps them up to
    #[non_exhaustive]
    SegmentIndexCountLimitExceeded {
        /// Number of segment indexes the reader would keep
        retained_segment_indexes: u64,
        /// Number of segment indexes the reader keeps up to
        limit_segment_indexes: u64,
    },
    /// Segment index whose subsegments would take those a reader keeps past the limit it keeps them up to
    #[non_exhaustive]
    SubsegmentCountLimitExceeded {
        /// Number of subsegments the reader would keep
        retained_subsegments: u64,
        /// Number of subsegments the reader keeps up to
        limit_subsegments: u64,
    },
    /// Segment index whose byte ranges or presentation times run past 64 bits
    #[non_exhaustive]
    SegmentIndexOutOfRange {
        /// Stream ID of the reference stream of the `sidx`
        reference_id: u32,
    },
    /// Sample locations that would take the locations a reader holds past the limit it holds them up to
    #[non_exhaustive]
    HeldLocationLimitExceeded {
        /// Number of locations the reader would hold
        held_locations: u64,
        /// Number of locations the reader holds up to
        limit_locations: u64,
    },
    /// Sample location whose media data is longer than the limit a reader holds one sample up to
    #[non_exhaustive]
    SampleBytesLimitExceeded {
        /// Length of the media data the location names
        declared_bytes: u64,
        /// Length of the media data of one sample the reader holds up to
        limit_bytes: u64,
    },
    /// Input that would have a reader allocate the media data of a sample past the limit of the bytes it holds, and of which it takes nothing
    #[non_exhaustive]
    HeldBytesLimitExceeded {
        /// Bytes the reader would hold allocated for samples not yet taken
        held_bytes: u64,
        /// Bytes the reader holds allocated for samples not yet taken up to
        limit_bytes: u64,
    },
    /// Input ends before the media data of a sample has arrived
    #[non_exhaustive]
    UnfinishedSample {
        /// `track_ID` of the track the sample is of
        track_id: u32,
        /// Length of the media data of the sample
        needed_bytes: u64,
        /// Length of the media data of the sample that arrived
        available_bytes: u64,
    },
    /// Box does not read off its bytes
    #[non_exhaustive]
    Box {
        /// Why it does not
        error: Error,
    },
}

impl fmt::Display for DemuxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnwantedInput { offset, wanted } => write!(
                formatter,
                "input at file offset {offset} where file offset {} is wanted",
                wanted.offset()
            ),
            Self::UndecidedBox { box_type } => write!(
                formatter,
                "input before the {box_type} header just read is answered"
            ),
            Self::NoOpenBox => formatter.write_str("an answer while no header waits for one"),
            Self::PayloadLimitExceeded {
                box_type,
                reached_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "{box_type} payload reaches {reached_bytes} bytes, past the limit of {limit_bytes}"
            ),
            Self::UnfinishedHeader {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "file ends {available_bytes} bytes into a box header of {needed_bytes}"
            ),
            Self::UnfinishedBox {
                box_type,
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "file ends {available_bytes} bytes into a {box_type} box of {needed_bytes}"
            ),
            Self::DuplicateBox { box_type } => write!(formatter, "a second {box_type} box"),
            Self::SampleCountLimitExceeded {
                declared_samples,
                limit_samples,
            } => write!(
                formatter,
                "{declared_samples} samples declared, past the limit of {limit_samples}"
            ),
            Self::SampleTableMismatch { track_id, box_type } => write!(
                formatter,
                "{box_type} of track {track_id} does not agree with the rest of its sample table"
            ),
            Self::UnknownSampleDescriptionIndex {
                track_id,
                sample_description_index,
            } => write!(
                formatter,
                "track {track_id} has no sample entry {sample_description_index}"
            ),
            Self::UnknownDataReferenceIndex {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "track {track_id} has no data reference entry {data_reference_index}"
            ),
            Self::ExternalDataReference {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "data reference entry {data_reference_index} of track {track_id} names another file"
            ),
            Self::UnsupportedDataReference {
                track_id,
                data_reference_index,
            } => write!(
                formatter,
                "data reference entry {data_reference_index} of track {track_id} names an identified media data box"
            ),
            Self::MissingMovie => formatter.write_str("a movie fragment before the movie"),
            Self::UnknownTrackId { track_id } => {
                write!(
                    formatter,
                    "a track fragment of track {track_id}, which the movie has none of"
                )
            }
            Self::MissingTrackExtends { track_id } => {
                write!(
                    formatter,
                    "a track fragment of track {track_id}, which has no trex"
                )
            }
            Self::MissingDecodingTime { track_id } => write!(
                formatter,
                "a track fragment of track {track_id} without a tfdt, whose decoding time is not known"
            ),
            Self::DataOffsetOutOfRange { track_id } => write!(
                formatter,
                "media data of a sample of track {track_id} lies outside 64-bit file offsets"
            ),
            Self::DecodingTimeOverflow { track_id } => write!(
                formatter,
                "decoding time of a sample of track {track_id} is past 64 bits"
            ),
            Self::SegmentIndexCountLimitExceeded {
                retained_segment_indexes,
                limit_segment_indexes,
            } => write!(
                formatter,
                "{retained_segment_indexes} segment indexes kept, past the limit of {limit_segment_indexes}"
            ),
            Self::SubsegmentCountLimitExceeded {
                retained_subsegments,
                limit_subsegments,
            } => write!(
                formatter,
                "{retained_subsegments} subsegments kept, past the limit of {limit_subsegments}"
            ),
            Self::SegmentIndexOutOfRange { reference_id } => write!(
                formatter,
                "segment index of stream {reference_id} runs past 64 bits"
            ),
            Self::HeldLocationLimitExceeded {
                held_locations,
                limit_locations,
            } => write!(
                formatter,
                "{held_locations} sample locations held, past the limit of {limit_locations}"
            ),
            Self::SampleBytesLimitExceeded {
                declared_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "sample of {declared_bytes} bytes, past the limit of {limit_bytes}"
            ),
            Self::HeldBytesLimitExceeded {
                held_bytes,
                limit_bytes,
            } => write!(
                formatter,
                "{held_bytes} bytes of samples held, past the limit of {limit_bytes}"
            ),
            Self::UnfinishedSample {
                track_id,
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "input ends {available_bytes} bytes into a sample of track {track_id} of {needed_bytes}"
            ),
            Self::Box { error } => fmt::Display::fmt(&error, formatter),
        }
    }
}

impl error::Error for DemuxError {}

/// Reason boxes do not write as the bytes of a file
///
/// A refused event leaves the writer as it was: nothing of it is output.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MuxError {
    /// Payload offered for a box is not the length its header declares
    #[non_exhaustive]
    PayloadLengthMismatch {
        /// Type of the box
        box_type: BoxType,
        /// Length of the payload the header declares
        declared_bytes: u64,
        /// Length of the payload offered
        offered_bytes: u64,
    },
    /// Event that does not fit the box being written: a payload or an end with no box open, or a header while one is
    #[non_exhaustive]
    UnexpectedEvent {
        /// Event the writer takes next: a header when no box is open, otherwise the end of the open box
        expected: BoxEventKind,
        /// Event offered
        found: BoxEventKind,
    },
    /// File finished while a box whose payload length is declared is open
    #[non_exhaustive]
    UnfinishedBox {
        /// Type of the box
        box_type: BoxType,
        /// Length of the payload the header declares
        declared_bytes: u64,
        /// Length of the payload written
        written_bytes: u64,
    },
    /// Box offered after one whose size runs to the end of the file
    #[non_exhaustive]
    PastEndOfFile {
        /// Type of the box offered
        box_type: BoxType,
    },
    /// Box offered after the file is finished
    #[non_exhaustive]
    AlreadyFinished,
    /// Box does not write into bytes
    #[non_exhaustive]
    Box {
        /// Why it does not
        error: Error,
    },
}

impl fmt::Display for MuxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::PayloadLengthMismatch {
                box_type,
                declared_bytes,
                offered_bytes,
            } => write!(
                formatter,
                "{box_type} header declares a payload of {declared_bytes} bytes, offered {offered_bytes}"
            ),
            Self::UnexpectedEvent { expected, found } => write!(
                formatter,
                "{found:?} offered where the writer takes {expected:?}"
            ),
            Self::UnfinishedBox {
                box_type,
                declared_bytes,
                written_bytes,
            } => write!(
                formatter,
                "file finished {written_bytes} bytes into a {box_type} payload of {declared_bytes}"
            ),
            Self::PastEndOfFile { box_type } => write!(
                formatter,
                "{box_type} box offered after a box that runs to the end of the file"
            ),
            Self::AlreadyFinished => formatter.write_str("box offered after the file is finished"),
            Self::Box { error } => fmt::Display::fmt(&error, formatter),
        }
    }
}

impl error::Error for MuxError {}
