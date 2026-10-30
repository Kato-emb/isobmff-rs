//! [`StructureDataReader`], which reads the structure-data of a file into the samples it declares

use alloc::vec::{self, Vec};
use core::iter::FusedIterator;
use core::ops::Range;

use isobmff_boxes::{MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox};
use isobmff_core::BoxDefinition;

use super::segment_index::SegmentIndex;
use super::{movie_fragment, sample_table};
use crate::error::DemuxError;
use crate::framing::to_u64;
use crate::leniency::Leniency;
use crate::sample::SampleLocation;

/// Limits a [`StructureDataReader`] resolves and keeps structure-data up to
///
/// Each bounds what the reader allocates for values the file declares: the
/// samples of one `moov` or `moof`, and the segment indexes, with their
/// subsegments, the reader keeps over the whole file. A subsegment takes
/// the same room whatever the file, so its count bounds its bytes.
///
/// # Examples
///
/// ```
/// use isobmff::structure_data::StructureDataLimits;
///
/// let limits = StructureDataLimits {
///     sample_count: 1_048_576,
///     segment_index_count: 1024,
///     subsegment_count: 1_048_576,
/// };
/// assert_eq!(limits.segment_index_count, 1024);
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "a limit added later is one every caller has to set, not one that quietly takes a default"
)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StructureDataLimits {
    /// Number of samples one `moov`, or one `moof`, is resolved up to
    pub sample_count: u64,
    /// Number of segment indexes the reader keeps up to
    pub segment_index_count: u64,
    /// Number of subsegments the segment indexes the reader keeps hold up to, together
    pub subsegment_count: u64,
}

/// Reader of the structure-data of a file: the movie and its fragments, resolved to the samples they declare
///
/// The reader keeps what structure-data declares: the movie, whose tracks the
/// fragments extend, the decoding time each track has reached (§8.8.12), the
/// segment indexes, and the movie fragment random access box.
///
/// A refused call leaves the reader as it was.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::MovieBox;
/// use isobmff::structure_data::{StructureDataLimits, StructureDataReader};
///
/// let mut reader = StructureDataReader::new(StructureDataLimits {
///     sample_count: 1_048_576,
///     segment_index_count: 1024,
///     subsegment_count: 1_048_576,
/// });
/// // A movie of no track declares no sample.
/// let samples = reader.handle_movie(MovieBox::default())?;
/// assert_eq!(samples.len(), 0);
/// assert!(reader.movie().is_some());
/// # Ok::<(), isobmff::DemuxError>(())
/// ```
#[derive(Debug)]
pub struct StructureDataReader {
    limits: StructureDataLimits,
    movie: Option<MovieBox>,
    /// Decoding time past the last sample of each track of the movie, in the order of its tracks, `None` once it is not known
    decoding_times: Vec<Option<u64>>,
    segment_indexes: Vec<SegmentIndex>,
    movie_fragment_random_access: Option<MovieFragmentRandomAccessBox>,
    leniencies: Vec<Leniency>,
}

impl StructureDataReader {
    /// Creates the reader of a file whose structure-data it resolves and keeps up to `limits`
    #[must_use]
    pub const fn new(limits: StructureDataLimits) -> Self {
        Self {
            limits,
            movie: None,
            decoding_times: Vec::new(),
            segment_indexes: Vec::new(),
            movie_fragment_random_access: None,
            leniencies: Vec::new(),
        }
    }

    /// Resolves the samples the sample tables of `movie` declare, and keeps the movie
    ///
    /// The samples of every track come out in the order their media data lies
    /// in the file, those of one chunk in sample order. The decoding time of
    /// each track starts at zero and runs on into the fragments that follow
    /// (§8.6.1.2, §8.8.12).
    ///
    /// A table that states another number of samples than its track has is
    /// read past, as FFmpeg does, and kept as a [`Leniency::SampleTable`]
    /// that [`leniencies`](Self::leniencies) returns: the track has the
    /// samples its chunks lay out, as far as its `stsz` or `stz2` gives them
    /// sizes; the samples past the end of a short `stts` take its last
    /// duration, those past the end of another short table take zero; and
    /// the entries of a table past the last sample are left out, as are the
    /// sample numbers the `stss` marks out of order.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](DemuxError::DuplicateBox): the reader already holds
    ///   a movie.
    /// * [`SampleCountLimitExceeded`](DemuxError::SampleCountLimitExceeded):
    ///   the sample size tables of every track together count more samples
    ///   than the limit, and none is resolved.
    /// * [`SampleTableMismatch`](DemuxError::SampleTableMismatch): the tables
    ///   of a track cannot lay out its samples: the track declares samples,
    ///   but its `stsz` or `stz2`, its `stco` or `co64`, its `stsc` or its
    ///   `stts` states none; or its `stsc` lays its runs over chunks it does
    ///   not have.
    /// * [`UnknownSampleDescriptionIndex`](DemuxError::UnknownSampleDescriptionIndex),
    ///   [`UnknownDataReferenceIndex`](DemuxError::UnknownDataReferenceIndex):
    ///   samples are described by a sample entry, or a sample entry names a
    ///   data reference entry, that the track has none of.
    /// * [`ExternalDataReference`](DemuxError::ExternalDataReference),
    ///   [`UnsupportedDataReference`](DemuxError::UnsupportedDataReference):
    ///   the media data of samples is in another file, or in an
    ///   `IdentifiedMediaDataBox`.
    /// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange),
    ///   [`DecodingTimeOverflow`](DemuxError::DecodingTimeOverflow): the media
    ///   data or the decoding times of a track run past 64 bits.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<SampleLocations, DemuxError> {
        if self.movie.is_some() {
            return Err(DemuxError::DuplicateBox {
                box_type: MovieBox::BOX_TYPE,
            });
        }
        let declared_samples = movie
            .trak
            .iter()
            .map(sample_table::sample_count)
            .fold(0, u64::saturating_add);
        let mut locations = self.room_for(declared_samples)?;
        let mut decoding_times = Vec::with_capacity(movie.trak.len());
        let mut leniencies = Vec::new();
        for trak in &movie.trak {
            let decoding_time = sample_table::resolve_track(trak, &mut locations, &mut leniencies)?;
            decoding_times.push(Some(decoding_time));
        }
        in_file_order(&mut locations);

        self.movie = Some(movie);
        self.decoding_times = decoding_times;
        self.leniencies = leniencies;
        Ok(SampleLocations {
            locations: locations.into_iter(),
        })
    }

    /// Resolves the samples the track fragments of `movie_fragment`, whose `moof` starts at file offset `moof_offset`, declare
    ///
    /// The samples come out in the order their media data lies in the file.
    /// A track fragment without a `tfdt` takes up the decoding time its track
    /// has reached in the movie and the fragments before it (§8.8.12), and
    /// the fragment moves that time on. The data offsets resolve to file
    /// offsets as Table 6 of §8.8.7.1 sets out.
    ///
    /// # Errors
    ///
    /// * [`MissingMovie`](DemuxError::MissingMovie): the reader holds no
    ///   movie.
    /// * [`SampleCountLimitExceeded`](DemuxError::SampleCountLimitExceeded):
    ///   the track runs together count more samples than the limit, and none
    ///   is resolved.
    /// * [`UnknownTrackId`](DemuxError::UnknownTrackId),
    ///   [`MissingTrackExtends`](DemuxError::MissingTrackExtends): a track
    ///   fragment is of a track the movie, or its `mvex`, has none of.
    /// * [`MissingDecodingTime`](DemuxError::MissingDecodingTime): a track
    ///   fragment has no `tfdt`, and the reader has resumed since its track
    ///   last had one.
    /// * The failures of the sample entries and the data references, and of
    ///   offsets and decoding times past 64 bits, as for
    ///   [`handle_movie`](Self::handle_movie).
    pub fn handle_movie_fragment(
        &mut self,
        movie_fragment: &MovieFragmentBox,
        moof_offset: u64,
    ) -> Result<SampleLocations, DemuxError> {
        let movie = self.movie.as_ref().ok_or(DemuxError::MissingMovie)?;
        let mut locations = self.room_for(movie_fragment::sample_count(movie_fragment))?;
        let mut decoding_times = self.decoding_times.clone();
        movie_fragment::resolve_fragment(
            movie,
            &mut decoding_times,
            movie_fragment,
            moof_offset,
            &mut locations,
        )?;
        in_file_order(&mut locations);

        self.decoding_times = decoding_times;
        Ok(SampleLocations {
            locations: locations.into_iter(),
        })
    }

    /// Resolves `sidx`, which lies at the file offsets `byte_range`, its anchor point the first byte after it, and keeps the segment index
    ///
    /// A segment index the reader already keeps, as it does after resuming
    /// before the `sidx`, is kept once, and counts once toward the limits.
    ///
    /// # Errors
    ///
    /// * [`SubsegmentCountLimitExceeded`](DemuxError::SubsegmentCountLimitExceeded):
    ///   the `sidx` references more subsegments than the limit, and none is
    ///   resolved; or keeping its index would take the subsegments of the
    ///   kept indexes past the limit.
    /// * [`SegmentIndexCountLimitExceeded`](DemuxError::SegmentIndexCountLimitExceeded):
    ///   keeping the index would take the kept indexes past the limit.
    /// * [`SegmentIndexOutOfRange`](DemuxError::SegmentIndexOutOfRange): the
    ///   byte ranges or the presentation times run past 64 bits.
    pub fn handle_segment_index(
        &mut self,
        sidx: &SegmentIndexBox,
        byte_range: Range<u64>,
    ) -> Result<(), DemuxError> {
        let subsegments_exceeded =
            |retained_subsegments| DemuxError::SubsegmentCountLimitExceeded {
                retained_subsegments,
                limit_subsegments: self.limits.subsegment_count,
            };
        let declared_subsegments = to_u64(sidx.references.len());
        if declared_subsegments > self.limits.subsegment_count {
            return Err(subsegments_exceeded(declared_subsegments));
        }
        let segment_index = SegmentIndex::new(sidx, byte_range)?;
        if self.segment_indexes.contains(&segment_index) {
            return Ok(());
        }

        let retained_segment_indexes = to_u64(self.segment_indexes.len()).saturating_add(1);
        if retained_segment_indexes > self.limits.segment_index_count {
            return Err(DemuxError::SegmentIndexCountLimitExceeded {
                retained_segment_indexes,
                limit_segment_indexes: self.limits.segment_index_count,
            });
        }
        let retained_subsegments = self
            .segment_indexes
            .iter()
            .map(|kept| to_u64(kept.subsegments().len()))
            .fold(declared_subsegments, u64::saturating_add);
        if retained_subsegments > self.limits.subsegment_count {
            return Err(subsegments_exceeded(retained_subsegments));
        }
        self.segment_indexes.push(segment_index);

        Ok(())
    }

    /// Keeps `mfra`, in place of one the reader kept before
    pub fn handle_movie_fragment_random_access(&mut self, mfra: MovieFragmentRandomAccessBox) {
        self.movie_fragment_random_access = Some(mfra);
    }

    /// Forgets the decoding time each track has reached, as a reader does that resumes reading at a later box of the file
    ///
    /// The fragments read after it resolve only where their `tfdt` gives the
    /// decoding time (§8.8.12). The movie, the segment indexes and the movie
    /// fragment random access box are kept, and so are the leniencies.
    pub fn resume(&mut self) {
        self.decoding_times.fill(None);
    }

    /// Returns the movie the reader holds, once one is read
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.movie.as_ref()
    }

    /// Returns the segment indexes the reader keeps, in the order their `sidx`es were read
    #[must_use]
    pub fn segment_indexes(&self) -> &[SegmentIndex] {
        &self.segment_indexes
    }

    /// Returns the movie fragment random access box the reader keeps, the last one read
    #[must_use]
    pub const fn movie_fragment_random_access(&self) -> Option<&MovieFragmentRandomAccessBox> {
        self.movie_fragment_random_access.as_ref()
    }

    /// Returns what the reader has read past in the structure-data, in the order it read it
    #[must_use]
    pub fn leniencies(&self) -> &[Leniency] {
        &self.leniencies
    }

    /// Returns room for the locations of `declared_samples` samples, or the failure of more than the limit
    fn room_for(&self, declared_samples: u64) -> Result<Vec<SampleLocation>, DemuxError> {
        if declared_samples > self.limits.sample_count {
            return Err(DemuxError::SampleCountLimitExceeded {
                declared_samples,
                limit_samples: self.limits.sample_count,
            });
        }

        Ok(Vec::with_capacity(
            usize::try_from(declared_samples).unwrap_or_default(),
        ))
    }
}

/// Locations of the samples a `moov` or a `moof` declares, in the order their media data lies in the file
///
/// The iterator owns the locations: it borrows nothing of the reader that
/// resolved them.
#[derive(Clone, Debug)]
pub struct SampleLocations {
    locations: vec::IntoIter<SampleLocation>,
}

impl Iterator for SampleLocations {
    type Item = SampleLocation;

    fn next(&mut self) -> Option<SampleLocation> {
        self.locations.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.locations.size_hint()
    }
}

impl ExactSizeIterator for SampleLocations {}

impl FusedIterator for SampleLocations {}

/// Sorts `locations` into the order their media data lies in the file, samples of no bytes at one offset by track and decoding time
fn in_file_order(locations: &mut [SampleLocation]) {
    // Why not a stable sort by offset alone: it allocates scratch in proportion to the locations, which the sample limit lets grow large.
    locations.sort_unstable_by_key(|location| {
        let properties = location.properties();
        (
            location.byte_range().start,
            properties.track_id,
            properties.decoding_time,
        )
    });
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::{
        ChunkOffsetBox, CompactSampleSizeBox, CompositionOffsetBox, CompositionOffsetEntry,
        DataEntry, DataEntryImdaBox, DataEntryUrlBox, DegradationPriorityBox, MovieBox,
        PaddingBitsBox, SampleDependencyTypeBox, SampleFlags, SampleSizeBox, SampleSizeVariant,
        SampleToChunkBox, SyncSampleBox, TimeToSampleBox, TimeToSampleEntry, TrackBox,
    };
    use isobmff_core::{BoxDecode, BoxDefinition, BoxType, Utf8String};

    use super::{StructureDataLimits, StructureDataReader};
    use crate::error::DemuxError;
    use crate::leniency::{Leniency, SampleTableDisagreement};
    use crate::sample::{SampleLocation, SampleProperties};
    use crate::structure_data::fixtures::{UNLIMITED, movie, track};

    /// Resolves the movie of `tracks` with no limit to speak of
    fn resolve(
        tracks: impl IntoIterator<Item = TrackBox>,
    ) -> Result<Vec<SampleLocation>, DemuxError> {
        StructureDataReader::new(UNLIMITED)
            .handle_movie(movie(tracks))
            .map(Iterator::collect)
    }

    /// Returns the location of a sample of 10 ticks of track 1, described by sample entry 1 and in the file itself
    fn sample(
        decoding_time: u64,
        byte_range: Range<u64>,
        sample_composition_time_offset: i64,
        sample_flags: u32,
    ) -> SampleLocation {
        SampleLocation::new(
            SampleProperties {
                track_id: 1,
                decoding_time,
                sample_duration: 10,
                sample_composition_time_offset,
                sample_flags: SampleFlags::from_bits(sample_flags),
                sample_description_index: 1,
            },
            1,
            byte_range,
        )
    }

    /// Returns the track, decoding time and byte range of each location
    fn placed(locations: &[SampleLocation]) -> Vec<(u32, u64, Range<u64>)> {
        locations
            .iter()
            .map(|location| {
                (
                    location.properties().track_id,
                    location.properties().decoding_time,
                    location.byte_range(),
                )
            })
            .collect()
    }

    #[test]
    fn the_samples_of_a_chunk_lie_one_after_another_from_its_offset() {
        // Chunk 1 at 100 holds samples 1 and 2, chunk 2 at 200 holds sample 3.
        let locations = resolve([track(1, 10, &[5, 6, 7], &[(1, 2), (2, 1)], &[100, 200])]);

        assert_eq!(
            placed(&locations.unwrap()),
            [(1, 0, 100..105), (1, 10, 105..111), (1, 20, 200..207)]
        );
    }

    #[test]
    fn the_samples_of_two_tracks_come_out_in_the_order_their_media_data_lies() {
        let audio = track(1, 1024, &[4, 4, 4, 4], &[(1, 2)], &[100, 300]);
        let video = track(2, 3000, &[50], &[(1, 1)], &[200]);

        assert_eq!(
            placed(&resolve([audio, video]).unwrap()),
            [
                (1, 0, 100..104),
                (1, 1024, 104..108),
                (2, 0, 200..250),
                (1, 2048, 300..304),
                (1, 3072, 304..308),
            ]
        );
    }

    #[test]
    fn an_stz2_and_a_co64_read_as_an_stsz_and_an_stco() {
        let mut trak = track(1, 10, &[3, 4], &[(1, 2)], &[0x1_0000_0000]);
        let stz2 =
            CompactSampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\x08\0\0\0\x02\x03\x04").unwrap();
        trak.mdia.minf.stbl.sample_size = Some(SampleSizeVariant::CompactSampleSize(stz2));

        assert_eq!(
            placed(&resolve([trak]).unwrap()),
            [
                (1, 0, 0x1_0000_0000..0x1_0000_0003),
                (1, 10, 0x1_0000_0003..0x1_0000_0007)
            ]
        );
    }

    #[test]
    fn the_composition_offsets_of_a_ctts_of_version_1_may_be_negative() {
        let mut trak = track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut ctts = CompositionOffsetBox::default();
        ctts.version = 1;
        ctts.entries = [(1, 20), (2, -10)]
            .map(|(sample_count, sample_offset)| {
                let mut entry = CompositionOffsetEntry::default();
                entry.sample_count = sample_count;
                entry.sample_offset = sample_offset;
                entry
            })
            .to_vec();
        trak.mdia.minf.stbl.ctts = Some(ctts);

        assert_eq!(
            resolve([trak]),
            Ok(Vec::from([
                sample(0, 0..1, 20, 0),
                sample(10, 1..2, -10, 0),
                sample(20, 2..3, -10, 0),
            ]))
        );
    }

    #[test]
    fn the_stss_and_the_sdtp_make_up_the_sample_flags() {
        let mut trak = track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = Vec::from([1, 3]);
        trak.mdia.minf.stbl.stss = Some(stss);
        // is_leading 0, sample_depends_on 2, then 1 and 1, sample_is_depended_on 1, 2, 1
        trak.mdia.minf.stbl.sdtp =
            Some(SampleDependencyTypeBox::decode_payload(b"\0\0\0\0\x24\x18\x14").unwrap());

        assert_eq!(
            resolve([trak]),
            Ok(Vec::from([
                sample(0, 0..1, 0, 0x0240_0000),
                sample(10, 1..2, 0, 0x0181_0000),
                sample(20, 2..3, 0, 0x0140_0000),
            ]))
        );
    }

    #[test]
    fn the_padb_and_the_stdp_make_up_the_sample_flags() {
        let mut trak = track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        // Three samples padded with 3, 5 and 6 bits, the second half of the last byte unused
        trak.mdia.minf.stbl.padb =
            Some(PaddingBitsBox::decode_payload(b"\0\0\0\0\0\0\0\x03\x35\x60").unwrap());
        trak.mdia.minf.stbl.stdp =
            Some(DegradationPriorityBox::decode_payload(b"\0\0\0\0\0\0\0\x07\x01\0").unwrap());

        assert_eq!(
            resolve([trak]),
            Ok(Vec::from([
                sample(0, 0..1, 0, 0x0006_0000),
                sample(10, 1..2, 0, 0x000a_0007),
                sample(20, 2..3, 0, 0x000c_0100),
            ]))
        );
    }

    #[test]
    fn every_sample_of_a_track_without_an_stss_is_a_sync_sample() {
        assert_eq!(
            resolve([track(1, 10, &[1, 1], &[(1, 2)], &[0])]),
            Ok(Vec::from([sample(0, 0..1, 0, 0), sample(10, 1..2, 0, 0)]))
        );
    }

    /// Decoding time, duration, composition time offset, `sample_flags` bits and byte range of a sample
    type Timed = (u64, u32, i64, u32, Range<u64>);

    /// Returns how each location is timed
    fn timed(locations: &[SampleLocation]) -> Vec<Timed> {
        locations
            .iter()
            .map(|location| {
                let properties = location.properties();
                (
                    properties.decoding_time,
                    properties.sample_duration,
                    properties.sample_composition_time_offset,
                    properties.sample_flags.to_bits().unwrap(),
                    location.byte_range(),
                )
            })
            .collect()
    }

    /// Resolves the movie of `tracks`, and returns how each sample is timed and what the reader read past
    fn resolve_leniently(
        tracks: impl IntoIterator<Item = TrackBox>,
    ) -> (Vec<Timed>, Vec<Leniency>) {
        let mut reader = StructureDataReader::new(UNLIMITED);
        let locations: Vec<_> = reader.handle_movie(movie(tracks)).unwrap().collect();
        (timed(&locations), reader.leniencies().to_vec())
    }

    /// Returns the leniency of the table `box_type` of track 7 of `kind`
    fn read_past(box_type: &[u8; 4], kind: SampleTableDisagreement) -> Leniency {
        Leniency::SampleTable {
            track_id: 7,
            box_type: BoxType::compact(*box_type),
            kind,
        }
    }

    #[test]
    fn the_samples_past_a_short_stts_take_its_last_duration() {
        let mut trak = track(7, 0, &[1, 1, 1], &[(1, 3)], &[0]);
        if let Some(stts) = trak.mdia.minf.stbl.stts.as_mut() {
            stts.entries = [(1, 10), (1, 20)]
                .map(|(sample_count, sample_delta)| {
                    let mut entry = TimeToSampleEntry::default();
                    entry.sample_count = sample_count;
                    entry.sample_delta = sample_delta;
                    entry
                })
                .to_vec();
        }

        assert_eq!(
            resolve_leniently([trak]),
            (
                Vec::from([
                    (0, 10, 0, 0, 0..1),
                    (10, 20, 0, 0, 1..2),
                    (30, 20, 0, 0, 2..3)
                ]),
                Vec::from([read_past(
                    b"stts",
                    SampleTableDisagreement::Short { missing_samples: 1 }
                )])
            )
        );
    }

    #[test]
    fn the_samples_a_track_has_are_those_its_chunks_lay_out_as_far_as_its_stsz_gives_sizes() {
        // Chunk 2 has room for two more samples than the stsz gives sizes for
        let chunk_past_the_samples = track(7, 10, &[1, 1], &[(1, 2)], &[0, 9]);
        // The stsz and the stts state a third sample no chunk lays out
        let samples_past_the_chunks = track(7, 10, &[1, 1, 1], &[(1, 2)], &[0]);
        let first_two = Vec::from([(0, 10, 0, 0, 0..1), (10, 10, 0, 0, 1..2)]);

        assert_eq!(
            [
                resolve_leniently([chunk_past_the_samples]),
                resolve_leniently([samples_past_the_chunks]),
            ],
            [
                (
                    first_two.clone(),
                    Vec::from([read_past(
                        b"stsc",
                        SampleTableDisagreement::Long { excess_samples: 2 }
                    )])
                ),
                (
                    first_two,
                    Vec::from([
                        read_past(b"stsz", SampleTableDisagreement::Long { excess_samples: 1 }),
                        read_past(b"stts", SampleTableDisagreement::Long { excess_samples: 1 }),
                    ])
                ),
            ]
        );
    }

    #[test]
    fn the_samples_past_another_short_table_take_zero_and_the_entries_past_the_last_sample_are_left_out()
     {
        let mut trak = track(7, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let stbl = &mut trak.mdia.minf.stbl;
        let mut ctts = CompositionOffsetBox::default();
        let mut entry = CompositionOffsetEntry::default();
        entry.sample_count = 1;
        entry.sample_offset = 5;
        ctts.entries = Vec::from([entry]);
        stbl.ctts = Some(ctts);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = Vec::from([1, 4, 5]);
        stbl.stss = Some(stss);
        let mut padb = PaddingBitsBox::default();
        padb.pads = Vec::from([3]);
        stbl.padb = Some(padb);
        let mut stdp = DegradationPriorityBox::default();
        stdp.priorities = Vec::from([1, 2, 3, 4]);
        stbl.stdp = Some(stdp);

        assert_eq!(
            resolve_leniently([trak]),
            (
                Vec::from([
                    (0, 10, 5, 0x0006_0001, 0..1),
                    (10, 10, 0, 0x0001_0002, 1..2),
                    (20, 10, 0, 0x0001_0003, 2..3),
                ]),
                Vec::from([
                    read_past(
                        b"ctts",
                        SampleTableDisagreement::Short { missing_samples: 2 }
                    ),
                    read_past(
                        b"padb",
                        SampleTableDisagreement::Short { missing_samples: 2 }
                    ),
                    read_past(b"stdp", SampleTableDisagreement::Long { excess_samples: 1 }),
                    read_past(b"stss", SampleTableDisagreement::Long { excess_samples: 2 }),
                ])
            )
        );
    }

    #[test]
    fn the_sample_numbers_an_stss_marks_out_of_order_are_passed_over() {
        let mut trak = track(7, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = Vec::from([2, 1, 3]);
        trak.mdia.minf.stbl.stss = Some(stss);

        assert_eq!(
            resolve_leniently([trak]),
            (
                Vec::from([
                    (0, 10, 0, 0x0001_0000, 0..1),
                    (10, 10, 0, 0, 1..2),
                    (20, 10, 0, 0, 2..3),
                ]),
                Vec::from([read_past(
                    b"stss",
                    SampleTableDisagreement::Unordered { sample_numbers: 1 }
                )])
            )
        );
    }

    #[test]
    fn a_track_whose_tables_cannot_lay_out_its_samples_is_refused() {
        let without = |box_type: &[u8; 4]| {
            let mut trak = track(7, 10, &[1, 1], &[(1, 2)], &[0]);
            let stbl = &mut trak.mdia.minf.stbl;
            match box_type {
                b"stsz" => stbl.sample_size = None,
                b"stco" => stbl.chunk_offset = None,
                b"stsc" => stbl.stsc = None,
                _ => stbl.stts = None,
            }
            trak
        };
        let runs_past_the_chunks = track(7, 10, &[1, 1], &[(1, 1), (3, 1)], &[0, 1]);

        for (trak, box_type) in [
            (without(b"stsz"), SampleSizeBox::BOX_TYPE),
            (without(b"stco"), ChunkOffsetBox::BOX_TYPE),
            (without(b"stsc"), SampleToChunkBox::BOX_TYPE),
            (without(b"stts"), TimeToSampleBox::BOX_TYPE),
            (runs_past_the_chunks, SampleToChunkBox::BOX_TYPE),
        ] {
            assert_eq!(
                resolve([trak]),
                Err(DemuxError::SampleTableMismatch {
                    track_id: 7,
                    box_type
                })
            );
        }
    }

    #[test]
    fn a_track_of_no_sample_is_read_without_a_leniency() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        let locations = reader.handle_movie(movie([track(7, 0, &[], &[], &[])]));

        assert_eq!(
            (locations.map(Iterator::count), reader.leniencies()),
            (Ok(0), &[][..])
        );
    }

    #[test]
    fn media_data_in_another_file_or_an_imda_is_refused() {
        let mut external = track(3, 10, &[1], &[(1, 1)], &[0]);
        let mut url = DataEntryUrlBox::default();
        url.location = Some(Utf8String::from_slice(b"other.mp4"));
        external.mdia.minf.dinf.dref.data_entries = Vec::from([DataEntry::Url(url)]);
        let mut imda = track(3, 10, &[1], &[(1, 1)], &[0]);
        imda.mdia.minf.dinf.dref.data_entries =
            Vec::from([DataEntry::Imda(DataEntryImdaBox::default())]);
        let mut unknown = track(3, 10, &[1], &[(1, 1)], &[0]);
        unknown.mdia.minf.dinf.dref.data_entries.clear();

        assert_eq!(
            resolve([external]),
            Err(DemuxError::ExternalDataReference {
                track_id: 3,
                data_reference_index: 1
            })
        );
        assert_eq!(
            resolve([imda]),
            Err(DemuxError::UnsupportedDataReference {
                track_id: 3,
                data_reference_index: 1
            })
        );
        assert_eq!(
            resolve([unknown]),
            Err(DemuxError::UnknownDataReferenceIndex {
                track_id: 3,
                data_reference_index: 1
            })
        );
    }

    #[test]
    fn a_movie_of_more_samples_than_the_limit_is_refused_and_not_kept() {
        let mut reader = StructureDataReader::new(StructureDataLimits {
            sample_count: 3,
            ..UNLIMITED
        });
        let tracks = [
            track(1, 10, &[1, 1], &[(1, 2)], &[0]),
            track(2, 10, &[1, 1], &[(1, 2)], &[2]),
        ];

        assert_eq!(
            reader.handle_movie(movie(tracks)).map(|_| ()),
            Err(DemuxError::SampleCountLimitExceeded {
                declared_samples: 4,
                limit_samples: 3
            })
        );
        assert!(reader.movie().is_none());
    }

    #[test]
    fn a_second_movie_is_refused() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader.handle_movie(movie([])).unwrap();

        assert_eq!(
            reader.handle_movie(movie([])).map(|_| ()),
            Err(DemuxError::DuplicateBox {
                box_type: MovieBox::BOX_TYPE
            })
        );
    }
}
