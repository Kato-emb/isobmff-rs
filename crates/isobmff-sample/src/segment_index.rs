//! [`subsegments`], the subsegments a segment index points at resolved to the file, ISO/IEC 14496-12 §8.16.3

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{SegmentIndexBox, SegmentIndexReference};

use crate::error::Error;

/// Subsegments a `sidx` indexes, placed in the file and on the presentation timeline
///
/// The times are in the `timescale` of the index, which is the time scale of
/// the track it names in files based on ISO/IEC 14496-12, and are
/// presentation times: they count on from the `earliest_presentation_time`
/// the index states.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SegmentIndex {
    reference_id: u32,
    timescale: u32,
    earliest_presentation_time: u64,
    subsegments: Vec<Subsegment>,
}

impl SegmentIndex {
    /// Returns the ID of the stream the index names, a track ID in files based on ISO/IEC 14496-12
    #[must_use]
    pub const fn reference_id(&self) -> u32 {
        self.reference_id
    }

    /// Returns the units per second the times of the index count
    #[must_use]
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Returns the earliest presentation time of the first subsegment, in the time scale of the index
    #[must_use]
    pub const fn earliest_presentation_time(&self) -> u64 {
        self.earliest_presentation_time
    }

    /// Returns the subsegments, in the order they lie in the file
    #[must_use]
    pub fn subsegments(&self) -> &[Subsegment] {
        &self.subsegments
    }

    /// Returns the subsegment whose presentation `time` falls in, in the time scale of the index
    ///
    /// A subsegment covers the times from its earliest presentation time up
    /// to, and not including, that time on by its duration, so a subsegment
    /// lasting no time covers none. A reference to another `sidx` is returned
    /// as it is, for the caller to read that index in turn.
    #[must_use]
    pub fn subsegment_at(&self, time: u64) -> Option<&Subsegment> {
        let after = self
            .subsegments
            .partition_point(|subsegment| subsegment.earliest_presentation_time <= time);
        let candidate = self.subsegments.get(after.checked_sub(1)?)?;

        (time < candidate.presentation_end()).then_some(candidate)
    }
}

/// One subsegment of a [`SegmentIndex`], with the bytes it occupies and the time it starts at
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Subsegment {
    extent: Range<u64>,
    earliest_presentation_time: u64,
    reference: SegmentIndexReference,
}

impl Subsegment {
    /// Returns the bytes of the file the subsegment occupies
    #[must_use]
    pub fn extent(&self) -> Range<u64> {
        self.extent.clone()
    }

    /// Returns the earliest presentation time of the subsegment, in the time scale of its index
    #[must_use]
    pub const fn earliest_presentation_time(&self) -> u64 {
        self.earliest_presentation_time
    }

    /// Returns the reference of the `sidx` the subsegment was resolved from
    ///
    /// It states what the bytes hold, how long the subsegment lasts and where
    /// its first stream access point lies.
    #[must_use]
    pub const fn reference(&self) -> &SegmentIndexReference {
        &self.reference
    }

    /// Returns the presentation time the subsegment ends at
    fn presentation_end(&self) -> u64 {
        // Why not checked_add: subsegments summed these same times and refused
        // the index on overflow, so this cannot saturate.
        self.earliest_presentation_time
            .saturating_add(u64::from(self.reference.subsegment_duration()))
    }
}

/// Resolves the references of `sidx` to the subsegments they point at, counting from `anchor`
///
/// `anchor` is where the file continues past the `sidx` — the first byte
/// after the box in the file holding it (ISO/IEC 14496-12 §8.16.3.3). The
/// first subsegment starts `first_offset` bytes on from it, each after it
/// where the one before ends, and each starts on the presentation timeline
/// where the one before ends.
///
/// # Errors
///
/// * [`DataOffsetOverflow`](crate::ErrorKind::DataOffsetOverflow): the
///   extents of the subsegments run past what 64 bits carry.
/// * [`PresentationTimeOverflow`](crate::ErrorKind::PresentationTimeOverflow):
///   the times of the subsegments run past what 64 bits carry.
///
/// Both name the stream the index names by its `reference_ID`.
pub fn subsegments(sidx: &SegmentIndexBox, anchor: u64) -> Result<SegmentIndex, Error> {
    let reference_id = sidx.reference_id();
    let offset_overflow = || Error::data_offset_overflow(reference_id);
    let time_overflow = || Error::presentation_time_overflow(reference_id);

    let mut start = anchor
        .checked_add(sidx.first_offset())
        .ok_or_else(offset_overflow)?;
    let mut earliest_presentation_time = sidx.earliest_presentation_time();
    let mut subsegments = Vec::with_capacity(sidx.references().len());
    for reference in sidx.references() {
        let end = start
            .checked_add(u64::from(reference.referenced_size()))
            .ok_or_else(offset_overflow)?;
        let presentation_end = earliest_presentation_time
            .checked_add(u64::from(reference.subsegment_duration()))
            .ok_or_else(time_overflow)?;

        subsegments.push(Subsegment {
            extent: start..end,
            earliest_presentation_time,
            reference: *reference,
        });

        start = end;
        earliest_presentation_time = presentation_end;
    }

    Ok(SegmentIndex {
        reference_id,
        timescale: sidx.timescale(),
        earliest_presentation_time: sidx.earliest_presentation_time(),
        subsegments,
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::{ReferenceType, SegmentIndexBox, SegmentIndexReference};

    use super::{SegmentIndex, Subsegment, subsegments};
    use crate::error::Error;

    /// Reference to a media subsegment of `referenced_size` bytes lasting `subsegment_duration`
    fn reference(referenced_size: u32, subsegment_duration: u32) -> SegmentIndexReference {
        SegmentIndexReference::new(
            ReferenceType::MediaContent,
            referenced_size,
            subsegment_duration,
            true,
            1,
            0,
        )
        .unwrap()
    }

    /// Index of track 1 at 90 kHz starting at `earliest_presentation_time`, `first_offset` past its anchor
    fn index(
        earliest_presentation_time: u64,
        first_offset: u64,
        references: Vec<SegmentIndexReference>,
    ) -> SegmentIndexBox {
        SegmentIndexBox::new(
            1,
            90_000,
            earliest_presentation_time,
            first_offset,
            references,
        )
        .unwrap()
    }

    /// Subsegment over `extent`, starting at `earliest_presentation_time`, resolved from `reference`
    fn subsegment(
        extent: Range<u64>,
        earliest_presentation_time: u64,
        reference: SegmentIndexReference,
    ) -> Subsegment {
        Subsegment {
            extent,
            earliest_presentation_time,
            reference,
        }
    }

    /// Index of three subsegments of 1000, 2000 and 500 bytes, lasting 3000, 3000 and 1500, from 200 past an anchor at 1000
    fn three_subsegments() -> SegmentIndex {
        subsegments(
            &index(
                9_000,
                200,
                vec![
                    reference(1_000, 3_000),
                    reference(2_000, 3_000),
                    reference(500, 1_500),
                ],
            ),
            1_000,
        )
        .unwrap()
    }

    #[test]
    fn each_subsegment_starts_where_the_one_before_ends_in_bytes_and_in_time() {
        assert_eq!(
            three_subsegments(),
            SegmentIndex {
                reference_id: 1,
                timescale: 90_000,
                earliest_presentation_time: 9_000,
                subsegments: vec![
                    subsegment(1_200..2_200, 9_000, reference(1_000, 3_000)),
                    subsegment(2_200..4_200, 12_000, reference(2_000, 3_000)),
                    subsegment(4_200..4_700, 15_000, reference(500, 1_500)),
                ],
            }
        );
    }

    #[test]
    fn a_time_is_looked_up_in_the_subsegment_it_falls_in() {
        let resolved = three_subsegments();
        let starting_at = |time: u64| {
            resolved
                .subsegment_at(time)
                .map(Subsegment::earliest_presentation_time)
        };

        assert_eq!(starting_at(9_000), Some(9_000));
        assert_eq!(starting_at(11_999), Some(9_000));
        assert_eq!(starting_at(12_000), Some(12_000));
        assert_eq!(starting_at(16_499), Some(15_000));
    }

    #[test]
    fn a_time_before_the_first_subsegment_or_past_the_last_falls_in_none() {
        let resolved = three_subsegments();

        assert_eq!(resolved.subsegment_at(8_999), None);
        assert_eq!(resolved.subsegment_at(16_500), None);
        assert_eq!(
            subsegments(&index(0, 0, Vec::new()), 0)
                .unwrap()
                .subsegment_at(0),
            None
        );
    }

    #[test]
    fn extents_running_past_what_64_bits_carry_are_refused() {
        assert_eq!(
            subsegments(&index(0, u64::MAX, Vec::new()), 1),
            Err(Error::data_offset_overflow(1))
        );
        assert_eq!(
            subsegments(&index(0, u64::MAX - 10, vec![reference(11, 0)]), 0),
            Err(Error::data_offset_overflow(1))
        );
    }

    #[test]
    fn times_running_past_what_64_bits_carry_are_refused() {
        assert_eq!(
            subsegments(&index(u64::MAX - 10, 0, vec![reference(1, 11)]), 0),
            Err(Error::presentation_time_overflow(1))
        );
    }
}
