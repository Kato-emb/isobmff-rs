//! [`BoxHeader`], the box header of ISO/IEC 14496-12 §4.2

use crate::data_types::fourcc::FourCC;
use crate::data_types::uuid::Uuid;
use crate::error::Error;
use crate::framing::box_size::{BoxSize, CompactSize, ExtendedSize};
use crate::framing::box_type::{BoxType, CompactType};

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
/// // The total covers the header, so the payload is what is left over
/// assert_eq!(header.payload_len(), Some(8));
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

    /// Returns the length of the payload the header spans, header excluded
    ///
    /// Returns `None` for [`BoxSize::ToEndOfFile`], which declares no total and
    /// so leaves the payload to run as far as the enclosing file does.
    #[must_use]
    pub const fn payload_len(self) -> Option<u64> {
        let header_length = header_length(
            matches!(self.size, BoxSize::Extended(_)),
            matches!(self.box_type, BoxType::Extended(_)),
        ) as u64;

        match self.size.total_bytes() {
            Some(total) => total.checked_sub(header_length),
            None => None,
        }
    }

    /// Returns the length of the header itself, payload excluded
    ///
    /// This is what [`encode`](Self::encode) writes, and what stands between
    /// the start of the box and the payload
    /// [`payload_len`](Self::payload_len) measures.
    #[must_use]
    pub const fn encoded_len(self) -> usize {
        header_length(
            matches!(self.size, BoxSize::Extended(_)),
            matches!(self.box_type, BoxType::Extended(_)),
        ) as usize
    }

    /// Creates the header that introduces a payload of the given length
    ///
    /// The total goes in the `size` field where it fits and moves to the
    /// `largesize` field where it does not, so the header is the shortest one
    /// that can declare the box. A `usertype` field is included whenever
    /// `box_type` carries one.
    ///
    /// Returns `None` when the total of header and payload overruns `u64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize, Uuid};
    ///
    /// // A payload the 32-bit `size` field can declare
    /// let header = BoxHeader::with_payload_len(BoxType::compact(*b"free"), 4).unwrap();
    /// assert_eq!(header.size(), BoxSize::Compact(CompactSize::new(12).unwrap()));
    /// assert_eq!(header.encoded_len(), 8);
    ///
    /// // A payload too long for that field moves the total into `largesize`
    /// let long = BoxHeader::with_payload_len(BoxType::compact(*b"mdat"), u32::MAX.into()).unwrap();
    /// assert_eq!(long.encoded_len(), 16);
    ///
    /// // A user type takes sixteen bytes more, which the total must cover
    /// let vendor = BoxType::Extended(Uuid::new([0xab; 16]));
    /// assert_eq!(BoxHeader::with_payload_len(vendor, 4).unwrap().encoded_len(), 24);
    ///
    /// // A payload leaving no room for the header it needs
    /// assert_eq!(BoxHeader::with_payload_len(BoxType::compact(*b"mdat"), u64::MAX), None);
    /// ```
    #[must_use]
    pub fn with_payload_len(box_type: BoxType, payload_len: u64) -> Option<Self> {
        let has_user_type = matches!(box_type, BoxType::Extended(_));

        let compact = payload_len
            .checked_add(u64::from(header_length(false, has_user_type)))
            .and_then(|total| u32::try_from(total).ok())
            .and_then(CompactSize::new);
        if let Some(size) = compact {
            return Some(Self {
                box_type,
                size: BoxSize::Compact(size),
            });
        }

        let total = payload_len.checked_add(u64::from(header_length(true, has_user_type)))?;

        Some(Self {
            box_type,
            size: BoxSize::Extended(ExtendedSize::new(total)?),
        })
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
    /// * [`TruncatedHeader`](crate::ErrorKind::TruncatedHeader): `input` ends inside
    ///   the header. A caller that reads in chunks can extend `input` to
    ///   `needed` bytes and decode again; once `input` holds the eight bytes
    ///   the `size` and `type` fields occupy, `needed` is the length of the
    ///   whole header, so one such extension always suffices.
    /// * [`SizeBelowHeader`](crate::ErrorKind::SizeBelowHeader): the declared total
    ///   is smaller than the header it prefixes.
    pub fn decode(input: &[u8]) -> Result<(Self, &[u8]), Error> {
        let truncated_at =
            |needed: u8| Error::truncated_header(u64::from(needed), input.len() as u64);

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

        let size_below_header = Error::size_below_header(
            u64::from(header_length),
            large_size.unwrap_or(u64::from(declared)),
        );

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

#[cfg(test)]
mod tests {
    use super::{BoxHeader, Error};
    use crate::data_types::uuid::Uuid;
    use crate::framing::box_size::{BoxSize, CompactSize, ExtendedSize};
    use crate::framing::box_type::BoxType;

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
            Err(Error::truncated_header(8, 3))
        );
    }

    #[test]
    fn input_ending_inside_the_large_size_field_is_truncated() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'm', b'd', b'a', b't', 0x00, 0x00, 0x00, 0x01,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(Error::truncated_header(16, 12))
        );
    }

    #[test]
    fn a_user_type_box_cut_short_inside_the_large_size_field_needs_the_whole_header() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x01,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(Error::truncated_header(32, 12))
        );
    }

    #[test]
    fn input_ending_before_the_user_type_field_is_truncated() {
        let input = [0x00, 0x00, 0x00, 0x18, b'u', b'u', b'i', b'd'];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(Error::truncated_header(24, 8))
        );
    }

    #[test]
    fn a_total_below_the_size_and_type_fields_is_rejected() {
        let input = [0x00, 0x00, 0x00, 0x04, b'f', b'r', b'e', b'e'];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(Error::size_below_header(8, 4))
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
            Err(Error::size_below_header(16, 8))
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
            Err(Error::size_below_header(24, 20))
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
            Err(Error::size_below_header(32, 16))
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
    fn the_payload_of_a_header_in_both_extended_forms_is_the_total_past_its_thirty_two_bytes() {
        let header = BoxHeader::new(
            BoxType::Extended(USER_TYPE),
            BoxSize::Extended(ExtendedSize::new(40).unwrap()),
        );

        assert_eq!(header.and_then(BoxHeader::payload_len), Some(8));
    }

    #[test]
    fn a_total_of_exactly_the_header_leaves_no_payload() {
        let header = BoxHeader::new(
            BoxType::compact(*b"free"),
            BoxSize::Compact(CompactSize::new(8).unwrap()),
        );

        assert_eq!(header.and_then(BoxHeader::payload_len), Some(0));
    }

    #[test]
    fn the_end_of_file_size_leaves_the_payload_without_a_declared_length() {
        let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::ToEndOfFile).unwrap();

        assert_eq!(header.payload_len(), None);
    }

    #[test]
    fn the_end_of_file_size_declares_no_total_and_fits_any_header() {
        let header = BoxHeader::new(BoxType::Extended(USER_TYPE), BoxSize::ToEndOfFile);

        assert_eq!(header.map(BoxHeader::size), Some(BoxSize::ToEndOfFile));
    }
}
