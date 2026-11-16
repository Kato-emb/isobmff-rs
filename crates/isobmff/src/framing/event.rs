//! What a [`BoxReader`](super::BoxReader) reports

use isobmff_core::{BoxHeader, BoxType};

use super::error::Error;

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
