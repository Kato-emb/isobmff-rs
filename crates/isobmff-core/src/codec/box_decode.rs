//! [`BoxDecode`], the box of ISO/IEC 14496-12 §4.2 read into a value

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
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07").unwrap(),
///     SequenceNumberBox { sequence_number: 7 }
/// );
///
/// // A payload ending inside the field says how far it had to reach
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0"),
///     Err(Error::truncated_payload(4, 3))
/// );
///
/// // Bytes past the field are an error, not a remainder to skip over
/// assert_eq!(
///     SequenceNumberBox::decode_payload(b"\0\0\0\x07!"),
///     Err(Error::trailing_payload(4, 5))
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
    /// What the payload is, and that every byte of it belongs to a field, is
    /// settled by [`decode_payload`](Self::decode_payload) around this call.
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of `Self`.
    /// * What the box makes of the fields it has read: a version or a flag it
    ///   does not read, a count that disagrees with what follows it, or the
    ///   failures a container brings.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error>;

    /// Decodes the payload of one box into a value
    ///
    /// `payload` is that payload whole and nothing besides: framing is settled
    /// before the call and the header is gone, as
    /// [`RawBox::payload`](crate::RawBox::payload) leaves it. Routing a box type
    /// to the implementation that reads it is the caller's part as well.
    ///
    /// Reading is strict. Every byte of `payload` belongs to a field of `Self`,
    /// and bytes the fields do not claim are
    /// [`TrailingPayload`](crate::ErrorKind::TrailingPayload) rather than a
    /// remainder to pass over. This is what the method holds
    /// [`decode_fields`](Self::decode_fields) to, so an implementation states
    /// the fields and leaves this one as it stands.
    ///
    /// A container reads the boxes its payload holds, so its failures reach
    /// past its own fields: the ones a child brings are named on
    /// [`ErrorKind`](crate::ErrorKind), and the child that brought one is on the
    /// [`containers`](Error::containers) path of the failure.
    ///
    /// # Errors
    ///
    /// * [`TrailingPayload`](crate::ErrorKind::TrailingPayload): `payload` holds
    ///   bytes past the fields of `Self`.
    /// * What [`decode_fields`](Self::decode_fields) reports for the fields
    ///   themselves.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let value = Self::decode_fields(&mut reader)?;
        reader.finish()?;

        Ok(value)
    }

    /// Reads the whole box at the front of `input` and returns what is left
    ///
    /// [`BoxDefinition`] names the box type, and the payload decodes above;
    /// between them a box lying at the front of an input is settled — the
    /// header states the type that is there and how far the box reaches, and
    /// the payload the header spans is what
    /// [`decode_payload`](Self::decode_payload) reads. A value with both
    /// therefore reads off an input already, and this method is that
    /// combination — it asks nothing further of the box. It is the mirror of
    /// [`BoxEncode::encode`](crate::BoxEncode::encode), which writes the same
    /// box back.
    ///
    /// The box is framed first and read after, so a header that frames nothing
    /// is refused before the type it declares is held against
    /// [`BOX_TYPE`](BoxDefinition::BOX_TYPE). What lies past the box is returned
    /// as the remainder, so a caller reading boxes laid end to end passes that
    /// remainder to the next read.
    ///
    /// A box declaring [`ToEndOfFile`](crate::BoxSize::ToEndOfFile) spans the
    /// rest of `input` and leaves an empty remainder, so `input` has to end
    /// where the file does for such a box to read as the whole of itself.
    ///
    /// A box the caller has no type for is split with [`RawBox::split_first`]
    /// instead, which frames it without reading it. [`AnyBox`](crate::AnyBox)
    /// carries its box type as a value rather than a constant, so it cannot
    /// implement [`BoxDefinition`] and does not have this method.
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
    /// use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType};
    /// use isobmff_core::{Error, FieldReader, FieldWriter};
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
    /// impl BoxEncode for SequenceNumberBox {
    ///     fn payload_len(&self) -> u64 {
    ///         4
    ///     }
    ///
    ///     fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
    ///         writer.write_u32(self.sequence_number)
    ///     }
    /// }
    ///
    /// // A box written whole reads back as the value that wrote it
    /// let sequence = SequenceNumberBox { sequence_number: 7 };
    /// let mut buffer = vec![0; usize::try_from(sequence.encoded_len()).unwrap()];
    /// sequence.encode(&mut buffer).unwrap();
    ///
    /// let (read, rest) = SequenceNumberBox::decode(&buffer).unwrap();
    /// assert_eq!(read, sequence);
    /// assert!(rest.is_empty());
    ///
    /// // What lies past the box is left for whoever reads next
    /// let input = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x08free";
    /// let (_sequence, rest) = SequenceNumberBox::decode(input).unwrap();
    /// assert_eq!(rest, b"\0\0\0\x08free");
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
    fn a_box_reads_off_the_front_of_the_input_and_leaves_what_follows() {
        let input = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x08free";

        assert_eq!(
            SequenceNumberBox::decode(input).unwrap(),
            (
                SequenceNumberBox { sequence_number: 7 },
                b"\0\0\0\x08free".as_slice()
            )
        );
    }

    #[test]
    fn a_box_running_to_the_end_of_the_file_takes_the_rest_of_the_input() {
        let input = b"\0\0\0\0sqnc\0\0\0\x07";

        assert_eq!(
            SequenceNumberBox::decode(input).unwrap(),
            (SequenceNumberBox { sequence_number: 7 }, b"".as_slice())
        );
    }

    #[test]
    fn a_box_of_another_type_is_refused_before_its_payload_is_read() {
        assert_eq!(
            SequenceNumberBox::decode(b"\0\0\0\x0cfree\0\0\0\x07"),
            Err(Error::box_type_mismatch(
                SequenceNumberBox::BOX_TYPE,
                BoxType::compact(*b"free")
            ))
        );
    }

    #[test]
    fn a_box_cut_short_by_the_input_is_refused_before_its_type_is_matched() {
        assert_eq!(
            SequenceNumberBox::decode(b"\0\0\0\x0cfree\0\0"),
            Err(Error::truncated_box(12, 10))
        );
    }
}
