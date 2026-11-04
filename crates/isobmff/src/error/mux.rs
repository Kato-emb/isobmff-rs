//! [`MuxError`], the reasons boxes do not write as the bytes of a file

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Error, FourCc};

use super::TimelineError;
use crate::framing::BoxEventKind;

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
    /// Box of a type the file holds one of, offered again
    #[non_exhaustive]
    DuplicateBox {
        /// Type of the box
        box_type: BoxType,
    },
    /// Box offered after a box it goes before in the file
    #[non_exhaustive]
    BoxOutOfOrder {
        /// Type of the box offered
        box_type: BoxType,
    },
    /// Samples, or the end of the file, offered before the movie that declares their tracks
    #[non_exhaustive]
    MissingMovie,
    /// Indexed segment of a number of subsegments other than its `sidx` was reserved for: a subsegment past the number, refused before anything of it is written, or fewer than the number as the segment finishes, which leaves the reserved space a `free` box, §8.14.3
    #[non_exhaustive]
    SegmentIndexCountMismatch {
        /// Number of subsegments the `sidx` was reserved for
        expected_subsegments: u32,
        /// Number of subsegments written, or the one past the number
        written_subsegments: u32,
    },
    /// Recoverable file that holds more samples than the muxer keeps the locations of, which it does not lay out again as a file that is not fragmented; the file written is a fragmented movie file whole
    #[non_exhaustive]
    NotFinalised {
        /// Number of samples written
        held_locations: u64,
        /// Number of samples the muxer keeps the locations of
        limit_locations: u64,
    },
    /// Bytes offered for space reserved with a `free` box that neither fill it nor leave room for a `free` box after them: longer than it, short of it by fewer than the 8 bytes of a `free` header, or offered for space of no known length, §8.1.2
    #[non_exhaustive]
    ReservedSpaceMismatch {
        /// Length of the space reserved, where it is known
        reserved_bytes: Option<u64>,
        /// Length of the bytes offered for it
        offered_bytes: u64,
    },
    /// Brands of a fragmented file or a segment that name none of `iso5` to `isod`, under which a reader takes the `moof` as the base data offset of a track fragment, §8.8.7.1
    #[non_exhaustive]
    UnsupportedBrand {
        /// The major brand
        brand: FourCc,
    },
    /// Table of the sample table of a track that declares samples and that the muxer does not write from the samples it takes: a sample group, sub-sample, sample auxiliary information or shadow sync table
    #[non_exhaustive]
    SampleTableNotEmpty {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the table
        box_type: BoxType,
    },
    /// Sample of a track the movie has none of
    #[non_exhaustive]
    UnknownTrackId {
        /// `track_ID` the sample gives
        track_id: u32,
    },
    /// Sample described by a sample entry the `stsd` of its track has none of, or one too short to hold a `data_reference_index`
    #[non_exhaustive]
    UnknownSampleDescriptionIndex {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, the sample gives
        sample_description_index: u32,
    },
    /// Sample entry that names a data reference entry the `dref` of its track has none of
    #[non_exhaustive]
    UnknownDataReferenceIndex {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, the sample entry gives
        data_reference_index: u16,
    },
    /// Sample whose sample entry names a data reference entry for another file, §8.7.2
    #[non_exhaustive]
    ExternalDataReference {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, of the entry of the `dref` that names the file
        data_reference_index: u16,
    },
    /// Sample whose sample entry names a data reference entry for an `IdentifiedMediaDataBox`, which the writer does not write
    #[non_exhaustive]
    UnsupportedDataReference {
        /// `track_ID` of the track
        track_id: u32,
        /// Index, from 1, of the entry of the `dref` that names the box
        data_reference_index: u16,
    },
    /// Samples of one track written together that more than one sample entry describes, which a track fragment does not state
    #[non_exhaustive]
    MixedSampleDescriptions {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// Sample whose decoding time is not where the sample before it of its track ends, or, in a movie file, the first of a track without an edit list, not 0, §8.6.1.2
    #[non_exhaustive]
    DecodingTimeMismatch {
        /// `track_ID` of the track
        track_id: u32,
        /// Decoding time the durations of the samples before it come to
        expected_decoding_time: u64,
        /// Decoding time the sample gives
        decoding_time: u64,
    },
    /// Sample whose media data is longer than a sample size field holds
    #[non_exhaustive]
    SampleSizeOutOfRange {
        /// `track_ID` of the track
        track_id: u32,
        /// Length of the media data of the sample
        sample_bytes: u64,
    },
    /// Sample whose media data lies further than a data offset or a chunk offset reaches
    #[non_exhaustive]
    DataOffsetOutOfRange {
        /// `track_ID` of the track
        track_id: u32,
    },
    /// Edit of a track in a movie file whose `media_time` is before the first sample of the track, where the sample table counts decoding times from, §8.6.1.2, §8.6.6
    #[non_exhaustive]
    MediaTimeOutOfRange {
        /// `track_ID` of the track
        track_id: u32,
        /// `media_time` of the edit
        media_time: i64,
        /// Decoding time of the first sample of the track
        decoding_time: u64,
    },
    /// Edit list of the reference track of an indexed segment that does not map its composition timeline, which a `sidx` states its times after, §8.6.6, §8.14.3
    #[non_exhaustive]
    Timeline {
        /// Why it does not
        error: TimelineError,
    },
    /// Subsegment that the `sidx` of its segment cannot state, §8.14.3
    #[non_exhaustive]
    SegmentIndexOutOfRange {
        /// Stream ID of the reference stream of the `sidx`
        reference_id: u32,
    },
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
            Self::DuplicateBox { box_type } => write!(formatter, "{box_type} box offered again"),
            Self::BoxOutOfOrder { box_type } => write!(
                formatter,
                "{box_type} box offered after a box it goes before"
            ),
            Self::MissingMovie => formatter.write_str("samples offered before the movie"),
            Self::SegmentIndexCountMismatch {
                expected_subsegments,
                written_subsegments,
            } => write!(
                formatter,
                "subsegment {written_subsegments} of a sidx reserved for {expected_subsegments}"
            ),
            Self::NotFinalised {
                held_locations,
                limit_locations,
            } => write!(
                formatter,
                "{held_locations} samples written, past the {limit_locations} whose locations the muxer keeps: the file stays fragmented"
            ),
            Self::ReservedSpaceMismatch {
                reserved_bytes: Some(reserved_bytes),
                offered_bytes,
            } => write!(
                formatter,
                "{offered_bytes} bytes offered for {reserved_bytes} bytes reserved, which they neither fill nor leave room for a free box in"
            ),
            Self::ReservedSpaceMismatch {
                reserved_bytes: None,
                offered_bytes,
            } => write!(
                formatter,
                "{offered_bytes} bytes offered for reserved space of no known length"
            ),
            Self::UnsupportedBrand { brand } => write!(
                formatter,
                "brands of major brand {brand} name none of iso5 to isod, which a track fragment based on its moof takes"
            ),
            Self::SampleTableNotEmpty { track_id, box_type } => write!(
                formatter,
                "{box_type} of track {track_id} declares samples, which the muxer does not carry"
            ),
            Self::UnknownTrackId { track_id } => {
                write!(
                    formatter,
                    "a sample of track {track_id}, which the movie has none of"
                )
            }
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
            Self::MixedSampleDescriptions { track_id } => write!(
                formatter,
                "samples of track {track_id} written together are described by more than one sample entry"
            ),
            Self::DecodingTimeMismatch {
                track_id,
                expected_decoding_time,
                decoding_time,
            } => write!(
                formatter,
                "a sample of track {track_id} at decoding time {decoding_time}, where the samples before it end at {expected_decoding_time}"
            ),
            Self::SampleSizeOutOfRange {
                track_id,
                sample_bytes,
            } => write!(
                formatter,
                "a sample of track {track_id} of {sample_bytes} bytes, past what a sample size holds"
            ),
            Self::DataOffsetOutOfRange { track_id } => write!(
                formatter,
                "media data of a sample of track {track_id} lies past what a data offset reaches"
            ),
            Self::MediaTimeOutOfRange {
                track_id,
                media_time,
                decoding_time,
            } => write!(
                formatter,
                "edit of track {track_id} from media time {media_time}, before its first sample at decoding time {decoding_time}"
            ),
            Self::Timeline { error } => fmt::Display::fmt(&error, formatter),
            Self::SegmentIndexOutOfRange { reference_id } => write!(
                formatter,
                "subsegment that the segment index of stream {reference_id} cannot state"
            ),
            Self::Box { error } => fmt::Display::fmt(&error, formatter),
        }
    }
}

impl error::Error for MuxError {}
