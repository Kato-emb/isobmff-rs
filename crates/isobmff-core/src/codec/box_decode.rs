//! [`BoxDecode`], the box payload of ISO/IEC 14496-12 §4.2 read into a value

use crate::codec::field::FieldReader;
use crate::error::Error;

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
    /// of the payload claims what is left with
    /// [`take_remainder`](FieldReader::take_remainder), which is the only way a
    /// field is bounded by the payload rather than by its own width.
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
}
