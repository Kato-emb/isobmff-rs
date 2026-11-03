//! [`SegmentIndex`] and [`Subsegment`], a `sidx` resolved to the byte ranges and the times of the subsegments it indexes, and the `sidx` of the subsegments a muxer writes

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{MovieBox, SegmentIndexBox, SegmentIndexReference};

use super::movie_fragment::Run;
use super::writer::version_for;
use crate::error::{DemuxError, MuxError};
use crate::timeline::{Timeline, composition_time};

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

/// Subsegments of the movie fragments written, each a `moof` and its `mdat`, indexed by the samples of the first track of the movie, ISO/IEC 14496-12 §8.14.3
#[derive(Debug)]
pub(super) struct SubsegmentTable {
    reference_id: u32,
    timescale: u32,
    earliest_presentation_time: Option<u64>,
    /// Index of the last subsegment with samples of the reference track, and their earliest presentation time
    ///
    /// Its duration runs to its own end until a next such subsegment starts.
    last_presented: Option<(usize, u64)>,
    references: Vec<SegmentIndexReference>,
}

impl SubsegmentTable {
    /// Creates the table of subsegments whose reference stream is the first track of `movie`, in the timescale of its media
    ///
    /// The presentation times a `sidx` states are those the edit list of
    /// the track maps the composition times of its samples to (§8.14.3.1),
    /// as a [`Timeline`] maps them.
    ///
    /// # Errors
    ///
    /// * [`Timeline`](MuxError::Timeline): the edit list of the reference
    ///   track does not map its composition timeline.
    pub(super) fn new(movie: &MovieBox) -> Result<Self, MuxError> {
        let first = movie.trak.first();
        let reference_id = first.map_or(0, |trak| trak.tkhd.track_id);
        if first.is_some() {
            Timeline::new(movie, reference_id).map_err(|error| MuxError::Timeline { error })?;
        }

        Ok(Self {
            reference_id,
            timescale: first.map_or(0, |trak| trak.mdia.mdhd.timescale),
            earliest_presentation_time: None,
            last_presented: None,
            references: Vec::new(),
        })
    }

    /// Indexes the movie fragment of `track_fragments` of `movie`, which with its media data takes `referenced_size` bytes, as the next subsegment
    ///
    /// The presentation time of a sample is the first the edit list of its
    /// track presents it at; a sample it does not present has none. A
    /// subsegment lasts until the earliest presentation time of the next
    /// with samples of the reference track presented, the last until the end
    /// of its own, and one without such samples lasts no time. It starts with
    /// a stream access point where the first sample of the reference track
    /// in it is a sync sample, and contains one where any is: of type 1 where
    /// that sample is composed first of those from it on in decoding order,
    /// and of type 2 otherwise (Annex I), at the earliest presentation time
    /// of those. A stream access point too far into the subsegment for
    /// `SAP_delta_time` is not stated, nor one none of whose samples from it
    /// on is presented.
    ///
    /// # Errors
    ///
    /// * [`Timeline`](MuxError::Timeline): the edit list of the reference
    ///   track does not map its composition timeline.
    /// * [`SegmentIndexOutOfRange`](MuxError::SegmentIndexOutOfRange): the
    ///   `sidx` cannot state the subsegment: it is longer than 31 bits of
    ///   bytes, past 65,535 subsegments, or the subsegment starts earlier
    ///   than the one before it, or more than 32 bits of ticks after it or
    ///   before its own end.
    pub(super) fn handle_movie_fragment(
        &mut self,
        movie: &MovieBox,
        track_fragments: &[Vec<&Run<'_>>],
        referenced_size: u64,
    ) -> Result<(), MuxError> {
        let timeline = Timeline::new(movie, self.reference_id)
            .map_err(|error| MuxError::Timeline { error })?;
        let out_of_range = MuxError::SegmentIndexOutOfRange {
            reference_id: self.reference_id,
        };
        let ticks = |ticks: Option<u64>| {
            ticks
                .and_then(|ticks| u32::try_from(ticks).ok())
                .ok_or(out_of_range)
        };
        let mut reference = SegmentIndexReference::default();
        reference.referenced_size = u32::try_from(referenced_size)
            .ok()
            .filter(|&size| size < 1 << 31)
            .ok_or(out_of_range)?;
        if self.references.len() >= usize::from(u16::MAX) {
            return Err(out_of_range);
        }
        let samples: Vec<(bool, i128, Option<Range<u64>>)> = track_fragments
            .iter()
            .flatten()
            .filter(|run| run.track == 0)
            .flat_map(|run| run.samples)
            .map(|sample| {
                let properties = sample.properties();
                let presented = match timeline.present(properties).next() {
                    Some(presentation) => {
                        let start = presentation.presentation_time();
                        Some(start..start.checked_add(presentation.duration())?)
                    }
                    None => None,
                };
                Some((
                    !properties.sample_flags.sample_is_non_sync_sample,
                    composition_time(properties),
                    presented,
                ))
            })
            .collect::<Option<_>>()
            .ok_or(out_of_range)?;
        let presentation_time = samples
            .iter()
            .filter_map(|(_, _, time)| time.clone())
            .reduce(|earlier, later| earlier.start.min(later.start)..earlier.end.max(later.end));
        let mut previous_duration = None;
        if let Some(time) = &presentation_time {
            reference.subsegment_duration = ticks(time.end.checked_sub(time.start))?;
            if let Some((previous, previous_start)) = self.last_presented {
                previous_duration =
                    Some((previous, ticks(time.start.checked_sub(previous_start))?));
            }
            let first_sync = samples.iter().position(|&(sync, _, _)| sync);
            let from_first_sync = first_sync.and_then(|first_sync| samples.get(first_sync..));
            if let Some(from_sync @ [(_, sync_composition_time, _), ..]) = from_first_sync {
                let composed_first = from_sync
                    .iter()
                    .all(|(_, composition_time, _)| composition_time >= sync_composition_time);
                let sap_time = from_sync
                    .iter()
                    .filter_map(|(_, _, time)| time.as_ref().map(|time| time.start))
                    .min();
                if let Some(sap_delta_time) = sap_time
                    .and_then(|sap_time| sap_time.checked_sub(time.start))
                    .and_then(|delta| u32::try_from(delta).ok())
                    .filter(|&delta| delta < 1 << 28)
                {
                    reference.starts_with_sap = first_sync == Some(0);
                    reference.sap_type = if composed_first { 1 } else { 2 };
                    reference.sap_delta_time = sap_delta_time;
                }
            }
        }

        if let Some((previous, duration)) = previous_duration
            && let Some(previous) = self.references.get_mut(previous)
        {
            previous.subsegment_duration = duration;
        }
        if let Some(time) = presentation_time {
            self.earliest_presentation_time.get_or_insert(time.start);
            self.last_presented = Some((self.references.len(), time.start));
        }
        self.references.push(reference);

        Ok(())
    }

    /// Returns the `sidx` of the subsegments indexed, in their order from its anchor point
    ///
    /// The `sidx` takes version 0 where its earliest presentation time fits
    /// in 32 bits.
    pub(super) fn finish(self) -> SegmentIndexBox {
        let mut sidx = SegmentIndexBox::default();
        sidx.reference_id = self.reference_id;
        sidx.timescale = self.timescale;
        sidx.earliest_presentation_time = self.earliest_presentation_time.unwrap_or_default();
        sidx.version = version_for(sidx.earliest_presentation_time);
        sidx.references = self.references;

        sidx
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::{
        EditBox, EditListBox, EditListEntry, MovieBox, MovieExtendsBox,
        MovieFragmentRandomAccessBox, SegmentIndexBox, SegmentIndexReference,
    };
    use isobmff_core::BoxDecode;

    use alloc::vec::Vec;
    use core::ops::Range;

    use super::{SegmentIndex, Subsegment, SubsegmentTable};
    use crate::error::{DemuxError, MuxError, TimelineError};
    use crate::sample::Sample;
    use crate::structure_data::fixtures::{UNLIMITED, laid_out, movie, sample, track};
    use crate::structure_data::movie_fragment::{runs, track_fragments};
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

    /// Returns a movie of tracks 1 and 2, track 1 at 100 ticks a second
    fn two_tracks() -> MovieBox {
        let mut movie = movie([1, 2].map(|track_id| track(track_id, 0, &[], &[], &[])));
        if let Some(trak) = movie.trak.first_mut() {
            trak.mdia.mdhd.timescale = 100;
        }
        movie
    }

    /// Indexes the movie fragment of `samples` of `movie`, `referenced_size` bytes with its media data, in `table`
    fn index(
        table: &mut SubsegmentTable,
        movie: &MovieBox,
        samples: &[Sample],
        referenced_size: u64,
    ) -> Result<(), MuxError> {
        let locations = laid_out(samples, 0);
        let runs = runs(movie, &locations, &[]).unwrap();
        table.handle_movie_fragment(movie, &track_fragments(&runs), referenced_size)
    }

    /// Returns the reference to a subsegment of `referenced_size` bytes lasting `subsegment_duration`, starting with a SAP where `starts_with_sap`, of `sap_type` `sap_delta_time` in
    fn reference(
        referenced_size: u32,
        subsegment_duration: u32,
        starts_with_sap: bool,
        sap_type: u8,
        sap_delta_time: u32,
    ) -> SegmentIndexReference {
        let mut reference = SegmentIndexReference::default();
        reference.referenced_size = referenced_size;
        reference.subsegment_duration = subsegment_duration;
        reference.starts_with_sap = starts_with_sap;
        reference.sap_type = sap_type;
        reference.sap_delta_time = sap_delta_time;
        reference
    }

    #[test]
    fn each_movie_fragment_is_a_subsegment_lasting_until_the_earliest_presentation_time_of_the_next_of_the_first_track()
     {
        let movie = two_tracks();
        let mut table = SubsegmentTable::new(&movie).unwrap();
        let fragments: [(&[Sample], u64); 4] = [
            (
                &[
                    sample(1, 0, 10, 20, true),
                    sample(1, 10, 10, 0, false),
                    sample(2, 0, 50, 0, true),
                    sample(1, 20, 10, 0, false),
                ],
                100,
            ),
            (
                &[sample(1, 30, 10, 0, false), sample(1, 40, 10, 0, true)],
                200,
            ),
            (&[sample(2, 50, 50, 0, true)], 300),
            (&[sample(1, 50, 10, 0, true)], 400),
        ];

        for (samples, referenced_size) in fragments {
            index(&mut table, &movie, samples, referenced_size).unwrap();
        }

        let mut expected = SegmentIndexBox::default();
        expected.reference_id = 1;
        expected.timescale = 100;
        expected.earliest_presentation_time = 10;
        expected.references = Vec::from([
            reference(100, 20, true, 2, 0),
            reference(200, 20, false, 1, 10),
            reference(300, 0, false, 0, 0),
            reference(400, 10, true, 1, 0),
        ]);
        assert_eq!(table.finish(), expected);
    }

    #[test]
    fn a_subsegment_the_sidx_cannot_state_is_refused_and_not_indexed() {
        let movie = two_tracks();
        let mut table = SubsegmentTable::new(&movie).unwrap();
        index(&mut table, &movie, &[sample(1, 100, 10, 0, true)], 100).unwrap();

        let refused = [
            index(&mut table, &movie, &[sample(1, 110, 10, 0, true)], 1 << 31),
            index(&mut table, &movie, &[sample(1, 0, 10, -1, true)], 100),
            index(&mut table, &movie, &[sample(1, 50, 10, 0, true)], 100),
            index(
                &mut table,
                &movie,
                &[sample(1, 100 + (1 << 32), 10, 0, true)],
                100,
            ),
        ];

        assert_eq!(
            refused,
            [Err(MuxError::SegmentIndexOutOfRange { reference_id: 1 }); 4]
        );
        let mut expected = SegmentIndexBox::default();
        expected.reference_id = 1;
        expected.timescale = 100;
        expected.earliest_presentation_time = 100;
        expected.references = Vec::from([reference(100, 10, true, 1, 0)]);
        assert_eq!(table.finish(), expected);
    }

    #[test]
    fn a_stream_access_point_past_what_sap_delta_time_holds_is_not_stated() {
        let movie = two_tracks();
        let mut table = SubsegmentTable::new(&movie).unwrap();

        index(
            &mut table,
            &movie,
            &[
                sample(1, 0, 1 << 28, 0, false),
                sample(1, 1 << 28, 10, 0, true),
            ],
            100,
        )
        .unwrap();

        let mut expected = SegmentIndexBox::default();
        expected.reference_id = 1;
        expected.timescale = 100;
        expected.references = Vec::from([reference(100, (1 << 28) + 10, false, 0, 0)]);
        assert_eq!(table.finish(), expected);
    }

    /// Returns [`two_tracks`], fragmented, in a movie of timescale 100, with an edit list on track 1 of one edit from `media_time` to the end of the media
    fn edited(media_time: i64) -> MovieBox {
        let mut movie = two_tracks();
        movie.mvhd.timescale = 100;
        movie.mvex = Some(MovieExtendsBox::default());
        let mut edit = EditListEntry::default();
        edit.media_time = media_time;
        edit.media_rate_integer = 1;
        let mut elst = EditListBox::default();
        elst.entries = Vec::from([edit]);
        let mut edts = EditBox::default();
        edts.elst = Some(elst);
        if let Some(trak) = movie.trak.first_mut() {
            trak.edts = Some(edts);
        }
        movie
    }

    #[test]
    fn the_subsegments_last_from_the_presentation_times_the_edit_list_of_the_first_track_maps() {
        let movie = edited(8);
        let mut table = SubsegmentTable::new(&movie).unwrap();
        let samples =
            [0, 4, 8, 12].map(|decoding_time| sample(1, decoding_time, 4, 0, decoding_time == 0));

        index(&mut table, &movie, &samples, 100).unwrap();

        let mut expected = SegmentIndexBox::default();
        expected.reference_id = 1;
        expected.timescale = 100;
        expected.references = Vec::from([reference(100, 8, true, 1, 0)]);
        assert_eq!(table.finish(), expected);
    }

    #[test]
    fn a_reference_track_whose_edit_list_does_not_map_its_composition_timeline_is_refused() {
        assert_eq!(
            SubsegmentTable::new(&edited(-2)).map(|_| ()),
            Err(MuxError::Timeline {
                error: TimelineError::InvalidMediaTime {
                    track_id: 1,
                    media_time: -2
                }
            })
        );
    }
}
