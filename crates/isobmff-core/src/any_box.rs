//! [`AnyBox`], the box of ISO/IEC 14496-12 §4.2 carried without its type

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

use crate::box_definition::BoxDefinition;
use crate::box_encode::{BoxEncode, EncodeError};
use crate::box_type::BoxType;
use crate::box_write::{encode_into, encoded_len_of};

/// Box payload once its type is erased
///
/// Blanket-implemented, so a payload joins by being what a box payload already
/// is — writable, printable, copyable, comparable, and safe to carry between
/// threads.
trait ErasedPayload: Any + BoxEncode + fmt::Debug + Send + Sync {
    /// Clones the payload back into an erased one
    fn clone_erased(&self) -> Box<dyn ErasedPayload>;

    /// Reports whether `other` is the same type and equal to `self`
    fn eq_erased(&self, other: &dyn ErasedPayload) -> bool;
}

impl<Payload> ErasedPayload for Payload
where
    Payload: Any + BoxEncode + fmt::Debug + Clone + PartialEq + Send + Sync,
{
    fn clone_erased(&self) -> Box<dyn ErasedPayload> {
        Box::new(self.clone())
    }

    fn eq_erased(&self, other: &dyn ErasedPayload) -> bool {
        let erased: &dyn Any = other;

        erased
            .downcast_ref::<Payload>()
            .is_some_and(|other| self == other)
    }
}

/// Payload of a box the reader has no type for, kept as the bytes it lies as
#[derive(Clone, PartialEq)]
struct OpaquePayload(Vec<u8>);

impl fmt::Debug for OpaquePayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "OpaquePayload({} bytes)", self.0.len())
    }
}

impl BoxEncode for OpaquePayload {
    fn payload_len(&self) -> u64 {
        // Why not unwrap: a usize above `u64::MAX` needs a 128-bit target to
        // exist, and saturating keeps the panic-free path.
        u64::try_from(self.0.len()).unwrap_or(u64::MAX)
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if actual != self.payload_len() {
            return Err(EncodeError::BufferLengthMismatch {
                expected: self.payload_len(),
                actual,
            });
        }

        buffer.copy_from_slice(&self.0);

        Ok(())
    }
}

/// Box carried without its payload type, under the box type that names it
///
/// Boxes of unlike types travel together as these: a container keeps the
/// children it has no field for in a `Vec` of them, and writes each back
/// without naming what it holds.
///
/// Two kinds of box fit. One the reader has a type for arrives through
/// [`From`], which takes the box type from the
/// [`BOX_TYPE`](BoxDefinition::BOX_TYPE) of that type so the two can never be
/// paired wrongly; [`downcast_ref`](Self::downcast_ref) and
/// [`downcast_mut`](Self::downcast_mut) reach it again. One the reader has no
/// type for arrives through [`raw`](Self::raw) as the bytes it lies as, and
/// comes back out of [`raw_payload`](Self::raw_payload).
///
/// # Examples
///
/// ```
/// use isobmff_core::{AnyBox, BoxDefinition, BoxType};
/// # use isobmff_core::{BoxEncode, EncodeError};
/// #
/// # #[derive(Clone, PartialEq, Debug)]
/// # struct SequenceNumberBox {
/// #     sequence_number: u32,
/// # }
/// #
/// # impl BoxDefinition for SequenceNumberBox {
/// #     const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
/// # }
/// #
/// # impl BoxEncode for SequenceNumberBox {
/// #     fn payload_len(&self) -> u64 {
/// #         4
/// #     }
/// #
/// #     fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
/// #         let mismatch = EncodeError::BufferLengthMismatch {
/// #             expected: 4,
/// #             actual: u64::try_from(buffer.len()).unwrap_or(u64::MAX),
/// #         };
/// #         let field = buffer.first_chunk_mut::<4>().ok_or(mismatch)?;
/// #         *field = self.sequence_number.to_be_bytes();
/// #
/// #         Ok(())
/// #     }
/// # }
/// #
/// // A box the reader has a type for keeps that type inside
/// let mut known = AnyBox::from(SequenceNumberBox { sequence_number: 7 });
/// assert_eq!(known.box_type(), SequenceNumberBox::BOX_TYPE);
/// assert_eq!(
///     known.downcast_ref(),
///     Some(&SequenceNumberBox { sequence_number: 7 })
/// );
///
/// // Editing it in place edits what gets written
/// known.downcast_mut::<SequenceNumberBox>().unwrap().sequence_number = 9;
///
/// // A box the reader has no type for keeps the bytes it lies as
/// let unknown = AnyBox::raw(BoxType::compact(*b"skip"), vec![1, 2, 3, 4]);
/// assert_eq!(unknown.raw_payload(), Some([1, 2, 3, 4].as_slice()));
/// assert_eq!(unknown.downcast_ref::<SequenceNumberBox>(), None);
///
/// // Either writes back as the whole box it stands for
/// let mut buffer = vec![0; 24];
/// let rest = known.encode(&mut buffer).unwrap();
/// unknown.encode(rest).unwrap();
/// assert_eq!(buffer, b"\0\0\0\x0csqnc\0\0\0\x09\0\0\0\x0cskip\x01\x02\x03\x04");
/// ```
#[derive(Debug)]
pub struct AnyBox {
    box_type: BoxType,
    payload: Box<dyn ErasedPayload>,
}

impl AnyBox {
    /// Creates a box from the bytes of a payload no type was available for
    ///
    /// `payload` is the payload of the box whole, header excluded, as
    /// [`RawBox::payload`](crate::RawBox::payload) leaves it.
    #[must_use]
    pub fn raw(box_type: BoxType, payload: Vec<u8>) -> Self {
        Self {
            box_type,
            payload: Box::new(OpaquePayload(payload)),
        }
    }

    /// Returns the box type that names the payload carried
    #[must_use]
    pub const fn box_type(&self) -> BoxType {
        self.box_type
    }

    /// Returns the payload as the bytes it lies as, for a box built by
    /// [`raw`](Self::raw)
    ///
    /// Returns `None` for a box built from a payload type, whose bytes exist
    /// only once [`encode`](Self::encode) writes them.
    #[must_use]
    pub fn raw_payload(&self) -> Option<&[u8]> {
        let erased: &dyn Any = self.payload.as_ref();

        erased
            .downcast_ref::<OpaquePayload>()
            .map(|opaque| opaque.0.as_slice())
    }

    /// Returns the length of the whole box, header included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        encoded_len_of(self.box_type, self)
    }

    /// Writes the whole box into the front of `buffer` and returns what is left
    ///
    /// This is [`BoxWrite::encode`](crate::BoxWrite::encode) under the same
    /// contract, which `AnyBox` cannot have as that trait: the box type is a
    /// value here rather than the constant [`BoxDefinition`] declares.
    ///
    /// # Errors
    ///
    /// * [`BufferTooShort`](EncodeError::BufferTooShort): `buffer` is shorter
    ///   than [`encoded_len`](Self::encoded_len).
    /// * What [`encode_payload`](BoxEncode::encode_payload) reports for the
    ///   payload carried.
    pub fn encode<'buffer>(
        &self,
        buffer: &'buffer mut [u8],
    ) -> Result<&'buffer mut [u8], EncodeError> {
        encode_into(self.box_type, self, buffer)
    }

    /// Returns a reference to the payload carried, when it is a `Payload`
    ///
    /// Returns `None` when the payload was erased from another type, even one
    /// that names the same box.
    #[must_use]
    pub fn downcast_ref<Payload>(&self) -> Option<&Payload>
    where
        Payload: Any + BoxEncode + fmt::Debug + Clone + PartialEq + Send + Sync,
    {
        let erased: &dyn Any = self.payload.as_ref();

        erased.downcast_ref::<Payload>()
    }

    /// Returns a mutable reference to the payload carried, when it is a
    /// `Payload`
    ///
    /// Editing through it edits what [`encoded_len`](Self::encoded_len) and
    /// [`encode`](Self::encode) go on to write.
    #[must_use]
    pub fn downcast_mut<Payload>(&mut self) -> Option<&mut Payload>
    where
        Payload: Any + BoxEncode + fmt::Debug + Clone + PartialEq + Send + Sync,
    {
        let erased: &mut dyn Any = self.payload.as_mut();

        erased.downcast_mut::<Payload>()
    }
}

impl Clone for AnyBox {
    fn clone(&self) -> Self {
        Self {
            box_type: self.box_type,
            payload: self.payload.clone_erased(),
        }
    }
}

impl PartialEq for AnyBox {
    fn eq(&self, other: &Self) -> bool {
        self.box_type == other.box_type && self.payload.eq_erased(other.payload.as_ref())
    }
}

impl BoxEncode for AnyBox {
    fn payload_len(&self) -> u64 {
        self.payload.payload_len()
    }

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        self.payload.encode_payload(buffer)
    }
}

impl<Payload> From<Payload> for AnyBox
where
    Payload: BoxDefinition + Any + BoxEncode + fmt::Debug + Clone + PartialEq + Send + Sync,
{
    fn from(payload: Payload) -> Self {
        Self {
            box_type: Payload::BOX_TYPE,
            payload: Box::new(payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::AnyBox;
    use crate::box_definition::BoxDefinition;
    use crate::box_encode::{BoxEncode, EncodeError};
    use crate::box_type::BoxType;
    use crate::box_write::BoxWrite as _;

    /// Box whose payload is one byte, standing in for a type the reader has
    #[derive(Clone, PartialEq, Debug)]
    struct MarkerBox(u8);

    impl BoxDefinition for MarkerBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"mark");
    }

    impl BoxEncode for MarkerBox {
        fn payload_len(&self) -> u64 {
            1
        }

        fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
            let mismatch = EncodeError::BufferLengthMismatch {
                expected: 1,
                actual: u64::try_from(buffer.len()).unwrap_or(u64::MAX),
            };
            if buffer.len() != 1 {
                return Err(mismatch);
            }
            let field = buffer.first_chunk_mut::<1>().ok_or(mismatch)?;
            *field = [self.0];

            Ok(())
        }
    }

    #[test]
    fn boxes_of_one_type_differing_in_payload_are_unequal() {
        assert_ne!(AnyBox::from(MarkerBox(7)), AnyBox::from(MarkerBox(9)));
    }

    #[test]
    fn a_payload_type_and_the_same_bytes_carried_raw_are_unequal() {
        assert_ne!(
            AnyBox::from(MarkerBox(7)),
            AnyBox::raw(MarkerBox::BOX_TYPE, vec![7])
        );
    }

    #[test]
    fn a_clone_carries_the_payload_type_it_was_cloned_from() {
        let known = AnyBox::from(MarkerBox(7));

        let cloned = known.clone();

        assert_eq!(cloned, known);
        assert_eq!(cloned.downcast_ref(), Some(&MarkerBox(7)));
    }

    #[test]
    fn only_a_raw_box_offers_the_bytes_of_its_payload() {
        assert_eq!(AnyBox::from(MarkerBox(7)).raw_payload(), None);
        assert_eq!(
            AnyBox::raw(MarkerBox::BOX_TYPE, vec![7]).raw_payload(),
            Some([7].as_slice())
        );
    }

    #[test]
    fn a_payload_of_another_type_does_not_come_back_out() {
        assert_eq!(
            AnyBox::raw(MarkerBox::BOX_TYPE, vec![7]).downcast_ref::<MarkerBox>(),
            None
        );
    }

    #[test]
    fn an_erased_box_writes_what_its_payload_type_would_have_written() {
        let mut written_directly = vec![0; 9];
        let mut written_erased = vec![0; 9];

        MarkerBox(7).encode(&mut written_directly).unwrap();
        AnyBox::from(MarkerBox(7))
            .encode(&mut written_erased)
            .unwrap();

        assert_eq!(written_directly, written_erased);
        assert_eq!(written_directly, b"\0\0\0\x09mark\x07");
    }
}
