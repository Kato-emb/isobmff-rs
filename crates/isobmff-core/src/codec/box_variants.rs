//! [`BoxVariants`], a child slot of ISO/IEC 14496-12 that one of several box types fills

use crate::error::Error;
use crate::framing::box_type::BoxType;
use crate::framing::raw_box::RawBox;

/// Child slot the spec fills with one of several box types, of which a container holds one
///
/// §8.7.3.1 has the sample sizes of a sample table stated by either a `stsz` or
/// a `stz2`, and §8.7.5.1 its chunk offsets by either a `stco` or a `co64`: one
/// slot, several box types that write the one child it holds. Implementing
/// this trait names those types and reads a child of any of them into the value
/// of the slot, which
/// [`ChildBoxes::exactly_one_variant`](crate::ChildBoxes::exactly_one_variant)
/// builds the slot with.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, BoxVariants, ChildBoxes, Error, FieldReader, RawBox, boxes};
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
/// // A container stating the slot with its 16-bit variant
/// let payload = b"\0\0\0\x0asqn2\0\x07";
/// let mut sequence_numbers = ChildBoxes::new();
/// for child in boxes(payload) {
///     sequence_numbers.push(child.unwrap());
/// }
///
/// // The one child stated is read as the variant its type names
/// assert_eq!(
///     sequence_numbers.exactly_one_variant::<SequenceNumber>(),
///     Ok(SequenceNumber::Narrow(7))
/// );
/// ```
pub trait BoxVariants: Sized {
    /// Every box type that writes the slot
    const VARIANTS: &'static [BoxType];

    /// Reads `child`, a box of one of [`VARIANTS`](Self::VARIANTS), into the slot
    ///
    /// Routing a child of one of those types here belongs to the caller. What
    /// comes of a child of another type is unspecified: an implementation may
    /// read it as one of them or fail, and either way the fault is the
    /// caller's rather than one this reports.
    ///
    /// # Errors
    ///
    /// * Whatever the box the child is read as reports for its payload,
    ///   without the child's box type on the [`containers`](Error::containers)
    ///   path: the caller that routed the child adds it.
    fn decode_variant(child: RawBox<'_>) -> Result<Self, Error>;
}
