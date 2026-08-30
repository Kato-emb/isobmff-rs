//! [`BoxDefinition`] and [`BoxFormat`], the box type of ISO/IEC 14496-12 §4.2 a
//! Definition subclause assigns, as a constant and as a value

use crate::framing::box_type::BoxType;

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

/// Box type that names a value on the wire
///
/// Most boxes know their type without a value in hand, and state it as the
/// constant of [`BoxDefinition`]; every such type is a `BoxFormat` too, by the
/// blanket implementation. Some do not: the spec declares classes whose box
/// type is a parameter of the class — `SampleEntry(format)` §8.5.2.2,
/// `TrackReferenceTypeBox(reference_type)` §8.3.3.2 and
/// `TrackGroupTypeBox(track_group_type)` §8.3.4 — and leaves the codes those
/// take open, for a registration or a derived specification to name one no
/// type here stands for. Such a value carries the code it was read under and
/// settles its type only once it exists, so it states that type here.
///
/// Whatever the source, the type a value reports is the one its box is written
/// under: [`BoxEncode::encode`](crate::BoxEncode::encode) and
/// [`AnyBox`](crate::AnyBox) take it from here.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDefinition, BoxFormat, BoxType};
///
/// // A box named by one code states it once, for the type
/// struct MediaDataBox;
///
/// impl BoxDefinition for MediaDataBox {
///     const BOX_TYPE: BoxType = BoxType::compact(*b"mdat");
/// }
///
/// // A track reference is named by the reference type it carries, which
/// // §8.3.3.2 leaves open, so the code travels in the value
/// struct TrackReferenceTypeBox {
///     reference_type: BoxType,
/// }
///
/// impl BoxFormat for TrackReferenceTypeBox {
///     fn box_type(&self) -> BoxType {
///         self.reference_type
///     }
/// }
///
/// assert_eq!(MediaDataBox.box_type(), MediaDataBox::BOX_TYPE);
/// assert_eq!(
///     TrackReferenceTypeBox { reference_type: BoxType::compact(*b"hint") }.box_type(),
///     BoxType::compact(*b"hint")
/// );
/// ```
pub trait BoxFormat {
    /// Returns the box type this value is written under
    #[must_use]
    fn box_type(&self) -> BoxType;
}

impl<Definition: BoxDefinition> BoxFormat for Definition {
    fn box_type(&self) -> BoxType {
        Self::BOX_TYPE
    }
}
