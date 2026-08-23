//! [`ChildBoxes`] and [`OtherBoxes`], the boxes a container box of ISO/IEC 14496-12 §4.2 holds

use alloc::vec::Vec;

use crate::any_box::AnyBox;
use crate::codec::box_decode::BoxDecode;
use crate::codec::box_definition::BoxDefinition;
use crate::error::Error;
use crate::framing::box_type::BoxType;
use crate::framing::raw_box::RawBox;

/// Children of one box type, gathered as a container reads its payload
///
/// The box tables of ISO/IEC 14496-12 state, for every child a container may
/// hold, the quantity of it the container may hold: `Exactly one`, `Zero or
/// one`, `One or more`, `Zero or more`. A container gathers the children of one
/// type here and states that quantity once, by which method it finishes with —
/// [`exactly_one`](Self::exactly_one), [`zero_or_one`](Self::zero_or_one),
/// [`one_or_more`](Self::one_or_more), [`zero_or_more`](Self::zero_or_more).
/// Each hands back the field the quantity calls for and reports the counts the
/// quantity forbids.
///
/// `Exactly one variant must be present` states a quantity across box types:
/// §8.7.3.1 writes the sample sizes as either a `stsz` or a `stz2`, and §8.7.5.1
/// the chunk offsets as either a `stco` or a `co64` — one slot, two ways of
/// writing the one table it holds. The gathering of the variant this container
/// reads finishes with
/// [`exactly_one_variant`](Self::exactly_one_variant), which names the whole
/// slot in what it reports.
///
/// Reading is left until then. The children are gathered as the bytes they were
/// framed as, so a count the quantity already forbids is reported without a
/// payload being read at all.
///
/// Routing a box type to the gathering that claims it belongs to the container.
/// A child of another type pushed here is read as the type the finish asks for,
/// which is a fault of the container rather than one this type reports.
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
///     fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
///         Ok(Self {
///             sequence_number: reader.read_u32()?,
///         })
///     }
/// }
///
/// // The payload of a container: the box a field claims, then one no field does
/// let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x08free";
///
/// // Reading it sorts the children by the type that names them
/// let mut sequence_number_boxes = ChildBoxes::new();
/// let mut other_boxes = OtherBoxes::new();
/// for child in boxes(payload) {
///     let child = child.unwrap();
///     if child.header().box_type() == SequenceNumberBox::BOX_TYPE {
///         sequence_number_boxes.push(child);
///     } else {
///         other_boxes.keep(child);
///     }
/// }
///
/// // The quantity the box table states is asked for once, as the field is built
/// let sequence_number: SequenceNumberBox =
///     sequence_number_boxes.exactly_one().unwrap();
/// assert_eq!(sequence_number, SequenceNumberBox { sequence_number: 7 });
/// assert_eq!(other_boxes.as_slice().len(), 1);
///
/// // A container holding none of a child it must hold does not read
/// assert_eq!(
///     ChildBoxes::new().exactly_one::<SequenceNumberBox>(),
///     Err(Error::missing_mandatory_box(SequenceNumberBox::BOX_TYPE))
/// );
/// ```
#[derive(Default, Debug)]
pub struct ChildBoxes<'payload> {
    children: Vec<RawBox<'payload>>,
}

impl<'payload> ChildBoxes<'payload> {
    /// Creates a gathering that has taken no child yet
    #[must_use]
    pub const fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    /// Takes one more child of the type this gathering claims
    pub fn push(&mut self, child: RawBox<'payload>) {
        self.children.push(child);
    }

    /// Returns whether no child of the type this gathering claims was taken
    ///
    /// A container settling a rule on whether a child is there at all asks this
    /// before it states the quantity, which is what leaves the payload unread.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Returns the one child of a quantity of `Exactly one`
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox): no child of
    ///   the type was gathered.
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): more than one was.
    /// * Whatever the child reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn exactly_one<Child>(self) -> Result<Child, Error>
    where
        Child: BoxDecode + BoxDefinition,
    {
        self.zero_or_one::<Child>()?
            .ok_or(Error::missing_mandatory_box(Child::BOX_TYPE))
    }

    /// Returns the one child of a quantity of `Exactly one variant`
    ///
    /// `variants` is every box type that writes the slot, which the failure
    /// names; this gathering holds the variant the container reads.
    ///
    /// # Errors
    ///
    /// * [`MissingAlternativeBox`](crate::ErrorKind::MissingAlternativeBox): no child
    ///   of the type was gathered.
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): more than one was.
    /// * Whatever the child reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn exactly_one_variant<Child>(self, variants: &'static [BoxType]) -> Result<Child, Error>
    where
        Child: BoxDecode + BoxDefinition,
    {
        self.zero_or_one::<Child>()?
            .ok_or(Error::missing_alternative_box(variants))
    }

    /// Returns the child of a quantity of `Zero or one`, if it was there
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): more than one child of
    ///   the type was gathered.
    /// * Whatever the child reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn zero_or_one<Child>(self) -> Result<Option<Child>, Error>
    where
        Child: BoxDecode + BoxDefinition,
    {
        let mut children = self.children.into_iter();
        let Some(child) = children.next() else {
            return Ok(None);
        };
        if children.next().is_some() {
            return Err(Error::duplicate_box(Child::BOX_TYPE));
        }

        Ok(Some(decode::<Child>(child)?))
    }

    /// Returns the children of a quantity of `One or more`, in the order they came
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox): no child of
    ///   the type was gathered.
    /// * Whatever one of the children reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn one_or_more<Child>(self) -> Result<Vec<Child>, Error>
    where
        Child: BoxDecode + BoxDefinition,
    {
        if self.children.is_empty() {
            return Err(Error::missing_mandatory_box(Child::BOX_TYPE));
        }

        self.zero_or_more()
    }

    /// Returns the children of a quantity of `Zero or more`, in the order they came
    ///
    /// # Errors
    ///
    /// * Whatever one of the children reports, with its box type on the
    ///   [`containers`](Error::containers) path of the failure.
    pub fn zero_or_more<Child>(self) -> Result<Vec<Child>, Error>
    where
        Child: BoxDecode + BoxDefinition,
    {
        self.children.into_iter().map(decode::<Child>).collect()
    }
}

/// Reads one child, naming it in whatever failure it reports
fn decode<Child>(child: RawBox<'_>) -> Result<Child, Error>
where
    Child: BoxDecode + BoxDefinition,
{
    Child::decode_payload(child.payload()).map_err(|error| error.in_container(Child::BOX_TYPE))
}

/// Children of a container that no field of it claims
///
/// They are kept as the bytes they lie as, under the box type that names them,
/// so a container writes back the children it has no field to read them into.
/// The order they came in is the order they are held.
///
/// Where they go among the children the container does read is that container's
/// own canonical order, so summing them and writing them is that container's
/// part: this type hands them over with [`as_slice`](Self::as_slice), and the
/// container treats that run as it treats any run of children it holds.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, OtherBoxes, boxes};
///
/// // Two children of a container that has no field for either
/// let payload = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
///
/// let mut other_boxes = OtherBoxes::new();
/// for child in boxes(payload) {
///     other_boxes.keep(child.unwrap());
/// }
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
    /// Creates a holding that has kept no child yet
    #[must_use]
    pub const fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    /// Keeps a child no field of the container claims
    ///
    /// The payload is copied out of `child`, so what is kept outlives the input
    /// it was framed from.
    pub fn keep(&mut self, child: RawBox<'_>) {
        self.children.push(AnyBox::from_raw_bytes(
            child.header().box_type(),
            child.payload().to_vec(),
        ));
    }

    /// Returns the children kept, in the order they came
    #[must_use]
    pub fn as_slice(&self) -> &[AnyBox] {
        &self.children
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
    use crate::codec::field::FieldReader;
    use crate::error::Error;
    use crate::framing::box_type::BoxType;
    use crate::framing::raw_box::boxes;

    /// Box whose payload is one 32-bit sequence number
    #[derive(PartialEq, Debug)]
    struct SequenceNumberBox(u32);

    impl BoxDefinition for SequenceNumberBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"sqnc");
    }

    impl BoxDecode for SequenceNumberBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self(reader.read_u32()?))
        }
    }

    /// Gathers every box a payload holds, whatever type names it
    fn gathered(payload: &[u8]) -> ChildBoxes<'_> {
        let mut children = ChildBoxes::new();
        for child in boxes(payload) {
            children.push(child.unwrap());
        }

        children
    }

    #[test]
    fn a_quantity_of_exactly_one_yields_the_child_it_gathered() {
        let children = gathered(b"\0\0\0\x0csqnc\0\0\0\x07");

        assert_eq!(
            children.exactly_one::<SequenceNumberBox>().unwrap(),
            SequenceNumberBox(7)
        );
    }

    #[test]
    fn a_quantity_of_exactly_one_refuses_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new().exactly_one::<SequenceNumberBox>(),
            Err(Error::missing_mandatory_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_quantity_of_zero_or_one_yields_nothing_for_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new()
                .zero_or_one::<SequenceNumberBox>()
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_quantity_of_at_most_one_refuses_a_second_child() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";

        assert_eq!(
            gathered(payload).exactly_one::<SequenceNumberBox>(),
            Err(Error::duplicate_box(SequenceNumberBox::BOX_TYPE))
        );
        assert_eq!(
            gathered(payload).zero_or_one::<SequenceNumberBox>(),
            Err(Error::duplicate_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_count_the_quantity_forbids_is_reported_before_any_child_is_read() {
        let payload = b"\0\0\0\x09sqnc!\0\0\0\x09sqnc!";

        assert_eq!(
            gathered(payload).zero_or_one::<SequenceNumberBox>(),
            Err(Error::duplicate_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_quantity_of_one_or_more_yields_the_children_in_the_order_they_came() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";

        assert_eq!(
            gathered(payload)
                .one_or_more::<SequenceNumberBox>()
                .unwrap(),
            vec![SequenceNumberBox(7), SequenceNumberBox(9)]
        );
    }

    #[test]
    fn a_gathering_reports_whether_it_took_a_child_before_any_quantity_is_stated() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07";

        assert!(ChildBoxes::new().is_empty());
        assert!(!gathered(payload).is_empty());
    }

    #[test]
    fn a_quantity_of_zero_or_more_yields_the_children_in_the_order_they_came() {
        let payload = b"\0\0\0\x0csqnc\0\0\0\x07\0\0\0\x0csqnc\0\0\0\x09";

        assert_eq!(
            gathered(payload)
                .zero_or_more::<SequenceNumberBox>()
                .unwrap(),
            vec![SequenceNumberBox(7), SequenceNumberBox(9)]
        );
    }

    #[test]
    fn a_quantity_of_zero_or_more_yields_nothing_for_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new()
                .zero_or_more::<SequenceNumberBox>()
                .unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn a_quantity_of_one_or_more_refuses_a_container_holding_none() {
        assert_eq!(
            ChildBoxes::new().one_or_more::<SequenceNumberBox>(),
            Err(Error::missing_mandatory_box(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_child_that_does_not_read_names_itself_on_the_path_of_the_failure() {
        let children = gathered(b"\0\0\0\x09sqnc!");

        assert_eq!(
            children.exactly_one::<SequenceNumberBox>(),
            Err(Error::truncated_payload(4, 1).in_container(SequenceNumberBox::BOX_TYPE))
        );
    }

    #[test]
    fn children_no_field_claims_are_kept_as_the_bytes_they_lie_as() {
        let payload = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
        let mut other_boxes = OtherBoxes::new();
        for child in boxes(payload) {
            other_boxes.keep(child.unwrap());
        }

        assert_eq!(
            other_boxes.as_slice(),
            [
                AnyBox::from_raw_bytes(BoxType::compact(*b"free"), b"AAAA".to_vec()),
                AnyBox::from_raw_bytes(BoxType::compact(*b"skip"), Vec::new()),
            ]
        );
    }
}
