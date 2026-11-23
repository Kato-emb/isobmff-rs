//! What a [`BoxReader`](super::BoxReader) reports

use isobmff_core::{BoxHeader, BoxType};

use super::error::Error;

/// What a [`BoxReader`](super::BoxReader) found in its input
///
/// `offset` is the file offset of the first byte of the box, its header.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event<'a> {
    /// A box whose payload the reader does not read: the payload is not
    /// requested, and reading goes on at the box after it
    Box {
        /// File offset of the box
        offset: u64,
        /// Header of the box
        header: BoxHeader,
    },
    /// A box whose payload the reader read whole, lent from the reader's
    /// buffer
    ///
    /// The header is not carried: the box ends where its payload does, at
    /// `payload_offset` plus the length of `payload`.
    BoxPayload {
        /// File offset of the box
        offset: u64,
        /// Type of the box
        box_type: BoxType,
        /// File offset of the payload, past the header
        payload_offset: u64,
        /// Payload of the box
        payload: &'a [u8],
    },
    /// A box whose payload the reader read only the front part of, its first
    /// bytes lent from the reader's buffer; the rest of its payload is not
    /// requested, and reading goes on at the box after it
    ///
    /// `front_part` holds the [`FrontPart::length`](super::FrontPart::length)
    /// bytes that start the payload, or the whole payload where it is shorter.
    FrontPart {
        /// File offset of the box
        offset: u64,
        /// Header of the box
        header: BoxHeader,
        /// The first bytes of the payload
        front_part: &'a [u8],
    },
    /// A box that cannot be read; reading goes on where the error says
    Error(Error),
}
