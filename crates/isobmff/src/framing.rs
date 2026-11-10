//! The boxes laid end to end in a file, read from bytes that arrive in pieces
//!
//! [`BoxReader`] cuts each box at file level off the bytes of a file: it
//! gathers the payload of a box whose type [`FramerConfig`] names, and skips
//! the payload of any other without requesting it. [`media_data_header`]
//! writes the header of an `mdat` whose length is known only once its
//! payload is written.

use alloc::vec::Vec;

use isobmff_core::{BoxHeader, BoxSize, BoxType, ExtendedSize};

mod event;
mod reader;

pub use event::{Error, Event};
pub use reader::BoxReader;

/// Settings of a [`BoxReader`]
///
/// ```
/// use isobmff::framing::FramerConfig;
/// use isobmff_core::BoxType;
///
/// // Gather the `meta` at file level too, besides the structure-data
/// let mut config = FramerConfig::new(16 * 1024 * 1024);
/// config.gathered_box_types.push(BoxType::compact(*b"meta"));
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FramerConfig {
    /// The types of the boxes whose payload the reader gathers and lends;
    /// the payload of a box of any other type is skipped
    pub gathered_box_types: Vec<BoxType>,
    /// The most bytes the payload of a box the reader gathers may hold; a
    /// longer one is reported as [`Error::PayloadOverLimit`] and skipped
    ///
    /// The `size` of a box is read from the input, so nothing but this limit
    /// bounds what the reader holds. The header needs no limit: it is at most
    /// 32 bytes.
    pub payload_limit: u64,
}

impl FramerConfig {
    /// The box types the reader gathers by default: those at file level that
    /// hold structure-data, the declarations that describe the media data,
    /// ISO/IEC 14496-12 A.3
    ///
    /// `ftyp` (§5.2), `styp` (§8.14.2), `moov` (§8.2.1), `moof` (§8.8.4),
    /// `sidx` (§8.14.3) and `mfra` (§8.8.9). The media data (`mdat` and
    /// `imda`, §8.1.1, §8.1.4), free space (`free` and `skip`, §8.1.2) and
    /// any other box are skipped.
    pub const STRUCTURE_DATA: [BoxType; 6] = [
        BoxType::compact(*b"ftyp"),
        BoxType::compact(*b"styp"),
        BoxType::compact(*b"moov"),
        BoxType::compact(*b"moof"),
        BoxType::compact(*b"sidx"),
        BoxType::compact(*b"mfra"),
    ];

    /// Creates the settings that gather [`FramerConfig::STRUCTURE_DATA`], each
    /// with a payload of at most `payload_limit` bytes
    #[must_use]
    pub fn new(payload_limit: u64) -> Self {
        Self {
            gathered_box_types: Self::STRUCTURE_DATA.to_vec(),
            payload_limit,
        }
    }
}

/// Returns the header of an `mdat` of `payload_length` bytes, in the 16 bytes of a `largesize`, or [`None`] where the box would be longer than 64 bits count
///
/// The `media data` box, ISO/IEC 14496-12 §8.1.1, under the header of §4.2.2
/// with `size` 1. The header is 16 bytes whatever the length, so one written
/// before the payload, while its length is unknown, is written over in place
/// once it is known.
///
/// ```
/// use isobmff::framing::media_data_header;
///
/// assert_eq!(
///     media_data_header(4),
///     Some(*b"\0\0\0\x01mdat\0\0\0\0\0\0\0\x14")
/// );
/// assert_eq!(media_data_header(u64::MAX), None);
/// ```
#[must_use]
pub fn media_data_header(payload_length: u64) -> Option<[u8; 16]> {
    let size = ExtendedSize::new(payload_length.checked_add(16)?)?;
    let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size))?;
    let mut bytes = [0; 16];
    header.encode(&mut bytes).ok()?;
    Some(bytes)
}
