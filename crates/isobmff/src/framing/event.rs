//! What a [`BoxReader`](super::BoxReader) reports

use core::fmt;

use isobmff_core::{BoxHeader, BoxType};

/// What a [`BoxReader`](super::BoxReader) found in its input
///
/// `offset` is the file offset of the first byte of the box, its header.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event<'a> {
    /// A box the reader does not gather: its payload is not requested, and
    /// reading goes on at the box after it
    Box {
        /// File offset of the box
        offset: u64,
        /// Header of the box
        header: BoxHeader,
    },
    /// A box the reader gathered whole, its payload lent from the reader's
    /// buffer
    ///
    /// The header is not carried: the box ends where its payload does, at
    /// `payload_offset` plus the length of `payload`.
    Gathered {
        /// File offset of the box
        offset: u64,
        /// Type of the box
        box_type: BoxType,
        /// File offset of the payload, past the header
        payload_offset: u64,
        /// Payload of the box
        payload: &'a [u8],
    },
    /// A box that cannot be read; reading goes on where the error says
    Error(Error),
}

/// A box a [`BoxReader`](super::BoxReader) cannot read
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Error {
    /// The bytes at `offset` are no box header, such as one whose `size` is
    /// smaller than the header; no box after it can be found, so reading ends
    MalformedHeader {
        /// File offset of the header
        offset: u64,
        /// Why the header does not read
        source: isobmff_core::Error,
    },
    /// The payload of a box the reader gathers is longer than
    /// [`FramerConfig::payload_limit`](super::FramerConfig::payload_limit);
    /// the box is skipped, and reading goes on at the box after it
    PayloadOverLimit {
        /// File offset of the box
        offset: u64,
        /// Header of the box
        header: BoxHeader,
        /// The limit the payload is longer than
        limit: u64,
    },
    /// The input ends inside the header at `offset`
    TruncatedHeader {
        /// File offset of the header
        offset: u64,
    },
    /// The input ends inside the box the reader gathers at `offset`, before
    /// the end its header declares
    TruncatedBox {
        /// File offset of the box
        offset: u64,
        /// Header of the box
        header: BoxHeader,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedHeader { offset, .. } => {
                write!(formatter, "the box header at offset {offset} does not read")
            }
            Self::PayloadOverLimit {
                offset,
                header,
                limit,
            } => match header.payload_len() {
                Some(payload_len) => write!(
                    formatter,
                    "the payload of the box `{}` at offset {offset} is {payload_len} bytes, over the limit of {limit}",
                    header.box_type()
                ),
                None => write!(
                    formatter,
                    "the payload of the box `{}` at offset {offset} runs past the limit of {limit} bytes",
                    header.box_type()
                ),
            },
            Self::TruncatedHeader { offset } => {
                write!(
                    formatter,
                    "the input ends inside the box header at offset {offset}"
                )
            }
            Self::TruncatedBox { offset, header } => write!(
                formatter,
                "the input ends inside the box `{}` at offset {offset}",
                header.box_type()
            ),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::MalformedHeader { source, .. } => Some(source),
            Self::PayloadOverLimit { .. }
            | Self::TruncatedHeader { .. }
            | Self::TruncatedBox { .. } => None,
        }
    }
}
