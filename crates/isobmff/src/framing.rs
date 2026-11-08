//! The boxes laid end to end in a file, read from bytes that arrive in pieces
//!
//! [`BoxReader`] cuts each box at file level off the bytes of a file: it
//! gathers the payload of a box whose type [`FramerConfig`] names, and skips
//! the payload of any other without requesting it.

use alloc::vec::Vec;

use isobmff_core::BoxType;

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
