//! [`BoxRead`], the whole box of ISO/IEC 14496-12 §4.2 read into a value

use crate::codec::box_decode::BoxDecode;
use crate::codec::box_definition::BoxDefinition;
use crate::error::Error;
use crate::framing::raw_box::RawBox;

/// Value that reads itself from a whole box, header and all
///
/// [`BoxDecode`] reads the payload, and [`BoxDefinition`] names the box type;
/// between them a box lying at the front of an input is settled — the header
/// states the type that is there and how far the box reaches, and the payload
/// the header spans is what the value reads from. A value with both therefore
/// reads off an input already, and this trait is that combination — every such
/// value has it, and nothing has to be written to opt in. It is the mirror of
/// [`BoxWrite`](crate::BoxWrite), which writes the same box back.
///
/// A box the caller has no type for is split with [`RawBox::split_first`]
/// instead, which frames it without reading it. [`AnyBox`](crate::AnyBox)
/// carries its box type as a value rather than a constant, so it cannot
/// implement [`BoxDefinition`] and does not have this trait.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxRead, BoxType, BoxWrite};
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
pub trait BoxRead: BoxDefinition + BoxDecode {
    /// Reads the whole box at the front of `input` and returns what is left
    ///
    /// The box is framed first and read after: the header settles how far the
    /// box reaches, the type it declares is held against
    /// [`BOX_TYPE`](BoxDefinition::BOX_TYPE), and the payload the header spans
    /// is what [`decode_payload`](BoxDecode::decode_payload) reads. What lies
    /// past the box is returned as the remainder, so a caller reading boxes laid
    /// end to end passes that remainder to the next read.
    ///
    /// A box declaring [`ToEndOfFile`](crate::BoxSize::ToEndOfFile) spans the
    /// rest of `input` and leaves an empty remainder, so `input` has to end
    /// where the file does for such a box to read as the whole of itself.
    ///
    /// # Errors
    ///
    /// * The failures of [`RawBox::split_first`]: `input` does not frame a box.
    /// * [`BoxTypeMismatch`](crate::ErrorKind::BoxTypeMismatch): the box at the
    ///   front of `input` declares another type. The payload goes unread, so a
    ///   caller may frame the box again as the type it is.
    /// * What [`decode_payload`](BoxDecode::decode_payload) reports, for the
    ///   payload the header spans.
    fn decode(input: &[u8]) -> Result<(Self, &[u8]), Error> {
        let (framed, rest) = RawBox::split_first(input)?;
        let found = framed.header().box_type();

        if found != Self::BOX_TYPE {
            return Err(Error::box_type_mismatch(Self::BOX_TYPE, found));
        }

        Ok((Self::decode_payload(framed.payload())?, rest))
    }
}

impl<Payload: BoxDefinition + BoxDecode> BoxRead for Payload {}

#[cfg(test)]
mod tests {
    use super::BoxRead;
    use crate::codec::box_decode::BoxDecode;
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
