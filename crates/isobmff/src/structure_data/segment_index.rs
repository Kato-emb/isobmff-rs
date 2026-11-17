//! [`subsegments`], the byte ranges and times a segment index declares of its subsegments

use core::slice;

use isobmff_boxes::{SegmentIndexBox, SegmentIndexReference};

use super::error::Error;

/// A subsegment, or a further segment index, as a [`SegmentIndexBox`] (`sidx`) references it: where its bytes start and when it is presented
///
/// ISO/IEC 14496-12 §8.14.3. `offset` is the file offset of the first byte
/// of the referenced item, such as a `moof`, and `earliest_presentation_time`
/// the earliest presentation time of the reference stream in it, in the
/// `timescale` of the box. The rest, the `referenced_size`, the
/// `subsegment_duration`, the `reference_type` and the stream access points,
/// is the `reference` as the box holds it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Subsegment<'a> {
    /// File offset of the first byte of the referenced item
    pub offset: u64,
    /// Earliest presentation time of the reference stream in the item
    pub earliest_presentation_time: u64,
    /// The reference to the item, as the box holds it
    pub reference: &'a SegmentIndexReference,
}

/// The items a [`SegmentIndexBox`] references, in its order
///
/// Returned by [`subsegments`].
#[derive(Debug, Clone)]
pub struct Subsegments<'a> {
    references: slice::Iter<'a, SegmentIndexReference>,
    /// File offset of the next item
    offset: u64,
    /// Earliest presentation time of the next item
    earliest_presentation_time: u64,
}

/// Returns the items `sidx` references, each with the file offset of its first byte and its earliest presentation time, where `anchor` is the file offset of the first byte after the box
///
/// ISO/IEC 14496-12 §8.14.3.1: the material starts `first_offset` bytes
/// from the anchor point, each item `referenced_size` bytes after the one
/// before, and each subsegment `subsegment_duration` after the one before in
/// presentation time, from `earliest_presentation_time`. A reference to a
/// further segment index is returned as one to a subsegment: its bytes are
/// that box, which the caller reads where it wants the subsegments it
/// documents.
///
/// ```
/// use isobmff::boxes::{SegmentIndexBox, SegmentIndexReference};
/// use isobmff::structure_data::subsegments;
///
/// // Two subsegments of 1,000 and 600 bytes, 90,000 ticks each, after 8 bytes of free space
/// let reference = |referenced_size| {
///     let mut reference = SegmentIndexReference::default();
///     reference.referenced_size = referenced_size;
///     reference.subsegment_duration = 90_000;
///     reference
/// };
/// let mut sidx = SegmentIndexBox::default();
/// sidx.version = 1;
/// sidx.timescale = 90_000;
/// sidx.earliest_presentation_time = 180_000;
/// sidx.first_offset = 8;
/// sidx.references = vec![reference(1_000), reference(600)];
///
/// // The box ends at 2,000
/// let ranges: Vec<_> = subsegments(&sidx, 2_000)?
///     .map(|subsegment| (subsegment.offset, subsegment.earliest_presentation_time))
///     .collect();
/// assert_eq!(ranges, [(2_008, 180_000), (3_008, 270_000)]);
/// # Ok::<(), isobmff::structure_data::Error>(())
/// ```
///
/// # Errors
/// * [`Error::SegmentIndexOverflow`]: the items reach past the last offset a
///   file can have, or past the last presentation time 64 bits count.
pub fn subsegments(sidx: &SegmentIndexBox, anchor: u64) -> Result<Subsegments<'_>, Error> {
    let overflow = || Error::SegmentIndexOverflow {
        reference_id: sidx.reference_id,
    };
    let offset = anchor.checked_add(sidx.first_offset).ok_or_else(overflow)?;
    // Why check the sums up front: the iterator then adds without failing,
    // as each item starts no later than the end of the last.
    sidx.references
        .iter()
        .try_fold(
            (offset, sidx.earliest_presentation_time),
            |(end, time), reference| {
                Some((
                    end.checked_add(u64::from(reference.referenced_size))?,
                    time.checked_add(u64::from(reference.subsegment_duration))?,
                ))
            },
        )
        .ok_or_else(overflow)?;
    Ok(Subsegments {
        references: sidx.references.iter(),
        offset,
        earliest_presentation_time: sidx.earliest_presentation_time,
    })
}

impl<'a> Iterator for Subsegments<'a> {
    type Item = Subsegment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let reference = self.references.next()?;
        let subsegment = Subsegment {
            offset: self.offset,
            earliest_presentation_time: self.earliest_presentation_time,
            reference,
        };
        self.offset = self
            .offset
            .saturating_add(u64::from(reference.referenced_size));
        self.earliest_presentation_time = self
            .earliest_presentation_time
            .saturating_add(u64::from(reference.subsegment_duration));
        Some(subsegment)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.references.size_hint()
    }
}

impl ExactSizeIterator for Subsegments<'_> {}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::{SegmentIndexBox, SegmentIndexReference};

    use super::{Error, subsegments};

    fn reference(referenced_size: u32, subsegment_duration: u32) -> SegmentIndexReference {
        let mut reference = SegmentIndexReference::default();
        reference.referenced_size = referenced_size;
        reference.subsegment_duration = subsegment_duration;
        reference
    }

    #[test]
    fn items_follow_one_another_from_the_anchor_and_the_first_offset() {
        let mut sidx = SegmentIndexBox::default();
        sidx.first_offset = 28;
        sidx.earliest_presentation_time = 10;
        sidx.references = vec![reference(100, 5), reference(50, 7), reference(1, 0)];

        let items: Vec<_> = subsegments(&sidx, 72)
            .unwrap()
            .map(|item| {
                (
                    item.offset,
                    item.earliest_presentation_time,
                    *item.reference,
                )
            })
            .collect();

        assert_eq!(
            items,
            [
                (100, 10, reference(100, 5)),
                (200, 15, reference(50, 7)),
                (250, 22, reference(1, 0)),
            ]
        );
    }

    #[test]
    fn items_past_the_last_offset_are_refused() {
        let mut sidx = SegmentIndexBox::default();
        sidx.reference_id = 3;
        sidx.first_offset = u64::MAX - 10;
        sidx.references = vec![reference(5, 0), reference(5, 0)];

        assert_eq!(
            subsegments(&sidx, 1).unwrap_err(),
            Error::SegmentIndexOverflow { reference_id: 3 }
        );
    }

    #[test]
    fn items_past_the_last_presentation_time_are_refused() {
        let mut sidx = SegmentIndexBox::default();
        sidx.reference_id = 3;
        sidx.earliest_presentation_time = u64::MAX;
        sidx.references = vec![reference(5, 1)];

        assert_eq!(
            subsegments(&sidx, 0).unwrap_err(),
            Error::SegmentIndexOverflow { reference_id: 3 }
        );
    }
}
