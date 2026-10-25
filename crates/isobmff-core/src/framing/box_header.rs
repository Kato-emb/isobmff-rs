//! [`BoxHeader`], the header every box opens with

use crate::data_types::fourcc::FourCc;
use crate::data_types::uuid::Uuid;
use crate::error::Error;
use crate::framing::box_size::{BoxSize, CompactSize, ExtendedSize};
use crate::framing::box_type::{BoxType, CompactType, ExtendedType};

/// Value of the `size` field that moves the total into the `largesize` field
const EXTENDED_SIZE_MARKER: u32 = 1;

/// Value of the `size` field that runs the payload to the end of what encloses the box
const TO_END_OF_FILE_MARKER: u32 = 0;

/// Byte length of a header carrying the given optional fields
const fn header_length(has_large_size: bool, has_extended_type: bool) -> u8 {
    match (has_large_size, has_extended_type) {
        (false, false) => 8,
        (true, false) => 16,
        (false, true) => 24,
        (true, true) => 32,
    }
}

/// Byte length of a header declaring `size` and `box_type`
fn header_length_of(size: BoxSize, box_type: BoxType) -> u8 {
    header_length(
        matches!(size, BoxSize::Extended(_)),
        matches!(box_type, BoxType::Extended(_)),
    )
}

/// Header that introduces a box and states how far it reaches, ISO/IEC 14496-12 §4.2.2
///
/// The wire order is `size`, `boxtype`, then the fields the extended forms
/// add: `largesize` for [`BoxSize::Extended`] and `extended_type` for
/// [`BoxType::Extended`]. A header is therefore 8, 16, 24, or 32 bytes long.
///
/// Every value declares a total that covers the header it prefixes.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize};
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
///         BoxType::extended([0xab; 16]),
///         BoxSize::Compact(CompactSize::new(16).unwrap()),
///     ),
///     None
/// );
///
/// // The header is written as the bytes it is read from; the payload is not read
/// let mut buffer = [0; BoxHeader::MAX_ENCODED_LEN];
/// assert_eq!(header.write_to(&mut buffer), Ok(b"\0\0\0\x10free".as_slice()));
/// assert_eq!(BoxHeader::decode(b"\0\0\0\x10free"), Ok((header, b"".as_slice())));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BoxHeader {
    box_type: BoxType,
    size: BoxSize,
}

impl BoxHeader {
    /// Buffer length [`write_to`](Self::write_to) writes into: the longest header
    pub const MAX_ENCODED_LEN: usize = header_length(true, true) as usize;

    /// Creates a header from a box type and a size
    ///
    /// Returns `None` when the declared total is smaller than the header those
    /// two forms occupy — 8 bytes, plus 8 for a `largesize` field and 16 for an
    /// `extended_type` field. [`BoxSize::ToEndOfFile`] declares no total and is
    /// accepted with either box type.
    #[must_use]
    pub fn new(box_type: BoxType, size: BoxSize) -> Option<Self> {
        match size.total_bytes() {
            Some(total) if total < u64::from(header_length_of(size, box_type)) => None,
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
    /// so leaves the payload to run as far as what encloses the box does.
    #[must_use]
    pub fn payload_len(self) -> Option<u64> {
        self.size
            .total_bytes()?
            .checked_sub(u64::from(header_length_of(self.size, self.box_type)))
    }

    /// Returns the length of the header itself, payload excluded
    ///
    /// This is what [`write_to`](Self::write_to) writes, and what stands between
    /// the start of the box and the payload
    /// [`payload_len`](Self::payload_len) measures.
    #[must_use]
    pub fn encoded_len(self) -> usize {
        usize::from(header_length_of(self.size, self.box_type))
    }

    /// Creates the header that introduces a payload of the given length
    ///
    /// The total goes in the `size` field where it fits and moves to the
    /// `largesize` field where it does not, so the header is the shortest one
    /// that can declare the box. An `extended_type` field is included whenever
    /// `box_type` carries one.
    ///
    /// Returns `None` when the total of header and payload overruns `u64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_core::{BoxHeader, BoxSize, BoxType, CompactSize};
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
    /// // An extended type takes sixteen bytes more, which the total must cover
    /// let vendor = BoxType::extended([0xab; 16]);
    /// assert_eq!(BoxHeader::with_payload_len(vendor, 4).unwrap().encoded_len(), 24);
    ///
    /// // A payload leaving no room for the header it needs
    /// assert_eq!(BoxHeader::with_payload_len(BoxType::compact(*b"mdat"), u64::MAX), None);
    /// ```
    #[must_use]
    pub fn with_payload_len(box_type: BoxType, payload_len: u64) -> Option<Self> {
        Self::compact(box_type, payload_len).or_else(|| {
            let has_extended_type = matches!(box_type, BoxType::Extended(_));
            let total =
                payload_len.checked_add(u64::from(header_length(true, has_extended_type)))?;

            Some(Self {
                box_type,
                size: BoxSize::Extended(ExtendedSize::new(total)?),
            })
        })
    }

    /// Creates the header that introduces a payload of the given length, with the total in the `size` field alone
    ///
    /// The total goes in the 32-bit `size` field, and `None` is returned where
    /// it does not fit there, where
    /// [`with_payload_len`](Self::with_payload_len) would reach for the
    /// `largesize` field. An `extended_type` field is included whenever
    /// `box_type` carries one.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_core::{BoxHeader, BoxType};
    ///
    /// // A payload the `size` field can declare gets the eight-byte header
    /// let header = BoxHeader::compact(BoxType::compact(*b"mdat"), 4).unwrap();
    /// assert_eq!(header, BoxHeader::with_payload_len(BoxType::compact(*b"mdat"), 4).unwrap());
    /// assert_eq!(header.encoded_len(), 8);
    ///
    /// // A payload only `largesize` can declare gets no header
    /// assert_eq!(BoxHeader::compact(BoxType::compact(*b"mdat"), u32::MAX.into()), None);
    /// ```
    #[must_use]
    pub fn compact(box_type: BoxType, payload_len: u64) -> Option<Self> {
        let has_extended_type = matches!(box_type, BoxType::Extended(_));
        let size = payload_len
            .checked_add(u64::from(header_length(false, has_extended_type)))
            .and_then(|total| u32::try_from(total).ok())
            .and_then(CompactSize::new)?;

        Some(Self {
            box_type,
            size: BoxSize::Compact(size),
        })
    }

    /// Decodes the header that starts `input`
    ///
    /// Returns the header and the bytes after it, where the payload of the box
    /// begins. At most [`MAX_ENCODED_LEN`](Self::MAX_ENCODED_LEN) bytes are
    /// read, and the payload is not examined: a declared total that overruns
    /// `input` decodes as it stands.
    ///
    /// An `extended_type` that ISO/IEC 14496-12 §4.2.3 reserves is kept as
    /// found; [`write_to`](Self::write_to) refuses to write it back.
    ///
    /// # Errors
    ///
    /// * [`TruncatedHeader`](crate::ErrorKind::TruncatedHeader): `input` ends inside
    ///   the header. A caller that reads in chunks can extend `input` to
    ///   `needed` bytes and decode again; once `input` holds the eight bytes
    ///   the `size` and `boxtype` fields occupy, `needed` is the length of the
    ///   whole header, so one such extension always suffices.
    /// * [`SizeBelowHeader`](crate::ErrorKind::SizeBelowHeader): the declared total
    ///   is smaller than the header it prefixes.
    pub fn decode(input: &[u8]) -> Result<(Self, &[u8]), Error> {
        let truncated_at = |needed: u8| {
            Error::truncated_header(
                u64::from(needed),
                u64::try_from(input.len()).unwrap_or(u64::MAX),
            )
        };

        let (size_field, after_size) = input
            .split_first_chunk::<4>()
            .ok_or_else(|| truncated_at(header_length(false, false)))?;
        let (type_field, after_type) = after_size
            .split_first_chunk::<4>()
            .ok_or_else(|| truncated_at(header_length(false, false)))?;

        let declared = u32::from_be_bytes(*size_field);
        let compact_type = CompactType::new(FourCc::new(*type_field));

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
                let (extended_type_field, rest) = after_large_size
                    .split_first_chunk::<16>()
                    .ok_or_else(|| truncated_at(header_length))?;
                let extended = ExtendedType::as_found(Uuid::new(*extended_type_field));
                (BoxType::Extended(extended), rest)
            }
        };

        let size_below_header = || {
            Error::size_below_header(
                u64::from(header_length),
                large_size.unwrap_or(u64::from(declared)),
            )
        };

        let size = match large_size {
            Some(large_size) => ExtendedSize::new(large_size)
                .map(BoxSize::Extended)
                .ok_or_else(size_below_header)?,
            None if declared == TO_END_OF_FILE_MARKER => BoxSize::ToEndOfFile,
            None => CompactSize::new(declared)
                .map(BoxSize::Compact)
                .ok_or_else(size_below_header)?,
        };

        Self::new(box_type, size)
            .map(|header| (header, remainder))
            .ok_or_else(size_below_header)
    }

    /// Writes the header into `buffer` and returns the bytes written
    ///
    /// The written prefix is 8, 16, 24, or 32 bytes long, depending on the
    /// forms the header carries; the rest of `buffer` is left untouched.
    ///
    /// # Errors
    ///
    /// * [`InvalidValue`](crate::ErrorKind::InvalidValue) naming
    ///   `extended_type`: the header was read off bytes declaring an
    ///   `extended_type` that ISO/IEC 14496-12 §4.2.3 reserves. Nothing is
    ///   written.
    pub fn write_to<'buffer>(
        &self,
        buffer: &'buffer mut [u8; Self::MAX_ENCODED_LEN],
    ) -> Result<&'buffer [u8], Error> {
        let (declared, large_size) = match self.size {
            BoxSize::ToEndOfFile => (TO_END_OF_FILE_MARKER, None),
            BoxSize::Compact(size) => (size.get(), None),
            BoxSize::Extended(size) => (EXTENDED_SIZE_MARKER, Some(size.get())),
        };
        let extended_type = match self.box_type {
            BoxType::Compact(_) => None,
            BoxType::Extended(extended) if ExtendedType::new(extended.uuid()).is_none() => {
                return Err(Error::invalid_value("extended_type"));
            }
            BoxType::Extended(extended) => Some(extended.uuid()),
        };

        let (encoded, _beyond_header) = buffer.split_at_mut(self.encoded_len());

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
        if let Some(extended_type) = extended_type {
            after_large_size.copy_from_slice(extended_type.as_bytes());
        }

        Ok(encoded)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use super::{BoxHeader, Error};
    use crate::framing::box_size::{BoxSize, CompactSize, ExtendedSize};
    use crate::framing::box_type::BoxType;

    const EXTENDED_TYPE: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];

    /// Every header form, in the wire order `size`, `boxtype`, `largesize`, `extended_type`
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

    /// Header of a `uuid` box whose `extended_type` is the one `free` maps to under §4.2.3
    const RESERVED_EXTENDED_TYPE: [u8; 24] = [
        0x00, 0x00, 0x00, 0x18, b'u', b'u', b'i', b'd', b'f', b'r', b'e', b'e', 0x00, 0x11, 0x00,
        0x10, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
    ];

    #[test]
    fn input_shorter_than_the_size_and_boxtype_fields_is_rejected_as_truncated() {
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
    fn an_extended_type_box_cut_short_inside_the_large_size_field_needs_the_whole_header() {
        let input = [
            0x00, 0x00, 0x00, 0x01, b'u', b'u', b'i', b'd', 0x00, 0x00, 0x00, 0x01,
        ];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(Error::truncated_header(32, 12))
        );
    }

    #[test]
    fn input_ending_before_the_extended_type_field_is_truncated() {
        let input = [0x00, 0x00, 0x00, 0x18, b'u', b'u', b'i', b'd'];

        assert_eq!(
            BoxHeader::decode(&input),
            Err(Error::truncated_header(24, 8))
        );
    }

    #[test]
    fn a_total_below_the_size_and_boxtype_fields_is_rejected() {
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
    fn a_total_that_leaves_out_the_extended_type_field_is_rejected() {
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
    fn a_large_size_that_leaves_out_the_extended_type_field_is_rejected() {
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
                    BoxType::extended(EXTENDED_TYPE),
                    BoxSize::Extended(ExtendedSize::new(0x0000_0001_0000_0020).unwrap()),
                )
                .unwrap(),
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn every_header_form_writes_the_bytes_it_was_read_from() {
        for encoded in EVERY_FORM {
            let (header, _payload) = BoxHeader::decode(encoded).unwrap();
            let mut buffer = [0x00; BoxHeader::MAX_ENCODED_LEN];

            assert_eq!(header.write_to(&mut buffer), Ok(encoded));
        }
    }

    #[test]
    fn a_reserved_extended_type_is_read_as_found() {
        let (header, _payload) = BoxHeader::decode(&RESERVED_EXTENDED_TYPE).unwrap();

        assert_eq!(header.payload_len(), Some(0));
        assert_eq!(
            header.box_type().to_string(),
            "uuid 66726565-0011-0010-8000-00aa00389b71"
        );
    }

    #[test]
    fn a_reserved_extended_type_is_not_written_back() {
        let (header, _payload) = BoxHeader::decode(&RESERVED_EXTENDED_TYPE).unwrap();
        let mut buffer = [0xff; BoxHeader::MAX_ENCODED_LEN];

        assert_eq!(
            header.write_to(&mut buffer),
            Err(Error::invalid_value("extended_type"))
        );
        assert_eq!(buffer, [0xff; BoxHeader::MAX_ENCODED_LEN]);
    }

    #[test]
    fn the_payload_of_a_header_in_both_extended_forms_is_the_total_past_its_thirty_two_bytes() {
        let header = BoxHeader::new(
            BoxType::extended(EXTENDED_TYPE),
            BoxSize::Extended(ExtendedSize::new(40).unwrap()),
        );

        assert_eq!(header.and_then(BoxHeader::payload_len), Some(8));
    }

    #[test]
    fn a_compact_header_is_made_for_a_payload_the_size_field_declares_and_for_no_other() {
        let mdat = BoxType::compact(*b"mdat");
        let longest =
            BoxHeader::new(mdat, BoxSize::Compact(CompactSize::new(u32::MAX).unwrap())).unwrap();
        let longest_payload = longest.payload_len().unwrap();

        assert_eq!(BoxHeader::compact(mdat, longest_payload), Some(longest));
        assert_eq!(
            BoxHeader::compact(mdat, longest_payload.checked_add(1).unwrap()),
            None
        );
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
    fn the_end_of_file_size_declares_no_total_and_fits_any_header() {
        let header = BoxHeader::new(BoxType::extended(EXTENDED_TYPE), BoxSize::ToEndOfFile);

        assert_eq!(
            header.map(|header| (header.size(), header.payload_len())),
            Some((BoxSize::ToEndOfFile, None))
        );
    }
}
