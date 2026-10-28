//! [`DemuxError`] and [`MuxError`], the reasons the bytes of a file do not read as its boxes, or boxes do not write as a file

use core::error;
use core::fmt;

use isobmff_core::{BoxType, Error};

use crate::byte_range::ByteRange;
use crate::framing::BoxEventKind;

/// Reason the bytes of a file do not read as its boxes
///
/// A reader that fails stays failed: every call after it returns the same
/// failure. [`UnwantedInput`](Self::UnwantedInput),
/// [`UndecidedBox`](Self::UndecidedBox), [`NoOpenBox`](Self::NoOpenBox) and a
/// [`PayloadLimitExceeded`](Self::PayloadLimitExceeded) refused by a gather
/// are the exceptions, which refuse one call and leave the reader as it was.
///
/// A box whose own bytes are malformed fails with the [`Error`] it reads with,
/// carried whole as [`Box`](Self::Box).
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DemuxError {
    /// Input starts at a file offset the reader does not want next
    #[non_exhaustive]
    UnwantedInput {
        /// File offset the input starts at
        offset: u64,
        /// Input the reader wants next, or `None` when it wants no more
        wanted: Option<ByteRange>,
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
            Self::UnwantedInput {
                offset,
                wanted: Some(wanted),
            } => write!(
                formatter,
                "input at file offset {offset} where file offset {} is wanted",
                wanted.offset()
            ),
            Self::UnwantedInput {
                offset,
                wanted: None,
            } => write!(
                formatter,
                "input at file offset {offset} where no more is wanted"
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
