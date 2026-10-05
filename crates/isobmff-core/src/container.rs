//! [`ChildBoxes`] and [`OtherBoxes`], the boxes a container box of ISO/IEC 14496-12 §4.2 holds

use alloc::vec::Vec;

use crate::any_box::AnyBox;
use crate::codec::box_decode::BoxDecode;
use crate::codec::box_definition::BoxDefinition;
use crate::codec::box_variants::BoxVariants;
use crate::error::{Error, InContainer};
use crate::framing::box_type::BoxType;
use crate::framing::raw_box::RawBox;

/// Children of a container, from which its fields are taken
///
/// A container collects every child its payload holds here, as the bytes they
/// were framed as, and takes each field out by the quantity the box tables of
/// ISO/IEC 14496-12 state for it: [`take_exactly_one`](Self::take_exactly_one),
/// [`take_zero_or_one`](Self::take_zero_or_one),
/// [`take_one_or_more`](Self::take_one_or_more),
/// [`take_zero_or_more`](Self::take_zero_or_more). Each takes out the children
/// of the type that names the field, hands back the field the quantity calls
/// for, and reports the counts the quantity forbids. What no take removed is
/// the container's [`OtherBoxes`], built with [`From`], in the order the
/// children came.
///
/// `Exactly one variant must be present` states a quantity across box types:
/// §8.7.3.1 writes the sample sizes as either a `stsz` or a `stz2`, and §8.7.5.1
/// the chunk offsets as either a `stco` or a `co64` — one slot, several ways of
/// writing the one child it holds.
/// [`take_exactly_one_variant`](Self::take_exactly_one_variant) and
/// [`take_zero_or_one_variant`](Self::take_zero_or_one_variant) take a child of
/// any type in [`BoxVariants::VARIANTS`] and read it as the variant its type
/// names.
///
/// A take reads only the children it removes, and reports a count the quantity
/// forbids before reading any of them.
///
/// A failure is reported as the [`Error`](BoxDecode::Error) of the child's
/// [`BoxDecode`]: the quantity's own failures are the [`Error`] they are named
/// by, converted into it, and a child's failure takes the child's box type
/// through [`InContainer`].
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxDecode, BoxDefinition, BoxType, ChildBoxes, Error, OtherBoxes};
/// use isobmff_core::{FieldReader, boxes};
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
///     type Error = Error;
///
///     fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
///         Ok(Self {
///             sequence_number: reader.read_u32()?,
///         })
///     }
/// }
///
/// // The payload of a container: the box a field claims, then one no field does
/// let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x08free";
/// let mut children: ChildBoxes<'_> = boxes(payload).collect::<Result<_, _>>().unwrap();
///
/// // The quantity the box table states is asked for once, as the field is taken
/// let sequence_number: SequenceNumberBox = children.take_exactly_one().unwrap();
/// assert_eq!(sequence_number, SequenceNumberBox { sequence_number: 7 });
///
/// // What no field took is kept as it came
/// let other_boxes = OtherBoxes::from(children);
/// assert_eq!(other_boxes.as_slice().len(), 1);
///
/// // A container holding none of a child it must hold does not read
/// assert_eq!(
///     ChildBoxes::new().take_exactly_one::<SequenceNumberBox>(),
///     Err(Error::missing_mandatory_box(SequenceNumberBox::BOX_TYPE))
/// );
/// ```
#[derive(Default, Debug)]
pub struct ChildBoxes<'payload> {
    children: Vec<RawBox<'payload>>,
}

impl<'payload> ChildBoxes<'payload> {
    /// Creates a collection of no child
    #[must_use]
    pub const fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    /// Returns whether a child of the type that names `Child` is there, without reading it
    #[must_use]
    pub fn contains<Child: BoxDefinition>(&self) -> bool {
        self.children
            .iter()
            .any(|child| child.header().box_type() == Child::BOX_TYPE)
    }

    /// Takes out the one child of a quantity of `Exactly one`
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox): no child of
    ///   the type is there.
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): more than one is.
    /// * Whatever the child reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn take_exactly_one<Child>(&mut self) -> Result<Child, Child::Error>
    where
        Child: BoxDecode + BoxDefinition,
        Child::Error: InContainer,
    {
        self.take_zero_or_one::<Child>()?
            .ok_or_else(|| Error::missing_mandatory_box(Child::BOX_TYPE).into())
    }

    /// Takes out the child of a quantity of `Zero or one`, if it is there
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): more than one child of
    ///   the type is there.
    /// * Whatever the child reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn take_zero_or_one<Child>(&mut self) -> Result<Option<Child>, Child::Error>
    where
        Child: BoxDecode + BoxDefinition,
        Child::Error: InContainer,
    {
        self.take_at_most_one_of(const { &[Child::BOX_TYPE] })
            .map_err(Child::Error::from)?
            .map(decode::<Child>)
            .transpose()
    }

    /// Takes out the children of a quantity of `One or more`, in the order they came
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox): no child of
    ///   the type is there.
    /// * Whatever the first of the children that does not read reports, with its
    ///   box type on the [`containers`](Error::containers) path of the failure.
    pub fn take_one_or_more<Child>(&mut self) -> Result<Vec<Child>, Child::Error>
    where
        Child: BoxDecode + BoxDefinition,
        Child::Error: InContainer,
    {
        let read = self.take_zero_or_more::<Child>()?;
        if read.is_empty() {
            return Err(Error::missing_mandatory_box(Child::BOX_TYPE).into());
        }

        Ok(read)
    }

    /// Takes out the children of a quantity of `Zero or more`, in the order they came
    ///
    /// # Errors
    ///
    /// * Whatever the first of the children that does not read reports, with its
    ///   box type on the [`containers`](Error::containers) path of the failure.
    pub fn take_zero_or_more<Child>(&mut self) -> Result<Vec<Child>, Child::Error>
    where
        Child: BoxDecode + BoxDefinition,
        Child::Error: InContainer,
    {
        let mut read = Vec::new();
        let mut failure = None;
        self.children.retain(|child| {
            let claimed = child.header().box_type() == Child::BOX_TYPE;
            if claimed && failure.is_none() {
                match decode::<Child>(*child) {
                    Ok(value) => read.push(value),
                    Err(error) => failure = Some(error),
                }
            }

            !claimed
        });

        failure.map_or(Ok(read), Err)
    }

    /// Takes out the slot a quantity of `Exactly one variant must be present` fills
    ///
    /// # Errors
    ///
    /// * [`MissingAlternativeBox`](crate::ErrorKind::MissingAlternativeBox): no child
    ///   of a type in [`Slot::VARIANTS`](BoxVariants::VARIANTS) is there.
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): more than one is, all of
    ///   one type.
    /// * [`DuplicateAlternativeBox`](crate::ErrorKind::DuplicateAlternativeBox): more
    ///   than one is, of more than one type.
    /// * Whatever the child reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn take_exactly_one_variant<Slot: BoxVariants>(&mut self) -> Result<Slot, Error> {
        self.take_zero_or_one_variant()?
            .ok_or(Error::missing_alternative_box(Slot::VARIANTS))
    }

    /// Takes out the slot of several box types that holds at most one child, if the child is there
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): more than one child of a
    ///   type in [`Slot::VARIANTS`](BoxVariants::VARIANTS) is there, all of one
    ///   type.
    /// * [`DuplicateAlternativeBox`](crate::ErrorKind::DuplicateAlternativeBox): more
    ///   than one is, of more than one type.
    /// * Whatever the child reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn take_zero_or_one_variant<Slot: BoxVariants>(&mut self) -> Result<Option<Slot>, Error> {
        self.take_at_most_one_of(Slot::VARIANTS)?
            .map(|stated| {
                let box_type = stated.header().box_type();
                Slot::decode_variant(stated).map_err(|error| error.in_container(box_type))
            })
            .transpose()
    }

    /// Takes out the one child of a type in `box_types`, if one is there
    fn take_at_most_one_of(
        &mut self,
        box_types: &'static [BoxType],
    ) -> Result<Option<RawBox<'payload>>, Error> {
        let mut claimed = self
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| box_types.contains(&child.header().box_type()));
        let Some((position, first)) = claimed.next() else {
            return Ok(None);
        };
        let box_type = first.header().box_type();
        if let Some((_, second)) = claimed.next() {
            let of_one_type = second.header().box_type() == box_type
                && claimed.all(|(_, other)| other.header().box_type() == box_type);
            return Err(if of_one_type {
                Error::duplicate_box(box_type)
            } else {
                Error::duplicate_alternative_box(box_types)
            });
        }

        Ok(Some(self.children.remove(position)))
    }
}

impl<'payload> FromIterator<RawBox<'payload>> for ChildBoxes<'payload> {
    fn from_iter<Children: IntoIterator<Item = RawBox<'payload>>>(children: Children) -> Self {
        Self {
            children: children.into_iter().collect(),
        }
    }
}

/// Reads one child, naming it in whatever failure it reports
fn decode<Child>(child: RawBox<'_>) -> Result<Child, Child::Error>
where
    Child: BoxDecode + BoxDefinition,
    Child::Error: InContainer,
{
    Child::decode_payload(child.payload()).map_err(|error| error.in_container(Child::BOX_TYPE))
}

/// Children of a container that no field of it claims
///
/// They are kept as the bytes they lie as, under the box type that names them,
/// so a container writes back the children it has no field to read them into.
/// They are what no take removed from the container's [`ChildBoxes`], in the
/// order they came.
///
/// Where they go among the children the container does read is that container's
/// own canonical order, so summing them and writing them is that container's
/// part: this type hands them over with [`as_slice`](Self::as_slice), and the
/// container treats that run as it treats any run of children it holds.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, ChildBoxes, OtherBoxes, boxes};
///
/// // Two children of a container that has no field for either
/// let payload = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
/// let children: ChildBoxes<'_> = boxes(payload).collect::<Result<_, _>>().unwrap();
/// let other_boxes = OtherBoxes::from(children);
///
/// // Each is held under the box type that named it, in the order it came
/// let box_types: Vec<BoxType> = other_boxes
///     .as_slice()
///     .iter()
///     .map(|kept| kept.box_type())
///     .collect();
/// assert_eq!(
///     box_types,
///     [BoxType::compact(*b"free"), BoxType::compact(*b"skip")]
/// );
///
/// // The container sums them as it sums any run of children
/// let length = other_boxes
///     .as_slice()
///     .iter()
///     .fold(0_u64, |total, kept| total.saturating_add(kept.encoded_len()));
///
/// // And writes them where its own order puts them
/// let mut buffer = vec![0; usize::try_from(length).unwrap()];
/// let mut rest = buffer.as_mut_slice();
/// for kept in other_boxes.as_slice() {
///     rest = kept.encode(rest).unwrap();
/// }
/// assert_eq!(buffer, payload);
/// ```
#[derive(Clone, Default, PartialEq, Debug)]
pub struct OtherBoxes {
    children: Vec<AnyBox>,
}

impl OtherBoxes {
    /// Creates a holding of no child
    #[must_use]
    pub const fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    /// Returns the children kept, in the order they came
    #[must_use]
    pub fn as_slice(&self) -> &[AnyBox] {
        &self.children
    }
}

impl From<ChildBoxes<'_>> for OtherBoxes {
    /// Keeps the children no take removed, copying each payload out of the input it was framed from
    fn from(children: ChildBoxes<'_>) -> Self {
        Self {
            children: children
                .children
                .into_iter()
                .map(|child| {
                    AnyBox::from_raw_bytes(child.header().box_type(), child.payload().to_vec())
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{ChildBoxes, OtherBoxes};
    use crate::any_box::AnyBox;
    use crate::codec::box_decode::BoxDecode;
    use crate::codec::box_definition::BoxDefinition;
    use crate::codec::box_variants::BoxVariants;
    use crate::codec::field::FieldReader;
    use crate::error::Error;
    use crate::framing::box_type::BoxType;
    use crate::framing::raw_box::{RawBox, boxes};

    /// Box whose payload is one 32-bit sequence number
    #[derive(PartialEq, Debug)]
    struct SequenceNumberBox(u32);

    impl BoxDefinition for SequenceNumberBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
    }

    impl BoxDecode for SequenceNumberBox {
        type Error = Error;

        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self(reader.read_u32()?))
        }
    }

    /// Collects every box a payload holds, whatever type names it
    fn collected(payload: &[u8]) -> ChildBoxes<'_> {
        boxes(payload).collect::<Result<_, _>>().unwrap()
    }

    #[test]
    fn a_quantity_of_exactly_one_yields_the_child_it_holds() {
        let mut children = collected(b"\0\0\0\x0csqnc\0\0\0\x07");

        assert_eq!(
            children.take_exactly_one::<SequenceNumberBox>().unwrap(),
            SequenceNumberBox(7)
        );
    }

    #[test]
    fn a_quantity_of_exactly_one_refuses_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new().take_exactly_one::<SequenceNumberBox>(),
            Err(Error::missing_mandatory_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_quantity_of_zero_or_one_yields_nothing_for_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new()
                .take_zero_or_one::<SequenceNumberBox>()
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_quantity_of_at_most_one_refuses_a_second_child() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";

        assert_eq!(
            collected(payload).take_exactly_one::<SequenceNumberBox>(),
            Err(Error::duplicate_box(SequenceNumberBox::BOX_TYPE))
        );
        assert_eq!(
            collected(payload).take_zero_or_one::<SequenceNumberBox>(),
            Err(Error::duplicate_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    /// Sequence number stated by a `sqnc` or, as a box of the same payload, by a `sqn2`
    #[derive(PartialEq, Debug)]
    enum SequenceNumber {
        Primary(SequenceNumberBox),
        Alternative(SequenceNumberBox),
    }

    impl BoxVariants for SequenceNumber {
        const VARIANTS: &'static [BoxType] =
            &[SequenceNumberBox::BOX_TYPE, BoxType::compact(*b"sqn2")];

        fn decode_variant(child: RawBox<'_>) -> Result<Self, Error> {
            let sequence_number = SequenceNumberBox::decode_payload(child.payload())?;
            if child.header().box_type() == SequenceNumberBox::BOX_TYPE {
                Ok(Self::Primary(sequence_number))
            } else {
                Ok(Self::Alternative(sequence_number))
            }
        }
    }

    #[test]
    fn a_quantity_of_exactly_one_variant_reads_the_child_stated_as_its_variant() {
        let mut children = collected(b"\0\0\0\x0csqn2\0\0\0\x07");

        assert_eq!(
            children.take_exactly_one_variant::<SequenceNumber>(),
            Ok(SequenceNumber::Alternative(SequenceNumberBox(7)))
        );
    }

    #[test]
    fn a_quantity_of_exactly_one_variant_refuses_none_and_more_than_one() {
        let of_one_type = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";
        let of_two_types = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqn2\0\0\0\x09";

        assert_eq!(
            ChildBoxes::new().take_exactly_one_variant::<SequenceNumber>(),
            Err(Error::missing_alternative_box(SequenceNumber::VARIANTS))
        );
        assert_eq!(
            collected(of_one_type).take_exactly_one_variant::<SequenceNumber>(),
            Err(Error::duplicate_box(SequenceNumberBox::BOX_TYPE))
        );
        assert_eq!(
            collected(of_two_types).take_exactly_one_variant::<SequenceNumber>(),
            Err(Error::duplicate_alternative_box(SequenceNumber::VARIANTS))
        );
    }

    #[test]
    fn a_variant_whose_payload_fails_to_read_is_named_on_the_path_of_the_failure() {
        let mut children = collected(b"\0\0\0\x09sqn2!");

        assert_eq!(
            children.take_exactly_one_variant::<SequenceNumber>(),
            Err(Error::truncated_payload(4, 1).in_container(BoxType::compact(*b"sqn2")))
        );
    }

    #[test]
    fn a_count_the_quantity_forbids_is_reported_before_any_child_is_read() {
        let payload = b"\0\0\0\x09sqnc!\0\0\0\x09sqnc!";

        assert_eq!(
            collected(payload).take_zero_or_one::<SequenceNumberBox>(),
            Err(Error::duplicate_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_quantity_of_one_or_more_yields_the_children_in_the_order_they_came() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";

        assert_eq!(
            collected(payload)
                .take_one_or_more::<SequenceNumberBox>()
                .unwrap(),
            vec![SequenceNumberBox(7), SequenceNumberBox(9)]
        );
    }

    #[test]
    fn a_collection_says_whether_a_child_of_a_type_is_there_without_reading_it() {
        assert!(collected(b"\0\0\0\x09sqnc!").contains::<SequenceNumberBox>());
        assert!(!collected(b"\0\0\0\x08free").contains::<SequenceNumberBox>());
    }

    #[test]
    fn a_quantity_of_zero_or_one_variant_yields_the_child_stated_and_nothing_for_none() {
        assert_eq!(
            collected(b"\0\0\0\x0csqnc\0\0\0\x07").take_zero_or_one_variant::<SequenceNumber>(),
            Ok(Some(SequenceNumber::Primary(SequenceNumberBox(7))))
        );
        assert_eq!(
            ChildBoxes::new().take_zero_or_one_variant::<SequenceNumber>(),
            Ok(None)
        );
    }

    #[test]
    fn a_quantity_of_zero_or_one_variant_refuses_more_than_one() {
        let of_one_type = b"\0\0\0\x0csqn2\0\0\0\x07\0\0\0\x0csqn2\0\0\0\x09";
        let of_two_types = b"\0\0\0\x0csqn2\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";

        assert_eq!(
            collected(of_one_type).take_zero_or_one_variant::<SequenceNumber>(),
            Err(Error::duplicate_box(BoxType::compact(*b"sqn2")))
        );
        assert_eq!(
            collected(of_two_types).take_zero_or_one_variant::<SequenceNumber>(),
            Err(Error::duplicate_alternative_box(SequenceNumber::VARIANTS))
        );
    }

    #[test]
    fn a_quantity_of_zero_or_more_yields_the_children_in_the_order_they_came() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";

        assert_eq!(
            collected(payload)
                .take_zero_or_more::<SequenceNumberBox>()
                .unwrap(),
            vec![SequenceNumberBox(7), SequenceNumberBox(9)]
        );
    }

    #[test]
    fn a_quantity_of_zero_or_more_reports_the_first_child_that_does_not_read() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x09sqnc!\0\0\0\x0asqnc!!";

        assert_eq!(
            collected(payload).take_zero_or_more::<SequenceNumberBox>(),
            Err(Error::truncated_payload(4, 1).in_container(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_take_leaves_the_children_of_other_types_unread() {
        let payload = b"\0\0\0\x09sqn2!\0\0\0\x0csqnc\0\0\0\x07";
        let mut children = collected(payload);

        assert_eq!(
            children.take_zero_or_more::<SequenceNumberBox>(),
            Ok(vec![SequenceNumberBox(7)])
        );
        assert_eq!(
            OtherBoxes::from(children).as_slice(),
            [AnyBox::from_raw_bytes(
                BoxType::compact(*b"sqn2"),
                b"!".to_vec()
            )]
        );
    }

    #[test]
    fn a_quantity_of_zero_or_more_yields_nothing_for_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new()
                .take_zero_or_more::<SequenceNumberBox>()
                .unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn a_quantity_of_one_or_more_refuses_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new().take_one_or_more::<SequenceNumberBox>(),
            Err(Error::missing_mandatory_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_child_that_does_not_read_names_itself_on_the_path_of_the_failure() {
        let mut children = collected(b"\0\0\0\x09sqnc!");

        assert_eq!(
            children.take_exactly_one::<SequenceNumberBox>(),
            Err(Error::truncated_payload(4, 1).in_container(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn the_children_no_take_removed_are_kept_as_the_bytes_they_lie_as_in_the_order_they_came() {
        let payload = b"\0\0\0\x0cfreeAAAA\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x08skip";
        let mut children = collected(payload);

        assert_eq!(
            children.take_exactly_one::<SequenceNumberBox>(),
            Ok(SequenceNumberBox(7))
        );
        assert_eq!(
            OtherBoxes::from(children).as_slice(),
            [
                AnyBox::from_raw_bytes(BoxType::compact(*b"free"), b"AAAA".to_vec()),
                AnyBox::from_raw_bytes(BoxType::compact(*b"skip"), Vec::new()),
            ]
        );
    }
}
