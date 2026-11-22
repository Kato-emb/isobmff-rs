//! [`BoxReaderConfig`], the settings of a [`BoxReader`](crate::file_level::BoxReader)

use alloc::vec::Vec;

use isobmff_core::BoxType;

/// Settings of a [`BoxReader`](crate::file_level::BoxReader)
///
/// ```
/// use isobmff::file_level::BoxReaderConfig;
/// use isobmff_core::BoxType;
///
/// // Read the payload of the `meta` at file level too, besides the structure-data
/// let mut config = BoxReaderConfig::new(16 * 1024 * 1024);
/// config.payload_box_types.push(BoxType::compact(*b"meta"));
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoxReaderConfig {
    /// The types of the boxes whose payload the reader reads and lends;
    /// the payload of a box of any other type is skipped
    pub payload_box_types: Vec<BoxType>,
    /// The most bytes a payload the reader reads may hold; a
    /// longer one is reported as [`Error::PayloadOverLimit`](crate::file_level::Error::PayloadOverLimit) and skipped
    ///
    /// The `size` of a box is read from the input, so nothing but this limit
    /// bounds what the reader holds. The header needs no limit: it is at most
    /// 32 bytes.
    pub payload_limit: u64,
    /// The types of the boxes whose payload the reader reads only the front
    /// part of, the rest being skipped; the payload of a box whose type
    /// [`payload_box_types`](Self::payload_box_types) holds is read whole
    ///
    /// A front part longer than [`payload_limit`](Self::payload_limit) is
    /// reported as [`Error::PayloadOverLimit`](crate::file_level::Error::PayloadOverLimit), and its box skipped.
    pub front_part_box_types: Vec<FrontPart>,
}

impl BoxReaderConfig {
    /// The box types whose payload the reader reads by default: those at file level that
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

    /// Creates the settings that read the payload of
    /// [`BoxReaderConfig::STRUCTURE_DATA`], each of at most `payload_limit` bytes
    #[must_use]
    pub fn new(payload_limit: u64) -> Self {
        Self {
            payload_box_types: Self::STRUCTURE_DATA.to_vec(),
            payload_limit,
            front_part_box_types: Vec::new(),
        }
    }
}

/// The first bytes of the payload of a box, the `front part` of it, that a [`BoxReader`](crate::file_level::BoxReader) reads
///
/// The `FrontPartBox`, ISO/IEC 23001-14, which ISO/IEC 14496-12 E.15.1
/// sets to hold the `imda_identifier` of an
/// [`IdentifiedMediaDataBox`](crate::boxes::IdentifiedMediaDataBox).
///
/// ```
/// use isobmff::file_level::{BoxReaderConfig, FrontPart};
/// use isobmff_core::BoxType;
///
/// // Read the identifier of each `imda`, and skip its media data
/// let mut config = BoxReaderConfig::new(16 * 1024 * 1024);
/// config.front_part_box_types.push(FrontPart { box_type: BoxType::compact(*b"imda"), length: 4 });
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "a caller of a reader builds one, and a front part is the bytes of a type of box up to a length"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrontPart {
    /// Type of the box
    pub box_type: BoxType,
    /// Number of the bytes of its payload read, from the first
    pub length: u64,
}
