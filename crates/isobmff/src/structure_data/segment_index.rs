//! [`SegmentIndex`] and [`Subsegment`], a `sidx` resolved to the byte ranges and the times of the subsegments it indexes

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{MovieFragmentRandomAccessOffsetBox, SegmentIndexBox};
use isobmff_core::BoxDecode;

use crate::error::DemuxError;

/// Segment index: how a (sub)segment divides into subsegments, each with its byte range and its presentation times, ISO/IEC 14496-12 §8.14.3
///
/// The byte ranges are file offsets, counted from the anchor of the `sidx`,
/// the first byte after it. The times are in the timescale of the index. Two
/// segment indexes are the same only when their `sidx`es lie at the same
/// byte range.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SegmentIndex {
    byte_range: Range<u64>,
    reference_id: u32,
    timescale: u32,
    subsegments: Vec<Subsegment>,
}

impl SegmentIndex {
    /// Resolves `sidx`, which lies at the file offsets `byte_range`, its anchor point the end of it
    ///
    /// The first subsegment starts `first_offset` bytes past the anchor and at
    /// the `earliest_presentation_time`, and each next one where the one
    /// before it ends, `referenced_size` bytes and `subsegment_duration` ticks
    /// on.
    ///
    /// # Errors
    ///
    /// * [`SegmentIndexOutOfRange`](DemuxError::SegmentIndexOutOfRange): the
    ///   byte ranges or the times run past 64 bits.
    pub(super) fn new(sidx: &SegmentIndexBox, byte_range: Range<u64>) -> Result<Self, DemuxError> {
        let out_of_range = DemuxError::SegmentIndexOutOfRange {
            reference_id: sidx.reference_id,
        };
        let mut offset = byte_range
            .end
            .checked_add(sidx.first_offset)
            .ok_or(out_of_range)?;
        let mut time = sidx.earliest_presentation_time;
        let subsegments = sidx
            .references
            .iter()
            .map(|reference| {
                let end = offset
                    .checked_add(u64::from(reference.referenced_size))
                    .ok_or(out_of_range)?;
                let time_end = time
                    .checked_add(u64::from(reference.subsegment_duration))
                    .ok_or(out_of_range)?;
                let subsegment = Subsegment {
                    byte_range: offset..end,
                    presentation_time: time..time_end,
                    references_segment_index: reference.reference_type,
                    starts_with_sap: reference.starts_with_sap,
                    sap_type: reference.sap_type,
                    sap_delta_time: reference.sap_delta_time,
                };
                offset = end;
                time = time_end;
                Ok::<_, DemuxError>(subsegment)
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            byte_range,
            reference_id: sidx.reference_id,
            timescale: sidx.timescale,
            subsegments,
        })
    }

    /// Returns the file offsets the `sidx` lies at
    #[must_use]
    pub fn byte_range(&self) -> Range<u64> {
        self.byte_range.clone()
    }

    /// Returns the stream ID of the reference stream, its `track_ID` in a file of this format
    #[must_use]
    pub const fn reference_id(&self) -> u32 {
        self.reference_id
    }

    /// Returns the ticks per second of the presentation times of the subsegments
    #[must_use]
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Returns the subsegments, in the order of their presentation times
    #[must_use]
    pub fn subsegments(&self) -> &[Subsegment] {
        &self.subsegments
    }

    /// Returns the subsegment whose presentation times hold `time`, in the timescale of the index
    #[must_use]
    pub fn subsegment_at(&self, time: u64) -> Option<&Subsegment> {
        self.subsegments
            .iter()
            .find(|subsegment| subsegment.presentation_time.contains(&time))
    }
}

/// Subsegment a segment index references: its byte range, its presentation times and its stream access points, ISO/IEC 14496-12 §8.14.3
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Subsegment {
    byte_range: Range<u64>,
    presentation_time: Range<u64>,
    references_segment_index: bool,
    starts_with_sap: bool,
    sap_type: u8,
    sap_delta_time: u32,
}

impl Subsegment {
    /// Returns the file offsets of the subsegment, or of the segment index it references
    #[must_use]
    pub fn byte_range(&self) -> Range<u64> {
        self.byte_range.clone()
    }

    /// Returns the presentation times the subsegment spans, from its earliest, in the timescale of the index
    #[must_use]
    pub fn presentation_time(&self) -> Range<u64> {
        self.presentation_time.clone()
    }

    /// Returns whether the byte range holds a further segment index rather than media content, the `reference_type`
    #[must_use]
    pub const fn references_segment_index(&self) -> bool {
        self.references_segment_index
    }

    /// Returns whether the subsegment starts with a stream access point
    #[must_use]
    pub const fn starts_with_sap(&self) -> bool {
        self.starts_with_sap
    }

    /// Returns the type of the stream access points, 1 to 6 as Annex I specifies, or 0
    #[must_use]
    pub const fn sap_type(&self) -> u8 {
        self.sap_type
    }

    /// Returns the presentation time of the first stream access point from the earliest presentation time of the subsegment
    #[must_use]
    pub const fn sap_delta_time(&self) -> u32 {
        self.sap_delta_time
    }
}

/// Returns the file offset of the `mfra` the last 16 bytes of a file of `file_length` bytes point at, where they are an `mfro`, ISO/IEC 14496-12 §8.8.11
///
/// The `mfro` is the last box of the `mfra`, and repeats its size, so the
/// `mfra` starts that many bytes before the end of the file. The offset is a
/// hint: what is there may not be an `mfra` (§8.8.9).
///
/// `None` where the bytes are not an `mfro` of 16 bytes, or it gives a size
/// past the start of the file.
///
/// # Examples
///
/// ```
/// use isobmff::structure_data::movie_fragment_random_access_start;
///
/// // An mfro of an mfra of 61 bytes, at the end of a file of 1000
/// let last_bytes = *b"\0\0\0\x10mfro\0\0\0\0\0\0\0\x3d";
/// assert_eq!(movie_fragment_random_access_start(1000, &last_bytes), Some(939));
/// assert_eq!(movie_fragment_random_access_start(60, &last_bytes), None);
/// assert_eq!(movie_fragment_random_access_start(1000, &[0; 16]), None);
/// ```
#[must_use]
pub fn movie_fragment_random_access_start(file_length: u64, last_bytes: &[u8; 16]) -> Option<u64> {
    let (mfro, rest) = MovieFragmentRandomAccessOffsetBox::decode(last_bytes).ok()?;
    if !rest.is_empty() {
        return None;
    }

    file_length.checked_sub(u64::from(mfro.parent_size))
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{MovieFragmentRandomAccessBox, SegmentIndexBox};
    use isobmff_core::BoxDecode;

    use alloc::vec::Vec;
    use core::ops::Range;

    use super::{SegmentIndex, Subsegment, movie_fragment_random_access_start};
    use crate::error::DemuxError;
    use crate::structure_data::fixtures::UNLIMITED;
    use crate::structure_data::{StructureDataLimits, StructureDataReader};

    /// Returns a byte range of a `sidx` whose anchor point is at file offset `anchor`, which is all of it the index resolves from
    fn anchored_at(anchor: u64) -> Range<u64> {
        anchor.saturating_sub(32)..anchor
    }

    /// `sidx` of version 0 of stream 1 at 1000 ticks a second, from time 500 and 100 bytes past the anchor: a subsegment of 2000 bytes and 3000 ticks starting with a SAP of type 1, then a reference to a segment index of 50 bytes and 1000 ticks
    const SIDX_V0: &[u8] = &[
        0, 0, 0, 0, // version, flags
        0, 0, 0, 1, // reference_ID
        0, 0, 0x03, 0xe8, // timescale
        0, 0, 0x01, 0xf4, // earliest_presentation_time
        0, 0, 0, 100, // first_offset
        0, 0, 0, 2, // reserved, reference_count
        0, 0, 0x07, 0xd0, 0, 0, 0x0b, 0xb8, 0x90, 0, 0,
        0, // media, 2000 bytes, 3000 ticks, SAP type 1
        0x80, 0, 0, 50, 0, 0, 0x03, 0xe8, 0, 0, 0, 0, // segment index, 50 bytes, 1000 ticks
    ];

    /// The `sidx` of [`SIDX_V0`] in version 1, from time 2^32 and 2^32 bytes past the anchor, of the first reference alone
    const SIDX_V1: &[u8] = &[
        1, 0, 0, 0, // version, flags
        0, 0, 0, 1, // reference_ID
        0, 0, 0x03, 0xe8, // timescale
        0, 0, 0, 1, 0, 0, 0, 0, // earliest_presentation_time
        0, 0, 0, 1, 0, 0, 0, 0, // first_offset
        0, 0, 0, 1, // reserved, reference_count
        0, 0, 0x07, 0xd0, 0, 0, 0x0b, 0xb8, 0x90, 0, 0,
        0, // media, 2000 bytes, 3000 ticks, SAP type 1
    ];

    #[test]
    fn the_subsegments_of_an_sidx_lie_one_after_another_from_its_anchor() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader
            .handle_segment_index(
                &SegmentIndexBox::decode_payload(SIDX_V0).unwrap(),
                anchored_at(1000),
            )
            .unwrap();

        let media = Subsegment {
            byte_range: 1100..3100,
            presentation_time: 500..3500,
            references_segment_index: false,
            starts_with_sap: true,
            sap_type: 1,
            sap_delta_time: 0,
        };
        let nested = Subsegment {
            byte_range: 3100..3150,
            presentation_time: 3500..4500,
            references_segment_index: true,
            starts_with_sap: false,
            sap_type: 0,
            sap_delta_time: 0,
        };
        let expected = SegmentIndex {
            byte_range: anchored_at(1000),
            reference_id: 1,
            timescale: 1000,
            subsegments: Vec::from([media.clone(), nested.clone()]),
        };
        assert_eq!(reader.segment_indexes(), [expected]);
        let index = reader.segment_indexes().first().unwrap();
        assert_eq!(index.subsegment_at(3499), Some(&media));
        assert_eq!(index.subsegment_at(3500), Some(&nested));
        assert_eq!(index.subsegment_at(499), None);
    }

    #[test]
    fn an_sidx_of_version_1_counts_in_64_bits() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader
            .handle_segment_index(
                &SegmentIndexBox::decode_payload(SIDX_V1).unwrap(),
                anchored_at(8),
            )
            .unwrap();

        assert_eq!(
            reader.segment_indexes(),
            [SegmentIndex {
                byte_range: anchored_at(8),
                reference_id: 1,
                timescale: 1000,
                subsegments: Vec::from([Subsegment {
                    byte_range: 0x1_0000_0008..0x1_0000_07d8,
                    presentation_time: 0x1_0000_0000..0x1_0000_0bb8,
                    references_segment_index: false,
                    starts_with_sap: true,
                    sap_type: 1,
                    sap_delta_time: 0,
                }]),
            }]
        );
    }

    #[test]
    fn an_index_kept_once_is_not_kept_again_and_resuming_keeps_it_and_the_mfra() {
        let sidx = SegmentIndexBox::decode_payload(SIDX_V0).unwrap();
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader
            .handle_segment_index(&sidx, anchored_at(1000))
            .unwrap();
        reader.handle_movie_fragment_random_access(MovieFragmentRandomAccessBox::default());
        reader.resume();
        reader
            .handle_segment_index(&sidx, anchored_at(1000))
            .unwrap();
        reader
            .handle_segment_index(&sidx, anchored_at(2000))
            .unwrap();

        let first = SegmentIndex::new(&sidx, anchored_at(1000)).unwrap();
        let second = SegmentIndex::new(&sidx, anchored_at(2000)).unwrap();
        assert_eq!(reader.segment_indexes(), [first, second]);
        assert_eq!(
            reader.movie_fragment_random_access(),
            Some(&MovieFragmentRandomAccessBox::default())
        );
    }

    #[test]
    fn an_index_past_the_limits_is_refused_and_not_kept() {
        let sidx = SegmentIndexBox::decode_payload(SIDX_V0).unwrap();
        let mut reader = StructureDataReader::new(StructureDataLimits {
            segment_index_count: 2,
            subsegment_count: 5,
            ..UNLIMITED
        });
        reader
            .handle_segment_index(&sidx, anchored_at(1000))
            .unwrap();
        reader
            .handle_segment_index(&sidx, anchored_at(2000))
            .unwrap();

        assert_eq!(
            reader.handle_segment_index(&sidx, anchored_at(3000)),
            Err(DemuxError::SegmentIndexCountLimitExceeded {
                retained_segment_indexes: 3,
                limit_segment_indexes: 2
            })
        );
        assert_eq!(
            reader.handle_segment_index(&sidx, anchored_at(1000)),
            Ok(())
        );
        assert_eq!(
            reader.segment_indexes(),
            [
                SegmentIndex::new(&sidx, anchored_at(1000)).unwrap(),
                SegmentIndex::new(&sidx, anchored_at(2000)).unwrap()
            ]
        );
    }

    #[test]
    fn subsegments_past_the_limit_are_refused_whether_one_sidx_or_the_kept_ones_hold_them() {
        let sidx = SegmentIndexBox::decode_payload(SIDX_V0).unwrap();
        let mut reader = StructureDataReader::new(StructureDataLimits {
            subsegment_count: 3,
            ..UNLIMITED
        });
        let mut too_few = StructureDataReader::new(StructureDataLimits {
            subsegment_count: 1,
            ..UNLIMITED
        });
        reader
            .handle_segment_index(&sidx, anchored_at(1000))
            .unwrap();

        assert_eq!(
            too_few.handle_segment_index(&sidx, anchored_at(1000)),
            Err(DemuxError::SubsegmentCountLimitExceeded {
                retained_subsegments: 2,
                limit_subsegments: 1
            })
        );
        assert_eq!(
            reader.handle_segment_index(&sidx, anchored_at(2000)),
            Err(DemuxError::SubsegmentCountLimitExceeded {
                retained_subsegments: 4,
                limit_subsegments: 3
            })
        );
        assert_eq!(
            reader.segment_indexes(),
            [SegmentIndex::new(&sidx, anchored_at(1000)).unwrap()]
        );
    }

    #[test]
    fn an_index_past_64_bits_is_refused() {
        let sidx = SegmentIndexBox::decode_payload(SIDX_V1).unwrap();

        assert_eq!(
            StructureDataReader::new(UNLIMITED).handle_segment_index(&sidx, anchored_at(u64::MAX)),
            Err(DemuxError::SegmentIndexOutOfRange { reference_id: 1 })
        );
    }

    #[test]
    fn the_mfra_starts_the_size_its_mfro_repeats_before_the_end_of_the_file() {
        let mfro = *b"\0\0\0\x10mfro\0\0\0\0\0\0\0\x3d";
        let not_an_mfro = *b"\0\0\0\x10free\0\0\0\0\0\0\0\x3d";
        let mfro_of_version_1 = *b"\0\0\0\x10mfro\x01\0\0\0\0\0\0\x3d";

        assert_eq!(movie_fragment_random_access_start(61, &mfro), Some(0));
        assert_eq!(movie_fragment_random_access_start(60, &mfro), None);
        assert_eq!(movie_fragment_random_access_start(1000, &not_an_mfro), None);
        assert_eq!(
            movie_fragment_random_access_start(1000, &mfro_of_version_1),
            None
        );
    }
}
