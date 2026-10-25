//! [`BoxDecode`], a box read into a value

use crate::codec::box_definition::BoxDefinition;
use crate::codec::field::FieldReader;
use crate::error::Error;
use crate::framing::raw_box::RawBox;

/// Value that the payload of a box decodes into
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDecode, Error, FieldReader};
///
/// // A box whose payload is one 32-bit sequence number
/// #[derive(PartialEq, Debug)]
/// struct SequenceNumberBox {
///     sequence_number: u32,
/// }
///
/// impl BoxDecode for SequenceNumberBox {
///     fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
///         Ok(Self {
///             sequence_number: reader.read_u32()?,
///         })
///     }
/// }
///
/// // The payload arrives whole, the header already consumed by the caller
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07"),
///     Ok(SequenceNumberBox { sequence_number: 7 })
/// );
///
/// // A payload ending inside the field says how far it had to reach
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0"),
///     Err(Error::truncated_payload(4, 3))
/// );
///
/// // Bytes past the field are syntax a later revision added, and are skipped
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07\0\x01"),
///     Ok(SequenceNumberBox { sequence_number: 7 })
/// );
/// ```
pub trait BoxDecode: Sized {
    /// Reads the fields of `Self` off the front of the payload of one box
    ///
    /// The Syntax subclause of a box lays out the fields its payload is made
    /// of; an implementation is that layout in Rust, reading each field off
    /// `reader` in the order the box declares it. A field that runs to the end
    /// of the payload takes what is left with
    /// [`take_remainder`](FieldReader::take_remainder).
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of `Self`.
    /// * What the box makes of the fields it has read: a version it does not
    ///   read, a count that disagrees with what follows it, a value the
    ///   specification does not allow.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error>;

    /// Decodes the payload of one box into a value
    ///
    /// `payload` is that payload whole and nothing besides: framing is settled
    /// before the call and the header is gone, as
    /// [`RawBox::payload`](crate::RawBox::payload) leaves it. Routing a box type
    /// to the implementation that reads it is the caller's part as well.
    ///
    /// Bytes past the fields [`decode_fields`](Self::decode_fields) reads are
    /// skipped, as ISO/IEC 14496-12 §4.2.2 has a reader do with syntax added at
    /// the end of a box that it does not recognize. They are not kept: a value
    /// written back writes the fields it holds and nothing after them.
    ///
    /// # Errors
    ///
    /// * What [`decode_fields`](Self::decode_fields) reports for the fields.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        Self::decode_fields(&mut FieldReader::new(payload))
    }

    /// Reads the whole box at the front of `input` and returns what is left
    ///
    /// The header states the type that is there and how far the box reaches,
    /// and the payload the header spans is what
    /// [`decode_payload`](Self::decode_payload) reads. It is the mirror of
    /// [`BoxEncode::encode`](crate::BoxEncode::encode), which writes the same
    /// box back.
    ///
    /// The box is framed first and read after, so a header that frames nothing
    /// is refused before the type it declares is held against
    /// [`BOX_TYPE`](BoxDefinition::BOX_TYPE). What lies past the box is returned
    /// as the remainder, so a caller reading boxes laid end to end passes that
    /// remainder to the next read.
    ///
    /// # Errors
    ///
    /// * The failures of [`RawBox::split_first`]: `input` does not frame a box.
    /// * [`BoxTypeMismatch`](crate::ErrorKind::BoxTypeMismatch): the box at the
    ///   front of `input` declares another type.
    /// * What [`decode_payload`](Self::decode_payload) reports, for the
    ///   payload the header spans.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_core::{BoxDecode, BoxDefinition, BoxType, Error, FieldReader};
    ///
    /// // A box whose payload is one 32-bit sequence number
    /// #[derive(PartialEq, Debug)]
    /// struct SequenceNumberBox {
    ///     sequence_number: u32,
    /// }
    ///
    /// impl BoxDefinition for SequenceNumberBox {
    ///     const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
    /// }
    ///
    /// impl BoxDecode for SequenceNumberBox {
    ///     fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
    ///         Ok(Self {
    ///             sequence_number: reader.read_u32()?,
    ///         })
    ///     }
    /// }
    ///
    /// // What lies past the box is left for whoever reads next
    /// let input = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x08free";
    /// assert_eq!(
    ///     SequenceNumberBox::decode(input),
    ///     Ok((SequenceNumberBox { sequence_number: 7 }, b"\0\0\0\x08free".as_slice()))
    /// );
    ///
    /// // A box of another type is refused, and names what was there instead
    /// assert_eq!(
    ///     SequenceNumberBox::decode(b"\0\0\0\x08free"),
    ///     Err(Error::box_type_mismatch(
    ///         SequenceNumberBox::BOX_TYPE,
    ///         BoxType::compact(*b"free")
    ///     ))
    /// );
    /// ```
    fn decode(input: &[u8]) -> Result<(Self, &[u8]), Error>
    where
        Self: BoxDefinition,
    {
        let (framed, rest) = RawBox::split_first(input)?;
        let found = framed.header().box_type();

        if found != Self::BOX_TYPE {
            return Err(Error::box_type_mismatch(Self::BOX_TYPE, found));
        }

        Ok((Self::decode_payload(framed.payload())?, rest))
    }
}

#[cfg(test)]
mod tests {
    use super::BoxDecode;
    use crate::codec::box_definition::BoxDefinition;
    use crate::codec::field::FieldReader;
    use crate::error::Error;
    use crate::framing::box_type::BoxType;

    /// Box whose payload is one 32-bit sequence number
    #[derive(PartialEq, Debug)]
    struct SequenceNumberBox {
        sequence_number: u32,
    }

    impl BoxDefinition for SequenceNumberBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
    }

    impl BoxDecode for SequenceNumberBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self {
                sequence_number: reader.read_u32()?,
            })
        }
    }

    #[test]
    fn a_box_running_to_the_end_of_the_file_takes_the_rest_of_the_input() {
        let input = b"\0\0\0\0sqnc\0\0\0\x07";

        assert_eq!(
            SequenceNumberBox::decode(input),
            Ok((SequenceNumberBox { sequence_number: 7 }, b"".as_slice()))
        );
    }

    #[test]
    fn a_box_cut_short_by_the_input_is_refused_before_its_type_is_matched() {
        assert_eq!(
            SequenceNumberBox::decode(b"\0\0\0\x0cfree\0\0"),
            Err(Error::truncated_box(12, 10))
        );
    }

    #[test]
    fn syntax_added_at_the_end_of_a_box_is_skipped_and_what_follows_the_box_is_left() {
        let input = b"\0\0\0\x0esqnc\0\0\0\x07\0\x01\0\0\0\x08free";

        assert_eq!(
            SequenceNumberBox::decode(input),
            Ok((
                SequenceNumberBox { sequence_number: 7 },
                b"\0\0\0\x08free".as_slice()
            ))
        );
    }
}
