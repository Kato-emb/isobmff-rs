//! The compressed boxes a structure-data reader has been told of, and the file offsets they move, ISO/IEC 14496-12 §8.16
//!
//! The file offsets in a box a compressed box replaces count in the file as
//! it would be with its compressed boxes read back (§8.16.1): the
//! uncompressed domain. The `otyp` stays in place there (§8.16.5 replaces it
//! only when a file is decompressed into another, §8.16.2.2). A sample whose
//! media data such an offset names lies in the file at that offset less the
//! bytes the compressed boxes before it grew by when read back, which is
//! known once every compressed box before it is read (§8.16.2.2 NOTE 3).

use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{MovieBox, MovieFragmentBox};
#[cfg(feature = "compressed-boxes")]
use isobmff_core::BoxHeader;
use isobmff_core::BoxType;

#[cfg(feature = "compressed-boxes")]
use super::StructureDataReader;
#[cfg(feature = "compressed-boxes")]
use crate::convert::to_u64;
use crate::error::DemuxError;
use crate::sample::SampleLocation;

/// Returns `offset`, which lies past `from_end` where it is counted, counted where `from_end` is `to_end`
const fn shifted(offset: u64, from_end: u64, to_end: u64) -> u64 {
    to_end.saturating_add(offset.saturating_sub(from_end))
}

/// Compressed box read: where it lies in the file, and in the uncompressed domain the box it replaces
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Compressed {
    box_type: BoxType,
    /// Where the compressed box lies in the file
    offset: u64,
    length: u64,
    /// Where the box it replaces lies in the uncompressed domain
    uncompressed_offset: u64,
    uncompressed_length: u64,
}

impl Compressed {
    /// Returns the file offset just past the compressed box
    const fn end(&self) -> u64 {
        self.offset.saturating_add(self.length)
    }

    /// Returns the offset in the uncompressed domain just past the box it replaces
    const fn uncompressed_end(&self) -> u64 {
        self.uncompressed_offset
            .saturating_add(self.uncompressed_length)
    }
}

/// The compressed boxes a structure-data reader has been told of, in the order they lie, and what it needs to tell where the offsets of the uncompressed domain lie in the file
#[derive(Debug)]
pub(super) struct CompressedBoxes {
    boxes: Vec<Compressed>,
    #[cfg(feature = "compressed-boxes")]
    limit_compressed_boxes: u64,
    /// File offset just past the media data of the samples in the file located so far
    located_end: u64,
    /// Whether the reader has resumed at a later box, past compressed boxes it may not have been told of
    resumed: bool,
    /// Whether the file is known to have been transformed, by a compressed box or an `otyp` read
    transformed: bool,
}

impl CompressedBoxes {
    /// Creates the record of no compressed box, which keeps the places of up to `limit_compressed_boxes`
    pub(super) const fn new(limit_compressed_boxes: u64) -> Self {
        #[cfg(not(feature = "compressed-boxes"))]
        let _ = limit_compressed_boxes;
        Self {
            boxes: Vec::new(),
            #[cfg(feature = "compressed-boxes")]
            limit_compressed_boxes,
            located_end: 0,
            resumed: false,
            transformed: false,
        }
    }

    /// Returns the type of the first compressed box in the file of those read
    pub(super) fn first_box_type(&self) -> Option<BoxType> {
        self.boxes.first().map(|compressed| compressed.box_type)
    }

    /// Notes that the file was transformed, as an `otyp` read says (§8.16.5)
    #[cfg(feature = "compressed-boxes")]
    pub(super) const fn note_transformed(&mut self) {
        self.transformed = true;
    }

    /// Notes that the reader resumes at a later box: it may not have been told of every compressed box before it
    pub(super) const fn resume(&mut self) {
        self.resumed = true;
    }

    /// Refuses `moof` where a track fragment of it takes a base data offset that a compressed box skipped by a resume may have moved, §8.16.2.2 NOTE 3
    ///
    /// # Errors
    ///
    /// * [`DataOffsetPastSkippedCompressedBoxes`](DemuxError::DataOffsetPastSkippedCompressedBoxes):
    ///   the reader has resumed in a file known to have been transformed,
    ///   and a track fragment of `moof` takes a base data offset.
    pub(super) fn refuse_base_data_offset_past_skipped(
        &self,
        moof: &MovieFragmentBox,
    ) -> Result<(), DemuxError> {
        if !(self.resumed && self.transformed) {
            return Ok(());
        }
        moof.traf
            .iter()
            .find(|traf| traf.tfhd.base_data_offset.is_some())
            .map_or(Ok(()), |traf| {
                Err(DemuxError::DataOffsetPastSkippedCompressedBoxes {
                    track_id: traf.tfhd.track_id,
                })
            })
    }

    /// Returns the offset in the uncompressed domain of file offset `offset`, where no compressed box read lies
    pub(super) fn uncompressed(&self, offset: u64) -> u64 {
        let before = self
            .boxes
            .partition_point(|compressed| compressed.end() <= offset);
        self.boxes
            .get(before.wrapping_sub(1))
            .map_or(offset, |previous| {
                shifted(offset, previous.end(), previous.uncompressed_end())
            })
    }

    /// Records the compressed box of `box_type` that lies `length` bytes at file offset `offset` and replaces a box of `uncompressed_length` bytes
    ///
    /// A box recorded before, as one read again after a resume, is recorded
    /// once.
    ///
    /// # Errors
    ///
    /// * [`CompressedBoxBeforeLocatedSample`](DemuxError::CompressedBoxBeforeLocatedSample):
    ///   the box grows or shrinks when read back, and starts before the media
    ///   data of a sample located already: the sample was located without
    ///   it, and its location is wrong (§8.16.2.2 NOTE 3).
    /// * [`CompressedBoxCountLimitExceeded`](DemuxError::CompressedBoxCountLimitExceeded):
    ///   recording the box would take the boxes recorded past the limit.
    #[cfg(feature = "compressed-boxes")]
    pub(super) fn record(
        &mut self,
        box_type: BoxType,
        offset: u64,
        length: u64,
        uncompressed_length: u64,
    ) -> Result<(), DemuxError> {
        self.transformed = true;
        let at = self
            .boxes
            .partition_point(|compressed| compressed.offset < offset);
        if self
            .boxes
            .get(at)
            .is_some_and(|compressed| compressed.offset == offset)
        {
            return Ok(());
        }
        if uncompressed_length != length && offset < self.located_end {
            return Err(DemuxError::CompressedBoxBeforeLocatedSample { box_type });
        }
        let retained_compressed_boxes = to_u64(self.boxes.len()).saturating_add(1);
        if retained_compressed_boxes > self.limit_compressed_boxes {
            return Err(DemuxError::CompressedBoxCountLimitExceeded {
                retained_compressed_boxes,
                limit_compressed_boxes: self.limit_compressed_boxes,
            });
        }
        // Why not leave the growth to `insert`: it may reserve past the limit, which bounds what the boxes allocate as well as how many they are.
        if self.boxes.len() == self.boxes.capacity() {
            let limit = usize::try_from(self.limit_compressed_boxes).unwrap_or(usize::MAX);
            let grown = self.boxes.len().saturating_mul(2).max(4).min(limit);
            self.boxes
                .reserve_exact(grown.saturating_sub(self.boxes.len()).max(1));
        }
        self.boxes.insert(
            at,
            Compressed {
                box_type,
                offset,
                length,
                uncompressed_offset: offset,
                uncompressed_length,
            },
        );
        // Why not place the box alone: each box counts its place in the uncompressed domain from the boxes before it, and those after it count this one.
        let mut previous = at
            .checked_sub(1)
            .and_then(|before| self.boxes.get(before))
            .map(|previous| (previous.end(), previous.uncompressed_end()));
        for compressed in self.boxes.iter_mut().skip(at) {
            compressed.uncompressed_offset = previous
                .map_or(compressed.offset, |(end, uncompressed_end)| {
                    shifted(compressed.offset, end, uncompressed_end)
                });
            previous = Some((compressed.end(), compressed.uncompressed_end()));
        }
        Ok(())
    }

    /// Notes that the media data of the samples in the file located so far ends at file offset `end`, as [`locate`](Self::locate) returns it
    pub(super) fn note_located(&mut self, end: u64) {
        self.located_end = self.located_end.max(end);
    }

    /// Locates in the file those of `locations` whose media data the uncompressed domain locates in the file itself, as `in_this_file` tells, and returns the file offset past the media data of them and of those located before
    ///
    /// Locations in another file are left as they are (§8.16.1). The end is
    /// for [`note_located`](Self::note_located), once the locations are
    /// returned.
    ///
    /// # Errors
    ///
    /// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange): the
    ///   media data of a sample lies in the box a compressed box replaces,
    ///   where the file holds no media data.
    pub(super) fn locate(
        &self,
        locations: &mut [SampleLocation],
        in_this_file: impl Fn(&SampleLocation) -> bool,
    ) -> Result<u64, DemuxError> {
        let mut located_end = self.located_end;
        for location in locations
            .iter_mut()
            .filter(|location| in_this_file(location))
        {
            if !self.boxes.is_empty() {
                let in_file = self.in_file(location.byte_range()).ok_or(
                    DemuxError::DataOffsetOutOfRange {
                        track_id: location.properties().track_id,
                    },
                )?;
                *location = location.with_byte_range(in_file);
            }
            located_end = located_end.max(location.byte_range().end);
        }
        Ok(located_end)
    }

    /// Locates in the file those of `locations`, samples of `movie`, whose media data is in the file itself, as [`locate`](Self::locate) does
    ///
    /// # Errors
    ///
    /// As for [`locate`](Self::locate).
    pub(super) fn locate_in_this_file(
        &self,
        movie: Option<&MovieBox>,
        locations: &mut [SampleLocation],
    ) -> Result<u64, DemuxError> {
        self.locate(locations, |location| {
            movie
                .and_then(|movie| {
                    super::sample_description::data_reference(movie, location.data_reference())
                })
                .is_some_and(|data_reference| data_reference.entry().is_in_this_file())
        })
    }

    /// Returns where `range` of the uncompressed domain lies in the file, or `None` where it runs into a box a compressed box replaces
    fn in_file(&self, range: Range<u64>) -> Option<Range<u64>> {
        let before = self
            .boxes
            .partition_point(|compressed| compressed.uncompressed_end() <= range.start);
        if let Some(next) = self.boxes.get(before) {
            let runs_into = if range.is_empty() {
                next.uncompressed_offset < range.start
            } else {
                next.uncompressed_offset < range.end
            };
            if runs_into {
                return None;
            }
        }
        let Some(previous) = self.boxes.get(before.wrapping_sub(1)) else {
            return Some(range);
        };
        let in_file = |offset: u64| shifted(offset, previous.uncompressed_end(), previous.end());
        Some(in_file(range.start)..in_file(range.end))
    }
}

#[cfg(feature = "compressed-boxes")]
impl StructureDataReader {
    /// Keeps the compressed box of `box_type` that lies `length` bytes at file offset `offset`, and reads back into a box of `replacement_type` of a payload of `payload_len` bytes, ISO/IEC 14496-12 §8.16
    ///
    /// A reader of the boxes at file level tells of each compressed box it
    /// reads back, before the box it replaces is handed to the reader: the
    /// samples the reader locates after it lie past it as it grew or shrank.
    /// The box read back takes the header its type and the length of its
    /// payload give it (§8.16.2.1). A box told of before, as one read again
    /// after a resume, is kept once.
    ///
    /// # Errors
    ///
    /// * [`CompressedBoxWithMovieFragmentRandomAccess`](DemuxError::CompressedBoxWithMovieFragmentRandomAccess):
    ///   the reader keeps an `mfra`, and a file has no compressed box with one
    ///   (§8.16.1).
    /// * [`CompressedBoxBeforeLocatedSample`](DemuxError::CompressedBoxBeforeLocatedSample):
    ///   the box grows or shrinks when read back, and starts before the media
    ///   data of a sample located already: the sample was located without
    ///   it (§8.16.2.2 NOTE 3).
    /// * [`CompressedBoxCountLimitExceeded`](DemuxError::CompressedBoxCountLimitExceeded):
    ///   keeping the box would take those the reader keeps past the limit.
    #[cfg(feature = "compressed-boxes")]
    pub fn handle_compressed_box(
        &mut self,
        box_type: BoxType,
        offset: u64,
        length: u64,
        replacement_type: BoxType,
        payload_len: u64,
    ) -> Result<(), DemuxError> {
        if self.movie_fragment_random_access.is_some() {
            return Err(DemuxError::CompressedBoxWithMovieFragmentRandomAccess { box_type });
        }
        let uncompressed_length = BoxHeader::with_payload_len(replacement_type, payload_len)
            .map_or(u64::MAX, |header| header.encoded_len())
            .saturating_add(payload_len);
        self.compressed
            .record(box_type, offset, length, uncompressed_length)
    }

    /// Notes that the file has an `otyp`, so that its box structure was transformed, ISO/IEC 14496-12 §8.16.5
    ///
    /// After a [`resume`](Self::resume), a `moof` of a file known to have been
    /// transformed, by an `otyp` or a compressed box told of, is refused where
    /// it takes a base data offset, as for
    /// [`handle_movie_fragment`](Self::handle_movie_fragment).
    #[cfg(feature = "compressed-boxes")]
    pub const fn handle_original_file_type(&mut self) {
        self.compressed.note_transformed();
    }
}

#[cfg(test)]
#[cfg(feature = "compressed-boxes")]
mod tests {
    use isobmff_boxes::SampleFlags;
    use isobmff_core::BoxType;

    use super::CompressedBoxes;
    use crate::error::DemuxError;
    use crate::sample::{DataReferenceKey, SampleLocation, SampleProperties};

    const MOVIE: BoxType = BoxType::compact(*b"!mov");
    const INDEX: BoxType = BoxType::compact(*b"!six");

    /// Returns the location of a sample of track 1 whose media data is `byte_range`
    fn at(byte_range: core::ops::Range<u64>) -> SampleLocation {
        let properties = SampleProperties {
            track_id: 1,
            decoding_time: 0,
            sample_duration: 1,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::default(),
            sample_description_index: 1,
        };
        SampleLocation::new(properties, DataReferenceKey::new(0), byte_range)
    }

    /// Returns the boxes of a `!mov` of 10 bytes at file offset 0 that reads back into 20, and a `!six` of 10 bytes at 30 that reads back into 5, recorded in the order `in_order` says
    fn recorded(in_order: bool) -> CompressedBoxes {
        let mut boxes = CompressedBoxes::new(8);
        let movie = (MOVIE, 0, 10, 20);
        let index = (INDEX, 30, 10, 5);
        let order = if in_order {
            [movie, index]
        } else {
            [index, movie]
        };
        for (box_type, offset, length, uncompressed_length) in order {
            boxes
                .record(box_type, offset, length, uncompressed_length)
                .unwrap();
        }
        boxes
    }

    /// Returns where `boxes` locates `location` in the file
    fn located(
        boxes: &CompressedBoxes,
        location: SampleLocation,
    ) -> Result<core::ops::Range<u64>, DemuxError> {
        let mut locations = [location];
        boxes.locate(&mut locations, |_| true)?;
        Ok(locations[0].byte_range())
    }

    #[test]
    fn an_offset_moves_by_what_the_compressed_boxes_before_it_grew_and_shrank_by_in_whatever_order_they_are_read()
     {
        for in_order in [true, false] {
            let boxes = recorded(in_order);
            assert_eq!(
                (
                    [5, 20, 30, 50].map(|offset| boxes.uncompressed(offset)),
                    located(&boxes, at(15..20)),
                    located(&boxes, at(55..57)),
                    located(&boxes, at(41..43)),
                    boxes.first_box_type(),
                ),
                (
                    [5, 30, 40, 55],
                    Err(DemuxError::DataOffsetOutOfRange { track_id: 1 }),
                    Ok(50..52),
                    Err(DemuxError::DataOffsetOutOfRange { track_id: 1 }),
                    Some(MOVIE),
                ),
                "in order: {in_order}"
            );
        }
    }

    #[test]
    fn a_box_recorded_again_is_recorded_once_and_one_past_a_located_sample_or_the_limit_is_refused()
    {
        let mut boxes = recorded(true);
        boxes.record(MOVIE, 0, 10, 20).unwrap();
        let located_end = boxes.locate(&mut [at(55..57)], |_| true).unwrap();
        boxes.note_located(located_end);
        let mut limited = CompressedBoxes::new(1);
        limited.record(MOVIE, 0, 10, 20).unwrap();
        let mut five = CompressedBoxes::new(5);
        for offset in 0..5 {
            five.record(INDEX, offset * 10, 10, 10).unwrap();
        }
        assert_eq!(
            (limited.boxes.capacity(), five.boxes.capacity()),
            (1, 5),
            "the boxes hold no more than the limit"
        );

        assert_eq!(
            (
                boxes.uncompressed(50),
                boxes.record(INDEX, 45, 10, 12),
                boxes.record(INDEX, 45, 10, 10),
                limited.record(INDEX, 30, 10, 5),
            ),
            (
                55,
                Err(DemuxError::CompressedBoxBeforeLocatedSample { box_type: INDEX }),
                Ok(()),
                Err(DemuxError::CompressedBoxCountLimitExceeded {
                    retained_compressed_boxes: 2,
                    limit_compressed_boxes: 1
                }),
            )
        );
    }
}

#[cfg(test)]
#[cfg(not(feature = "compressed-boxes"))]
mod without_the_feature {
    use isobmff_boxes::SampleFlags;

    use super::CompressedBoxes;
    use crate::sample::{DataReferenceKey, SampleLocation, SampleProperties};

    #[test]
    fn no_compressed_box_is_kept_and_every_offset_stays_where_it_is() {
        let properties = SampleProperties {
            track_id: 1,
            decoding_time: 0,
            sample_duration: 1,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::default(),
            sample_description_index: 1,
        };
        let location = SampleLocation::new(properties, DataReferenceKey::new(0), 15..20);
        let boxes = CompressedBoxes::new(0);
        let mut locations = [location.clone()];

        assert_eq!(
            (
                [0, 15, 1 << 40].map(|offset| boxes.uncompressed(offset)),
                boxes.locate(&mut locations, |_| true),
                locations,
            ),
            ([0, 15, 1 << 40], Ok(20), [location])
        );
    }
}

#[cfg(test)]
#[cfg(feature = "compressed-boxes")]
mod reader_tests {
    use alloc::vec::Vec;

    use isobmff_boxes::MovieBox;
    use isobmff_core::{BoxDefinition, BoxType};

    use crate::error::DemuxError;
    use crate::structure_data::StructureDataReader;
    use crate::structure_data::fixtures::{UNLIMITED, movie, track};

    /// Returns a reader with no limit to speak of, told of a `!mov` of 10 bytes at file offset 0 that reads back into a `moov` of 20
    #[cfg(feature = "compressed-boxes")]
    fn told_of_a_grown_movie() -> StructureDataReader {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader
            .handle_compressed_box(BoxType::compact(*b"!mov"), 0, 10, MovieBox::BOX_TYPE, 12)
            .unwrap();
        reader
    }

    #[cfg(feature = "compressed-boxes")]
    #[test]
    fn the_samples_past_a_compressed_box_lie_where_it_grew_by_and_a_segment_index_where_it_says() {
        let mut sidx = isobmff_boxes::SegmentIndexBox::default();
        let mut reference = isobmff_boxes::SegmentIndexReference::default();
        reference.referenced_size = 100;
        sidx.references.push(reference);
        let read = |mut reader: StructureDataReader| {
            let locations = reader
                .handle_movie(movie([track(1, 10, &[2, 4], &[(1, 2)], &[30])]))
                .unwrap()
                .map(|location| location.byte_range())
                .collect::<Vec<_>>();
            reader.handle_segment_index(&sidx, 40..60).unwrap();
            (locations, reader.segment_indexes().to_vec())
        };
        let (moved, indexes) = read(told_of_a_grown_movie());
        let (unmoved, unmoved_indexes) = read(StructureDataReader::new(UNLIMITED));

        assert_eq!(
            (moved, unmoved, indexes),
            (
                [20..22, 22..26].to_vec(),
                [30..32, 32..36].to_vec(),
                unmoved_indexes
            )
        );
    }

    #[cfg(feature = "compressed-boxes")]
    #[test]
    fn a_base_data_offset_after_a_resume_in_a_file_known_to_be_transformed_is_refused() {
        let mut moof = isobmff_boxes::MovieFragmentBox::default();
        let mut traf = isobmff_boxes::TrackFragmentBox::default();
        traf.tfhd.track_id = 1;
        traf.tfhd.base_data_offset = Some(0);
        moof.traf.push(traf);
        let after = |transformed: bool, resumed: bool| {
            let mut reader = StructureDataReader::new(UNLIMITED);
            if transformed {
                reader.handle_original_file_type();
            }
            if resumed {
                reader.resume();
            }
            reader.handle_movie_fragment(&moof, 0).map(|_| ())
        };

        assert_eq!(
            [after(true, true), after(true, false), after(false, true)],
            [
                Err(DemuxError::DataOffsetPastSkippedCompressedBoxes { track_id: 1 }),
                Err(DemuxError::MissingMovie),
                Err(DemuxError::MissingMovie),
            ]
        );
    }

    #[cfg(feature = "compressed-boxes")]
    #[test]
    fn a_compressed_box_and_an_mfra_are_refused_whichever_comes_first_and_leave_the_reader_as_it_was()
     {
        let mfra = isobmff_boxes::MovieFragmentRandomAccessBox::default();
        let refused = |box_type| {
            Err(DemuxError::CompressedBoxWithMovieFragmentRandomAccess {
                box_type: BoxType::compact(box_type),
            })
        };
        let mut compressed_first = told_of_a_grown_movie();
        let mut mfra_first = StructureDataReader::new(UNLIMITED);
        mfra_first
            .handle_movie_fragment_random_access(mfra.clone())
            .unwrap();

        assert_eq!(
            [
                compressed_first.handle_movie_fragment_random_access(mfra),
                mfra_first.handle_compressed_box(
                    BoxType::compact(*b"!six"),
                    100,
                    10,
                    isobmff_boxes::SegmentIndexBox::BOX_TYPE,
                    2
                ),
            ],
            [refused(*b"!mov"), refused(*b"!six")]
        );
        let mut unmoved = mfra_first
            .handle_movie(movie([track(1, 10, &[2], &[(1, 1)], &[200])]))
            .unwrap()
            .map(|location| location.byte_range());
        assert_eq!(
            (
                compressed_first.movie_fragment_random_access(),
                unmoved.next(),
                unmoved.next()
            ),
            (None, Some(200..202), None)
        );
    }
}
