//! [`MuxError`], the reasons boxes do not write as the bytes of a file

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Error, FourCc};

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
    /// Brands of a fragmented file or a segment that name none of `iso5` to `isod`, under which a reader takes the `moof` as the base data offset of a track fragment, §8.8.7.1
    #[non_exhaustive]
    UnsupportedBrand {
        /// The major brand
        brand: FourCc,
    },
    /// Box offered where the file it is written into takes none: an `mvex` in the movie of a file that is not fragmented
    #[non_exhaustive]
    UnexpectedBox {
        /// Type of the box
        box_type: BoxType,
    },
    /// Table of the sample table of a track that declares samples, which the samples written declare in its place
    #[non_exhaustive]
    SampleTableNotEmpty {
        /// `track_ID` of the track
        track_id: u32,
        /// Type of the table
        box_type: BoxType,
    },
    /// Track of the movie of a fragmented file with no `trex`, which its fragments take defaults from
    #[non_exhaustive]
    MissingTrackExtends {
        /// `track_ID` of the track
        track_id: u32,
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
    /// Sample whose decoding time is not where the sample before it of its track ends, §8.6.1.2
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
            Self::UnsupportedBrand { brand } => write!(
                formatter,
                "brands of major brand {brand} name none of iso5 to isod, which a track fragment based on its moof takes"
            ),
            Self::UnexpectedBox { box_type } => {
                write!(formatter, "{box_type} box where the file takes none")
            }
            Self::SampleTableNotEmpty { track_id, box_type } => write!(
                formatter,
                "{box_type} of track {track_id} declares samples already"
            ),
            Self::MissingTrackExtends { track_id } => {
                write!(
                    formatter,
                    "track {track_id} of a fragmented movie has no trex"
                )
            }
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
            Self::Box { error } => fmt::Display::fmt(&error, formatter),
        }
    }
}

impl error::Error for MuxError {}
