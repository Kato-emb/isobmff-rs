//! [`BoxVariants`], a child of a container that one of several box types writes

use crate::error::Error;
use crate::framing::box_type::BoxType;
use crate::framing::raw_box::RawBox;

/// Child of a container written by one of the variants of a box, of which the container holds one
///
/// ISO/IEC 14496-12 states some boxes in variants, one Definition listing
/// several box types: the sample sizes of a sample table are stated by an
/// `stsz` or an `stz2` (§8.7.3.1), and its chunk offsets by an `stco` or a
/// `co64` (§8.7.5.1), each under the quantity "Exactly one variant shall be
/// present". The media header of §8.4.5 — "Exactly one specific media header
/// shall be present" — is a child of the same shape. These are not the
/// variant forms of §4.2.4, which are versions of one box.
///
/// Implementing this trait names the box types of the variants and reads a
/// child of any of them into one value, which
/// [`ChildBoxes::take_exactly_one_variant`](crate::ChildBoxes::take_exactly_one_variant)
/// and its siblings take the child with.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "alloc")] {
/// use isobmff_core::{BoxType, BoxVariants, ChildBoxes, Error, FieldReader, RawBox};
///
/// // A sequence number stated in 32 bits by a `sqnc`, or in 16 bits by a `sqn2`
/// #[derive(PartialEq, Debug)]
/// enum SequenceNumber {
///     Wide(u32),
///     Narrow(u16),
/// }
///
/// impl BoxVariants for SequenceNumber {
///     const VARIANTS: &'static [BoxType] =
///         &[BoxType::compact(*b"sqnc"), BoxType::compact(*b"sqn2")];
///
///     fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
///         let mut reader = FieldReader::new(child.payload());
///         if child.header().box_type() == BoxType::compact(*b"sqnc") {
///             Ok(Self::Wide(reader.read_u32()?))
///         } else {
///             Ok(Self::Narrow(reader.read_u16()?))
///         }
///     }
/// }
///
/// // A container stating the sequence number with its 16-bit variant
/// let mut children = ChildBoxes::decode(b"\0\0\0\x0asqn2\0\x07").unwrap();
///
/// // The one child is read as the variant its type names
/// assert_eq!(
///     children.take_exactly_one_variant::<SequenceNumber>(),
///     Ok(SequenceNumber::Narrow(7))
/// );
/// # }
/// ```
pub trait BoxVariants: Sized {
    /// Box types of the variants
    const VARIANTS: &'static [BoxType];

    /// Reads `child`, a box of one of the [`VARIANTS`](Self::VARIANTS), into a value
    ///
    /// [`ChildBoxes`](crate::ChildBoxes) hands this a child of one of the
    /// [`VARIANTS`](Self::VARIANTS) and of no other type.
    ///
    /// # Errors
    ///
    /// * Whatever the box the child is read as reports for its payload, without
    ///   the child's box type on the [`containers`](Error::containers) path:
    ///   [`ChildBoxes`](crate::ChildBoxes) adds it.
    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error>;
}
