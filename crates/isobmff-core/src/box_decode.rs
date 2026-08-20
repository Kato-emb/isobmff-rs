//! [`BoxDecode`], the box payload of ISO/IEC 14496-12 §4.2 read into a value

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
///     fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
///         let mut reader = FieldReader::new(payload);
///         let sequence_number = reader.read_u32()?;
///         reader.finish()?;
///
///         Ok(Self { sequence_number })
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
    /// remainder to pass over.
    ///
    /// A container reads the boxes its payload holds, so its failures reach
    /// past its own fields: the ones a child brings are named on
    /// [`ErrorKind`](crate::ErrorKind), and the child that brought one is on the
    /// [`containers`](Error::containers) path of the failure.
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](crate::ErrorKind::TruncatedPayload) or
    ///   [`TrailingPayload`](crate::ErrorKind::TrailingPayload): `payload` ends
    ///   inside a field of `Self`, or holds bytes past them.
    /// * The failures a container brings, for a box whose payload is the boxes
    ///   it contains: its children do not frame, or one of them does not read
    ///   as the type it was gathered under.
    fn decode_payload(payload: &[u8]) -> Result<Self, Error>;
}
