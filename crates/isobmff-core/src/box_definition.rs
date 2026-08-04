//! [`BoxDefinition`], the box type of ISO/IEC 14496-12 §4.2 a Definition subclause assigns

use crate::box_type::BoxType;

/// Box type the spec assigns to `Self`
///
/// The Definition subclause of a box states the type that names it on the wire;
/// implementing this trait is that statement in Rust. The type is available
/// without a value in hand, so a reader can route to it and a writer can put it
/// in a header it is about to build.
///
/// Identity stands apart from [`BoxDecode`](crate::BoxDecode) and
/// [`BoxEncode`](crate::BoxEncode): a definition may come with a decode, an
/// encode, both, or neither.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDefinition, BoxHeader, BoxSize, BoxType, CompactSize, Uuid};
///
/// // A box named by a four-character code
/// struct FreeSpaceBox;
///
/// impl BoxDefinition for FreeSpaceBox {
///     const BOX_TYPE: BoxType = BoxType::compact(*b"free");
/// }
///
/// // A vendor box named by the UUID of its `usertype` field
/// struct VendorBox;
///
/// impl BoxDefinition for VendorBox {
///     const BOX_TYPE: BoxType = BoxType::Extended(Uuid::new([0xab; 16]));
/// }
///
/// // The type of a box is known before any value of it exists
/// fn empty_header<Definition: BoxDefinition>() -> Option<BoxHeader> {
///     BoxHeader::new(Definition::BOX_TYPE, BoxSize::Compact(CompactSize::new(8)?))
/// }
///
/// assert_eq!(
///     empty_header::<FreeSpaceBox>().map(BoxHeader::box_type),
///     Some(FreeSpaceBox::BOX_TYPE)
/// );
///
/// // A user type needs 16 bytes more, which a total of eight does not cover
/// assert_eq!(empty_header::<VendorBox>(), None);
/// ```
pub trait BoxDefinition {
    /// Box type that names `Self` on the wire
    const BOX_TYPE: BoxType;
}
