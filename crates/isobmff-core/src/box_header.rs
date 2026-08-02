//! [`BoxHeader`] and [`DecodeError`], the box header of ISO/IEC 14496-12 §4.2

use core::error;
use core::fmt;

use crate::box_size::{BoxSize, CompactSize, ExtendedSize};
use crate::box_type::{BoxType, CompactType};
use crate::fourcc::FourCC;
use crate::uuid::Uuid;

/// Value of the `size` field that moves the total into the `largesize` field
const EXTENDED_SIZE_MARKER: u32 = 1;

/// Value of the `size` field that runs the box to the end of the file
const TO_END_OF_FILE_MARKER: u32 = 0;

/// Byte length of a header carrying the given optional fields
const fn header_length(has_large_size: bool, has_user_type: bool) -> u8 {
    match (has_large_size, has_user_type) {
        (false, false) => 8,
        (true, false) => 16,
        (false, true) => 24,
        (true, true) => 32,
    }
}

/// Fields that introduce a box and state how far it reaches
///
/// The wire order is `size`, `type`, then the fields the extended forms add:
/// `largesize` for [`BoxSize::Extended`] and `usertype` for
/// [`BoxType::Extended`]. A header is therefore 8, 16, 24, or 32 bytes long.
///
/// Every value declares a total that covers the header it prefixes;
/// [`encode`](Self::encode) is therefore infallible.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize, Uuid};
///
/// // A header states the total of the whole box, itself included
/// let header = BoxHeader::new(
///     BoxType::compact(*b"free"),
///     BoxSize::Compact(CompactSize::new(16).unwrap()),
/// )
/// .unwrap();
///
/// // A total that leaves no room for the header it prefixes is not a header
/// assert_eq!(
///     BoxHeader::new(
///         BoxType::Extended(Uuid::new([0xab; 16])),
///         BoxSize::Compact(CompactSize::new(16).unwrap()),
///     ),
///     None
/// );
///
/// // Decoding and encoding are inverse, byte for byte; the payload is not read
/// let mut buffer = [0; BoxHeader::MAX_ENCODED_LEN];
/// assert_eq!(header.encode(&mut buffer), b"\0\0\0\x10free");
/// assert_eq!(BoxHeader::decode(b"\0\0\0\x10free"), Ok((header, b"".as_slice())));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BoxHeader {
    box_type: BoxType,
    size: BoxSize,
}

impl BoxHeader {
    /// Buffer length [`encode`](Self::encode) writes into: the longest header
    pub const MAX_ENCODED_LEN: usize = header_length(true, true) as usize;

    /// Creates a header from a box type and a size
    ///
    /// Returns `None` when the declared total is smaller than the header those
    /// two forms occupy — 8 bytes, plus 8 for a `largesize` field and 16 for a
    /// `usertype` field. [`BoxSize::ToEndOfFile`] declares no total and is
    /// accepted with either box type.
    #[must_use]
    pub const fn new(box_type: BoxType, size: BoxSize) -> Option<Self> {
        let header_length = header_length(
            matches!(size, BoxSize::Extended(_)),
            matches!(box_type, BoxType::Extended(_)),
        ) as u64;

        match size.total_bytes() {
            Some(total) if total < header_length => None,
            Some(_) | None => Some(Self { box_type, size }),
        }
    }

    /// Returns the type of the box
    #[must_use]
    pub const fn box_type(self) -> BoxType {
        self.box_type
    }

    /// Returns the size of the box, in the form the wire carries it
    #[must_use]
    pub const fn size(self) -> BoxSize {
        self.size
    }

    /// Decodes the header that starts `input`
    ///
    /// Returns the header and the bytes after it, where the payload of the box
    /// begins. At most [`MAX_ENCODED_LEN`](Self::MAX_ENCODED_LEN) bytes are
    /// read, and the payload is not examined: a declared total that overruns
    /// `input` decodes as it stands.
    ///
    /// # Errors
    ///
    /// * [`TruncatedHeader`](DecodeError::TruncatedHeader): `input` ends inside
    ///   the header. A caller that reads in chunks can extend `input` to
    ///   `needed` bytes and decode again.
    /// * [`SizeBelowHeader`](DecodeError::SizeBelowHeader): the declared total
    ///   is smaller than the header it prefixes.
    pub fn decode(input: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let truncated_at = |needed: u8| DecodeError::TruncatedHeader {
            needed: usize::from(needed),
            available: input.len(),
        };

        let (size_field, after_size) = input
            .split_first_chunk::<4>()
            .ok_or_else(|| truncated_at(header_length(false, false)))?;
        let (type_field, after_type) = after_size
            .split_first_chunk::<4>()
            .ok_or_else(|| truncated_at(header_length(false, false)))?;

        let declared = u32::from_be_bytes(*size_field);
        let compact_type = CompactType::new(FourCC::new(*type_field));

        let (large_size, after_large_size) = if declared == EXTENDED_SIZE_MARKER {
            let (large_size_field, rest) = after_type
                .split_first_chunk::<8>()
                .ok_or_else(|| truncated_at(header_length(true, compact_type.is_none())))?;
            (Some(u64::from_be_bytes(*large_size_field)), rest)
        } else {
            (None, after_type)
        };

        let header_length = header_length(large_size.is_some(), compact_type.is_none());

        let (box_type, remainder) = match compact_type {
            Some(compact) => (BoxType::Compact(compact), after_large_size),
            None => {
                let (user_type_field, rest) = after_large_size
                    .split_first_chunk::<16>()
                    .ok_or_else(|| truncated_at(header_length))?;
                (BoxType::Extended(Uuid::new(*user_type_field)), rest)
            }
        };

        let size_below_header = DecodeError::SizeBelowHeader {
            declared: large_size.unwrap_or(u64::from(declared)),
            header_length: u64::from(header_length),
        };

        let size = match large_size {
            Some(large_size) => ExtendedSize::new(large_size)
                .map(BoxSize::Extended)
                .ok_or(size_below_header)?,
            None if declared == TO_END_OF_FILE_MARKER => BoxSize::ToEndOfFile,
            None => CompactSize::new(declared)
                .map(BoxSize::Compact)
                .ok_or(size_below_header)?,
        };

        Self::new(box_type, size)
            .map(|header| (header, remainder))
            .ok_or(size_below_header)
    }

    /// Writes the header into `buffer` and returns the bytes written
    ///
    /// The written prefix is 8, 16, 24, or 32 bytes long, depending on the
    /// forms the header carries; the rest of `buffer` is left untouched.
    #[must_use]
    pub fn encode<'buffer>(
        &self,
        buffer: &'buffer mut [u8; Self::MAX_ENCODED_LEN],
    ) -> &'buffer [u8] {
        let (declared, large_size) = match self.size {
            BoxSize::ToEndOfFile => (TO_END_OF_FILE_MARKER, None),
            BoxSize::Compact(size) => (size.get(), None),
            BoxSize::Extended(size) => (EXTENDED_SIZE_MARKER, Some(size.get())),
        };
        let user_type = match self.box_type {
            BoxType::Compact(_) => None,
            BoxType::Extended(user_type) => Some(user_type),
        };

        let (encoded, _beyond_header) = buffer.split_at_mut(usize::from(header_length(
            large_size.is_some(),
            user_type.is_some(),
        )));

        let (size_field, after_size) = encoded.split_at_mut(4);
        size_field.copy_from_slice(&declared.to_be_bytes());
        let (type_field, after_type) = after_size.split_at_mut(4);
        type_field.copy_from_slice(self.box_type.four_cc().as_bytes());

        let after_large_size = match large_size {
            Some(large_size) => {
                let (large_size_field, rest) = after_type.split_at_mut(8);
                large_size_field.copy_from_slice(&large_size.to_be_bytes());
                rest
            }
            None => after_type,
        };
        if let Some(user_type) = user_type {
            after_large_size.copy_from_slice(user_type.as_bytes());
        }

        encoded
    }
}

/// Reason a byte sequence does not start with a box
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DecodeError {
    /// Input ends inside the header
    TruncatedHeader {
        /// Bytes the header occupies, as far as the fields read so far tell
        needed: usize,
        /// Bytes the input offered
        available: usize,
    },
    /// Declared total is smaller than the header it prefixes
    SizeBelowHeader {
        /// Total the `size` or `largesize` field declares
        declared: u64,
        /// Bytes the header occupies
        header_length: u64,
    },
    /// Declared total overruns the input
    TruncatedBox {
        /// Bytes the box occupies, as the `size` or `largesize` field declares
        needed: u64,
        /// Bytes the input offered
        available: u64,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TruncatedHeader { needed, available } => write!(
                formatter,
                "box header of {needed} bytes cut short by an input of {available}"
            ),
            Self::SizeBelowHeader {
                declared,
                header_length,
            } => write!(
                formatter,
                "box declares a total of {declared} bytes, below its {header_length}-byte header"
            ),
            Self::TruncatedBox { needed, available } => write!(
                formatter,
                "box of {needed} bytes cut short by an input of {available}"
            ),
        }
    }
}

impl error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::{BoxHeader, DecodeError};
    use crate::box_size::{BoxSize, CompactSize, ExtendedSize};
    use crate::box_type::BoxType;
    use crate::uuid::Uuid;

    const USER_TYPE: Uuid = Uuid::new([
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ]);

    /// Every header form, in the wire order `size`, `type`, `largesize`, `usertype`
    const EVERY_FORM: [&[u8]; 6] = [
        &[0x00, 0x00, 0x00, 0x10, b'f', b'r', b'e', b'e'],
        &[
            0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x20,
        ],
        &[
            0x00, 0x00, 0x00, 0x20, b'u', b'u', b'i', b'd', 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ],
        &[
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x20, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98,
            0x76, 0x54, 0x32, 0x10,
        ],
        &[0x00, 0x00, 0x00, 0x00, b'm', b'd', b'a', b't'],
        &[
            0x00, 0x00, 0x00, 0x00, b'u', b'u', b'i', b'd', 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ],
    ];

    #[test]
    fn input_shorter_than_the_size_and_type_fields_is_rejected_as_truncated() {
        assert_eq!(
            BoxHeader::decode(&[0x00, 0x00, 0x00]),
            Err(DecodeError::TruncatedHeader {
                needed: 8,
                available: 3
            })
        );
    }

    #[test]
    fn input_ending_inside_the_large_size_field_is_truncated() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00, 0x00, 0x00, 0x01,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(DecodeError::TruncatedHeader {
                needed: 16,
                available: 12
            })
        );
    }

    #[test]
    fn a_user_type_box_cut_short_inside_the_large_size_field_needs_the_whole_header() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x01,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(DecodeError::TruncatedHeader {
                needed: 32,
                available: 12
            })
        );
    }

    #[test]
    fn input_ending_before_the_user_type_field_is_truncated() {
        let input = [0x00, 0x00, 0x00, 0x18, b'u', b'u', b'i', b'd'];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(DecodeError::TruncatedHeader {
                needed: 24,
                available: 8
            })
        );
    }

    #[test]
    fn a_total_below_the_size_and_type_fields_is_rejected() {
        let input = [0x00, 0x00, 0x00, 0x04, b'f', b'r', b'e', b'e'];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(DecodeError::SizeBelowHeader {
                declared: 4,
                header_length: 8
            })
        );
    }

    #[test]
    fn a_large_size_below_the_fields_it_is_stored_in_is_rejected() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x08,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(DecodeError::SizeBelowHeader {
                declared: 8,
                header_length: 16
            })
        );
    }

    #[test]
    fn a_total_that_leaves_out_the_user_type_field_is_rejected() {
        let input = [
            0x00, 0x00, 0x00, 0x14, b'u', b'u', b'i', b'd', 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(DecodeError::SizeBelowHeader {
                declared: 20,
                header_length: 24
            })
        );
    }

    #[test]
    fn a_large_size_that_leaves_out_the_user_type_field_is_rejected() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x10, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98,
            0x76, 0x54, 0x32, 0x10,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(DecodeError::SizeBelowHeader {
                declared: 16,
                header_length: 32
            })
        );
    }

    #[test]
    fn a_total_overrunning_the_input_decodes_as_it_stands() {
        let mut input = [0x00; 50];
        *input.first_chunk_mut::<8>().unwrap() = [0x00, 0x00, 0x00, 0x64, b'f', b'r', b'e', b'e'];

        assert_eq!(
            BoxHeader::decode(&input),
            Ok((
                BoxHeader::new(
                    BoxType::compact(*b"free"),
                    BoxSize::Compact(CompactSize::new(100).unwrap()),
                )
                .unwrap(),
                [0x00; 42].as_slice()
            ))
        );
    }

    #[test]
    fn the_end_of_file_size_leaves_the_rest_of_the_input_to_the_box() {
        let mut input = [0x00; 50];
        *input.first_chunk_mut::<8>().unwrap() = [0x00, 0x00, 0x00, 0x00, b'm', b'd', b'a', b't'];

        assert_eq!(
            BoxHeader::decode(&input),
            Ok((
                BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap(),
                [0x00; 42].as_slice()
            ))
        );
    }

    #[test]
    fn a_header_in_both_extended_forms_decodes_to_both() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x20, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98,
            0x76, 0x54, 0x32, 0x10,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Ok((
                BoxHeader::new(
                    BoxType::Extended(USER_TYPE),
                    BoxSize::Extended(ExtendedSize::new(0x0000_0001_0000_0020).unwrap()),
                )
                .unwrap(),
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn every_header_form_encodes_to_the_bytes_it_decoded_from() {
        for encoded in EVERY_FORM {
            let (header, _payload) = BoxHeader::decode(encoded).unwrap();
            let mut buffer = [0x00; BoxHeader::MAX_ENCODED_LEN];

            assert_eq!(header.encode(&mut buffer), encoded);
        }
    }

    #[test]
    fn a_total_that_leaves_no_room_for_the_user_type_field_is_not_a_header() {
        assert_eq!(
            BoxHeader::new(
                BoxType::Extended(USER_TYPE),
                BoxSize::Compact(CompactSize::new(20).unwrap()),
            ),
            None
        );
    }

    #[test]
    fn the_end_of_file_size_declares_no_total_and_fits_any_header() {
        let header = BoxHeader::new(BoxType::Extended(USER_TYPE), BoxSize::ToEndOfFile);

        assert_eq!(header.map(BoxHeader::size), Some(BoxSize::ToEndOfFile));
    }

    #[test]
    fn display_of_a_truncated_header_names_both_lengths() {
        let error = DecodeError::TruncatedHeader {
            needed: 16,
            available: 12,
        };

        assert_eq!(
            error.to_string(),
            "box header of 16 bytes cut short by an input of 12"
        );
    }

    #[test]
    fn display_of_a_truncated_box_names_both_lengths() {
        let error = DecodeError::TruncatedBox {
            needed: 32,
            available: 24,
        };

        assert_eq!(
            error.to_string(),
            "box of 32 bytes cut short by an input of 24"
        );
    }

    #[test]
    fn display_of_a_size_below_its_header_names_both_totals() {
        let error = DecodeError::SizeBelowHeader {
            declared: 20,
            header_length: 24,
        };

        assert_eq!(
            error.to_string(),
            "box declares a total of 20 bytes, below its 24-byte header"
        );
    }
}
