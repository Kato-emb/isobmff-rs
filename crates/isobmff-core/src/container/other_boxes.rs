//! [`OtherBoxes`], the children a container holds no field for, and [`ChildBoxWriter`], which writes them back among the others

use alloc::vec::Vec;
use core::mem;

use super::ChildBoxes;
use crate::any_box::AnyBox;
use crate::codec::box_definition::BoxDefinition;
use crate::codec::box_encode::BoxEncode;
use crate::codec::field::byte_count;
use crate::error::Error;
use crate::framing::box_type::BoxType;

/// Where a kept child lies among the known children of its container
///
/// The order of the variants is the order the writer meets them in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Placement {
    /// Lay before every known child
    Leading,
    /// Lay right after the known child of `box_type` that `occurrence` known children of that type preceded
    Following {
        box_type: BoxType,
        occurrence: usize,
    },
    /// Added after the container was read
    Added,
}

/// Returns how many known children of `box_type` `counts` has met, and counts one more
///
/// The known children of one type are told apart by this count, both where
/// they were read and where they are written.
fn next_occurrence(counts: &mut Vec<(BoxType, usize)>, box_type: BoxType) -> usize {
    if let Some((_, count)) = counts.iter_mut().find(|(counted, _)| *counted == box_type) {
        return mem::replace(count, count.saturating_add(1));
    }
    counts.push((box_type, 1));

    0
}

/// Children of a container that no field of it claims, and those a field claims that did not read
///
/// They are kept as the bytes they lie as, under the box type that names them,
/// so a container writes back the children it holds no value of. They are what
/// no take removed from the container's [`ChildBoxes`], in the order they came,
/// and [`as_slice`](Self::as_slice) hands them over in that order.
///
/// Each child read remembers the known child it followed: the child a take
/// removed that lay last before it. [`ChildBoxWriter`] writes it right after
/// that known child, so a container that writes its known children in the
/// order they lay writes its payload back as it was read. A child that lay
/// before every known child is written before them, and a child added with
/// [`push`](Self::push) after them; [`ChildBoxWriter`] states the bounds
/// ISO/IEC 14496-12 §7.3.4 sets on both.
///
/// Two holdings are equal when they keep the same children in the same order,
/// whichever known child each followed.
///
/// # Examples
///
/// ```
/// use isobmff_core::{BoxType, ChildBoxes, OtherBoxes};
///
/// // Two children of a container that has no field for either
/// let payload = b"\0\0\0\x0cfreeAAAA\0\0\0\x08skip";
/// let other_boxes = OtherBoxes::from(ChildBoxes::decode(payload).unwrap());
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
/// ```
#[derive(Clone, Default, Debug)]
pub struct OtherBoxes {
    children: Vec<AnyBox>,
    placements: Vec<Placement>,
}

impl OtherBoxes {
    /// Creates a holding of no child
    #[must_use]
    pub const fn new() -> Self {
        Self {
            children: Vec::new(),
            placements: Vec::new(),
        }
    }

    /// Returns the children kept, in the order they came
    #[must_use]
    pub fn as_slice(&self) -> &[AnyBox] {
        &self.children
    }

    /// Adds a child, which [`ChildBoxWriter`] writes after the known children and before the child given to [`write_last`](ChildBoxWriter::write_last)
    pub fn push(&mut self, child: AnyBox) {
        self.children.push(child);
        self.placements.push(Placement::Added);
    }
}

impl PartialEq for OtherBoxes {
    fn eq(&self, other: &Self) -> bool {
        self.children == other.children
    }
}

impl From<ChildBoxes<'_>> for OtherBoxes {
    /// Keeps the children no take removed, copying each payload out of the input it was framed from
    ///
    /// Each child remembers the known child it followed, among the children
    /// the takes removed.
    fn from(children: ChildBoxes<'_>) -> Self {
        let ChildBoxes {
            children,
            mut known,
        } = children;
        known.sort_unstable_by_key(|(position, _)| *position);
        let mut preceding = known.into_iter().peekable();
        let mut counts = Vec::new();
        let mut placement = Placement::Leading;
        let (children, placements) = children
            .into_iter()
            .map(|(position, child)| {
                // Why not a search per child: the kept children and the known
                // ones both run in the order they lay, so one pass pairs them.
                while let Some((_, box_type)) = preceding.next_if(|(known, _)| *known < position) {
                    placement = Placement::Following {
                        box_type,
                        occurrence: next_occurrence(&mut counts, box_type),
                    };
                }
                let kept =
                    AnyBox::from_raw_bytes(child.header().box_type(), child.payload().to_vec());

                (kept, placement)
            })
            .unzip();

        Self {
            children,
            placements,
        }
    }
}

/// Writer of the children of a container, which puts each child of its [`OtherBoxes`] back after the known child it followed
///
/// A container writes the children it holds a field for — its known
/// children — through this, one by one in its own order, and ends with
/// [`write_last`](Self::write_last) or [`finish`](Self::finish), which write
/// the kept children still unwritten. After each known child, the writer
/// writes the kept children that followed the same known child when the
/// container was read. Known children of one box type are told apart by the order they are
/// written in, so a field holding several children of one type writes them
/// in the order they were read.
///
/// ISO/IEC 14496-12 §7.3.4 bounds where a child may go, and the writer keeps
/// to both bounds for the kept children:
///
/// * Rule 2: "It is strongly recommended that all header boxes be placed
///   first in their container". A container writes its header box with
///   [`write_header`](Self::write_header), as the first child it writes, and
///   no kept child is written before it: a kept child that lay before the
///   header box is written right after it.
/// * Rule 6: "It is recommended that `UserDataBoxes` be placed last in their
///   container". A container writes such a child with
///   [`write_last`](Self::write_last), and no kept child is written after it:
///   a kept child that lay after it is written right before it.
///
/// A kept child that lay before every known child of a container with no
/// header box is written first. A kept child whose known child the container
/// does not write, and a child added with [`OtherBoxes::push`], are written
/// before the child given to [`write_last`](Self::write_last), or last where
/// there is none, in the order they are held.
///
/// The writer allocates a flag and an index for each kept child.
///
/// # Examples
///
/// ```
/// use isobmff_core::{
///     BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxWriter, ChildBoxes, Error,
///     FieldReader, FieldWriter, OtherBoxes,
/// };
///
/// // A header box whose payload is one 16-bit level
/// #[derive(PartialEq, Debug)]
/// struct LevelBox {
///     level: u16,
/// }
///
/// impl BoxDefinition for LevelBox {
///     const BOX_TYPE: BoxType = BoxType::compact(*b"vlvl");
/// }
///
/// impl BoxDecode for LevelBox {
///     fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
///         Ok(Self { level: reader.read_u16()? })
///     }
/// }
///
/// impl BoxEncode for LevelBox {
///     fn payload_len(&self) -> u64 {
///         2
///     }
///
///     fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
///         writer.write_u16(self.level)
///     }
/// }
///
/// // A container payload whose header box lies after a child no field claims
/// let payload = b"\0\0\0\x08free\0\0\0\x0avlvl\0\x07";
/// let mut children = ChildBoxes::decode(payload).unwrap();
/// let level: LevelBox = children.take_exactly_one().unwrap();
/// let other_boxes = OtherBoxes::from(children);
///
/// // Written back, the header box comes first and the kept child right after it
/// let mut buffer = [0; 18];
/// let mut writer = ChildBoxWriter::new(&mut buffer, &other_boxes);
/// writer.write_header(&level).unwrap();
/// writer.finish().unwrap();
/// assert_eq!(&buffer, b"\0\0\0\x0avlvl\0\x07\0\0\0\x08free");
/// ```
#[must_use = "a writer leaves kept children unwritten until write_last or finish"]
#[derive(Debug)]
pub struct ChildBoxWriter<'other_boxes, 'buffer> {
    rest: &'buffer mut [u8],
    buffer_len: usize,
    other_boxes: &'other_boxes OtherBoxes,
    by_placement: Vec<usize>,
    written: Vec<bool>,
    known_written: Vec<(BoxType, usize)>,
}

impl<'other_boxes, 'buffer> ChildBoxWriter<'other_boxes, 'buffer> {
    /// Creates a writer of the children of a container into `buffer`, the room its children take
    pub fn new(buffer: &'buffer mut [u8], other_boxes: &'other_boxes OtherBoxes) -> Self {
        // Why not scan the kept children for each known child written: that
        // takes time in the product of the two counts, which the input sets.
        let mut by_placement: Vec<usize> = (0..other_boxes.placements.len()).collect();
        by_placement.sort_by_key(|kept| other_boxes.placements.get(*kept).copied());

        Self {
            buffer_len: buffer.len(),
            rest: buffer,
            other_boxes,
            by_placement,
            written: alloc::vec![false; other_boxes.children.len()],
            known_written: Vec::new(),
        }
    }

    /// Writes the header box of the container, the first child it writes, and then the kept children that lay before it or followed it
    ///
    /// # Errors
    ///
    /// * What writing `child` or a kept child reports, such as
    ///   [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer).
    pub fn write_header<Child: BoxEncode + BoxDefinition>(
        &mut self,
        child: &Child,
    ) -> Result<(), Error> {
        let following = self.write_known(child)?;
        self.write_placed(Placement::Leading)?;
        self.write_placed(following)
    }

    /// Writes a known child, and then the kept children that followed it
    ///
    /// Before the first known child, the writer writes the kept children that
    /// lay before every known child.
    ///
    /// # Errors
    ///
    /// * What writing `child` or a kept child reports, such as
    ///   [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer).
    pub fn write<Child: BoxEncode + BoxDefinition>(&mut self, child: &Child) -> Result<(), Error> {
        self.write_placed(Placement::Leading)?;
        let following = self.write_known(child)?;
        self.write_placed(following)
    }

    /// Writes every kept child not yet written, and then the last child of the container, which ends the writing
    ///
    /// # Errors
    ///
    /// * What writing `child` or a kept child reports, such as
    ///   [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer).
    /// * [`TrailingBuffer`](crate::ErrorKind::TrailingBuffer): the buffer holds
    ///   bytes past the children written.
    pub fn write_last<Child: BoxEncode + BoxDefinition>(
        mut self,
        child: &Child,
    ) -> Result<(), Error> {
        self.write_unwritten()?;
        self.rest = child.encode(mem::take(&mut self.rest))?;
        self.finish()
    }

    /// Writes every kept child not yet written, which ends the writing of a container with no last child
    ///
    /// # Errors
    ///
    /// * What writing a kept child reports, such as
    ///   [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer).
    /// * [`TrailingBuffer`](crate::ErrorKind::TrailingBuffer): the buffer holds
    ///   bytes past the children written.
    pub fn finish(mut self) -> Result<(), Error> {
        self.write_unwritten()?;
        if self.rest.is_empty() {
            return Ok(());
        }

        Err(Error::trailing_buffer(
            byte_count(self.buffer_len.saturating_sub(self.rest.len())),
            byte_count(self.buffer_len),
        ))
    }

    /// Writes a known child, and returns the placement of the kept children that followed it
    fn write_known<Child: BoxEncode + BoxDefinition>(
        &mut self,
        child: &Child,
    ) -> Result<Placement, Error> {
        self.rest = child.encode(mem::take(&mut self.rest))?;

        Ok(Placement::Following {
            box_type: Child::BOX_TYPE,
            occurrence: next_occurrence(&mut self.known_written, Child::BOX_TYPE),
        })
    }

    /// Writes the kept children of `placement` not yet written, in the order they are held
    fn write_placed(&mut self, placement: Placement) -> Result<(), Error> {
        let placements = &self.other_boxes.placements;
        let placed_at = |kept: &usize| placements.get(*kept).copied();
        let start = self
            .by_placement
            .partition_point(|kept| placed_at(kept) < Some(placement));
        let end = self
            .by_placement
            .partition_point(|kept| placed_at(kept) <= Some(placement));
        for kept in self.by_placement.get(start..end).unwrap_or_default() {
            self.rest = write_kept(
                self.other_boxes,
                &mut self.written,
                *kept,
                mem::take(&mut self.rest),
            )?;
        }

        Ok(())
    }

    /// Writes every kept child not yet written, in the order they are held
    fn write_unwritten(&mut self) -> Result<(), Error> {
        for kept in 0..self.other_boxes.children.len() {
            self.rest = write_kept(
                self.other_boxes,
                &mut self.written,
                kept,
                mem::take(&mut self.rest),
            )?;
        }

        Ok(())
    }
}

/// Writes the kept child at `kept` into the front of `buffer` unless it is written already, and returns what is left
fn write_kept<'buffer>(
    other_boxes: &OtherBoxes,
    written: &mut [bool],
    kept: usize,
    buffer: &'buffer mut [u8],
) -> Result<&'buffer mut [u8], Error> {
    match (other_boxes.children.get(kept), written.get_mut(kept)) {
        (Some(child), Some(done)) if !*done => {
            *done = true;
            child.encode(buffer)
        }
        _ => Ok(buffer),
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{ChildBoxWriter, OtherBoxes};
    use crate::any_box::AnyBox;
    use crate::codec::box_decode::BoxDecode;
    use crate::codec::box_definition::BoxDefinition;
    use crate::codec::box_encode::BoxEncode;
    use crate::codec::field::{FieldReader, FieldWriter};
    use crate::container::ChildBoxes;
    use crate::error::Error;
    use crate::framing::box_type::BoxType;

    /// Box whose payload is one byte, under the box type `Type` names
    #[derive(PartialEq, Debug)]
    struct ByteBox<const TYPE: u32>(u8);

    impl<const TYPE: u32> BoxDefinition for ByteBox<TYPE> {
        const BOX_TYPE: BoxType = BoxType::compact(TYPE.to_be_bytes());
    }

    impl<const TYPE: u32> BoxDecode for ByteBox<TYPE> {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self(*reader.read_bytes::<1>()?.first().unwrap_or(&0)))
        }
    }

    impl<const TYPE: u32> BoxEncode for ByteBox<TYPE> {
        fn payload_len(&self) -> u64 {
            1
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            writer.write_bytes(&[self.0])
        }
    }

    /// Header box of the container these tests read and write
    type HeaderBox = ByteBox<{ u32::from_be_bytes(*b"head") }>;

    /// Known child of which the container holds any number
    type ItemBox = ByteBox<{ u32::from_be_bytes(*b"item") }>;

    /// Known child the container writes last, as a `UserDataBox`
    type TrailerBox = ByteBox<{ u32::from_be_bytes(*b"tail") }>;

    const HEAD: &[u8] = b"\0\0\0\x09head\x01";
    const ITEM_A: &[u8] = b"\0\0\0\x09item\x0a";
    const ITEM_B: &[u8] = b"\0\0\0\x09item\x0b";
    const TAIL: &[u8] = b"\0\0\0\x09tail\x02";
    const FREE: &[u8] = b"\0\0\0\x08free";
    const SKIP: &[u8] = b"\0\0\0\x08skip";
    const UUID: &[u8] =
        b"\0\0\0\x18uuid\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10";

    /// The `free` of [`FREE`], as a child added to a container
    fn free() -> AnyBox {
        AnyBox::from_raw_bytes(BoxType::compact(*b"free"), Vec::new())
    }

    /// Children read from a container payload: its header box, its items, its trailer if any, and what no field claims
    struct Read {
        head: Option<HeaderBox>,
        items: Vec<ItemBox>,
        tail: Option<TrailerBox>,
        other_boxes: OtherBoxes,
    }

    fn read(payload: &[u8]) -> Read {
        let mut children = ChildBoxes::decode(payload).unwrap();

        Read {
            head: children.take_zero_or_one().unwrap(),
            items: children.take_zero_or_more().unwrap(),
            tail: children.take_zero_or_one().unwrap(),
            other_boxes: OtherBoxes::from(children),
        }
    }

    /// Writes the children `read` holds — header box, items, then trailer — into the room they take
    fn written(read: &Read) -> Result<Vec<u8>, Error> {
        let len = read
            .head
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(read.items.iter().map(BoxEncode::encoded_len))
            .chain(read.tail.iter().map(BoxEncode::encoded_len))
            .chain(read.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add);
        let mut buffer = vec![0; usize::try_from(len).unwrap()];
        let mut writer = ChildBoxWriter::new(&mut buffer, &read.other_boxes);
        if let Some(head) = &read.head {
            writer.write_header(head)?;
        }
        for item in &read.items {
            writer.write(item)?;
        }
        match &read.tail {
            Some(tail) => writer.write_last(tail)?,
            None => writer.finish()?,
        }

        Ok(buffer)
    }

    #[test]
    fn children_no_field_claims_are_written_back_where_they_lay_among_the_known_children() {
        let payload = [FREE, ITEM_A, SKIP, ITEM_B, UUID].concat();

        assert_eq!(written(&read(&payload)), Ok(payload));
    }

    #[test]
    fn a_payload_that_keeps_to_the_header_and_the_last_child_is_written_back_as_it_was_read() {
        let payload = [HEAD, FREE, ITEM_A, SKIP, ITEM_B, UUID, TAIL].concat();

        assert_eq!(written(&read(&payload)), Ok(payload));
    }

    #[test]
    fn no_child_is_written_before_the_header_box_or_after_the_last_child() {
        let payload = [FREE, HEAD, ITEM_A, SKIP, TAIL, UUID].concat();

        assert_eq!(
            written(&read(&payload)),
            Ok([HEAD, FREE, ITEM_A, SKIP, UUID, TAIL].concat())
        );
    }

    #[test]
    fn a_child_no_field_claims_follows_the_known_child_it_followed_when_the_known_children_are_reordered()
     {
        let payload = [ITEM_A, FREE, HEAD, SKIP].concat();

        assert_eq!(
            written(&read(&payload)),
            Ok([HEAD, SKIP, ITEM_A, FREE].concat())
        );
    }

    #[test]
    fn a_child_whose_known_child_is_not_written_goes_before_the_last_child() {
        let mut without_item = read(&[HEAD, ITEM_A, FREE, TAIL].concat());
        without_item.items.clear();

        assert_eq!(written(&without_item), Ok([HEAD, FREE, TAIL].concat()));
    }

    #[test]
    fn an_added_child_goes_after_the_known_children_and_before_the_last_child() {
        let mut read = read(&[HEAD, ITEM_A, TAIL].concat());
        read.other_boxes.push(free());

        assert_eq!(written(&read), Ok([HEAD, ITEM_A, FREE, TAIL].concat()));
    }

    #[test]
    fn an_added_child_goes_last_in_a_container_with_no_last_child() {
        let mut read = read(&[HEAD, ITEM_A].concat());
        read.other_boxes.push(free());

        assert_eq!(written(&read), Ok([HEAD, ITEM_A, FREE].concat()));
    }

    #[test]
    fn finishing_with_room_left_past_the_children_is_refused() {
        let other_boxes = OtherBoxes::from(ChildBoxes::decode(FREE).unwrap());
        let mut buffer = [0; 10];
        let writer = ChildBoxWriter::new(&mut buffer, &other_boxes);

        assert_eq!(writer.finish(), Err(Error::trailing_buffer(8, 10)));
    }

    #[test]
    fn holdings_of_the_same_children_are_equal_whichever_known_child_each_followed() {
        let after_item = read(&[ITEM_A, FREE].concat()).other_boxes;
        let leading = read(&[FREE, ITEM_A].concat()).other_boxes;

        assert_eq!(after_item, leading);
    }
}
