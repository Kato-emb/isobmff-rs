//! Samples structure-data declares, resolved to where their media data lies
//!
//! Structure-data describes media data by reference (ISO/IEC 14496-12 A.3):
//! the `moov` declares the samples of each track in its sample tables (§8.5),
//! and each `moof` adds samples to them in track runs (§8.8). A
//! [`StructureDataReader`] takes these boxes as they come in the file and
//! resolves each sample to a [`SampleLocation`]: the values declared about
//! it, and the byte range of its media data in file offsets.

mod movie_fragment;
mod sample_table;
mod segment_index;

use alloc::vec::{self, Vec};
use core::iter::{self, FusedIterator};

use isobmff_boxes::{
    DataEntry, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SampleDescription,
    SegmentIndexBox, TrackBox,
};
use isobmff_core::BoxDefinition;

use crate::error::DemuxError;
use crate::framing::to_u64;
use crate::sample::SampleLocation;

pub use segment_index::{SegmentIndex, Subsegment, movie_fragment_random_access_start};

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
        }
    }

    /// Resolves the samples the sample tables of `movie` declare, and keeps the movie
    ///
    /// The samples of every track come out in the order their media data lies
    /// in the file, those of one chunk in sample order. The decoding time of
    /// each track starts at zero and runs on into the fragments that follow
    /// (§8.6.1.2, §8.8.12).
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](DemuxError::DuplicateBox): the reader already holds
    ///   a movie.
    /// * [`SampleCountLimitExceeded`](DemuxError::SampleCountLimitExceeded):
    ///   the sample size tables of every track together count more samples
    ///   than the limit, and none is resolved.
    /// * [`SampleTableMismatch`](DemuxError::SampleTableMismatch): the tables
    ///   of a track do not agree on its samples.
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
        for trak in &movie.trak {
            let decoding_time = sample_table::resolve_track(trak, &mut locations)?;
            decoding_times.push(Some(decoding_time));
        }
        in_file_order(&mut locations);

        self.movie = Some(movie);
        self.decoding_times = decoding_times;
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

    /// Resolves `sidx`, whose anchor point, the first byte after it, is at file offset `anchor`, and keeps the segment index
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
        anchor: u64,
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
        let segment_index = SegmentIndex::new(sidx, anchor)?;
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
    /// fragment random access box are kept.
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

/// Returns the index of the entry of the `dref` of `trak` that names where the samples sample entry `sample_description_index` describes are, once it names the file itself
///
/// # Errors
///
/// * [`UnknownSampleDescriptionIndex`](DemuxError::UnknownSampleDescriptionIndex):
///   the `stsd` has no entry `sample_description_index`, or the entry is too
///   short to hold a `data_reference_index`.
/// * [`UnknownDataReferenceIndex`](DemuxError::UnknownDataReferenceIndex): the
///   `dref` has no entry `data_reference_index`.
/// * [`ExternalDataReference`](DemuxError::ExternalDataReference): the entry
///   names a file other than the file itself (§8.7.2).
/// * [`UnsupportedDataReference`](DemuxError::UnsupportedDataReference): the
///   entry names an `IdentifiedMediaDataBox`.
fn data_reference_index(trak: &TrackBox, sample_description_index: u32) -> Result<u16, DemuxError> {
    let track_id = trak.tkhd.track_id;
    let unknown_description = DemuxError::UnknownSampleDescriptionIndex {
        track_id,
        sample_description_index,
    };
    let description = nth_from_one(
        &trak.mdia.minf.stbl.stsd.sample_descriptions,
        sample_description_index,
    )
    .ok_or(unknown_description)?;
    let data_reference_index = match description {
        SampleDescription::Typed(entry) => entry.data_reference_index,
        // Why not refuse an entry that does not read as a `SampleEntry`: every sample entry opens with six reserved bytes and its `data_reference_index` (§8.5.2.2), whatever its format.
        SampleDescription::Other(entry) => entry
            .raw_payload()
            .and_then(|payload| payload.get(6..8))
            .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
            .map(u16::from_be_bytes)
            .ok_or(unknown_description)?,
        _ => return Err(unknown_description),
    };

    match nth_from_one(
        &trak.mdia.minf.dinf.dref.data_entries,
        u32::from(data_reference_index),
    ) {
        Some(DataEntry::Url(url)) if url.location.is_none() => Ok(data_reference_index),
        Some(DataEntry::Imda(_) | DataEntry::SeqNumImda(_)) => {
            Err(DemuxError::UnsupportedDataReference {
                track_id,
                data_reference_index,
            })
        }
        Some(_) => Err(DemuxError::ExternalDataReference {
            track_id,
            data_reference_index,
        }),
        None => Err(DemuxError::UnknownDataReferenceIndex {
            track_id,
            data_reference_index,
        }),
    }
}

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

/// Returns the value of each sample a table of runs of samples of one value states, in sample order
fn expand<Value: Clone>(runs: impl Iterator<Item = (u32, Value)>) -> impl Iterator<Item = Value> {
    runs.flat_map(|(sample_count, value)| {
        iter::repeat_n(value, usize::try_from(sample_count).unwrap_or(usize::MAX))
    })
}

/// Returns entry `index` of a table that numbers its entries from 1
fn nth_from_one<Entry>(entries: &[Entry], index: u32) -> Option<&Entry> {
    entries.get(usize::try_from(index.checked_sub(1)?).ok()?)
}

/// Movies built from the fields of their boxes, which the tests of resolving share
#[cfg(test)]
pub(crate) mod fixtures {
    use alloc::vec::Vec;

    use super::StructureDataLimits;

    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, DataEntry, DataEntryUrlBox,
        MovieBox, SampleDescription, SampleEntry, SampleSizeBox, SampleSizeVariant, SampleSizes,
        SampleToChunkBox, SampleToChunkEntry, TimeToSampleBox, TimeToSampleEntry, TrackBox,
    };

    /// Returns the track `track_id` of the samples of `sizes`, each lasting `sample_delta`, laid in chunks at `chunk_offsets` by the runs `(first_chunk, samples_per_chunk)`
    ///
    /// The track has one sample entry, whose media data is in the file itself.
    pub(crate) fn track(
        track_id: u32,
        sample_delta: u32,
        sizes: &[u32],
        runs: &[(u32, u32)],
        chunk_offsets: &[u64],
    ) -> TrackBox {
        let mut trak = TrackBox::default();
        trak.tkhd.track_id = track_id;
        let stbl = &mut trak.mdia.minf.stbl;
        let mut entry = SampleEntry::default();
        entry.data_reference_index = 1;
        stbl.stsd.sample_descriptions = Vec::from([SampleDescription::Typed(entry)]);
        let mut stts = TimeToSampleBox::default();
        let mut run = TimeToSampleEntry::default();
        run.sample_count = u32::try_from(sizes.len()).unwrap();
        run.sample_delta = sample_delta;
        stts.entries = Vec::from([run]);
        stbl.stts = Some(stts);
        let mut stsc = SampleToChunkBox::default();
        stsc.entries = runs
            .iter()
            .map(|&(first_chunk, samples_per_chunk)| {
                let mut entry = SampleToChunkEntry::default();
                entry.first_chunk = first_chunk;
                entry.samples_per_chunk = samples_per_chunk;
                entry.sample_description_index = 1;
                entry
            })
            .collect();
        stbl.stsc = Some(stsc);
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Table {
            entry_sizes: sizes.to_vec(),
        };
        stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
        stbl.chunk_offset = Some(
            match chunk_offsets
                .iter()
                .map(|&offset| u32::try_from(offset))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(compact) => {
                    let mut stco = ChunkOffsetBox::default();
                    stco.chunk_offsets = compact;
                    ChunkOffsetVariant::ChunkOffset(stco)
                }
                Err(_) => {
                    let mut co64 = ChunkLargeOffsetBox::default();
                    co64.chunk_offsets = chunk_offsets.to_vec();
                    ChunkOffsetVariant::ChunkLargeOffset(co64)
                }
            },
        );
        trak.mdia.minf.dinf.dref.data_entries =
            Vec::from([DataEntry::Url(DataEntryUrlBox::default())]);
        trak
    }

    /// Limits no structure-data of the tests reaches
    pub(crate) const UNLIMITED: StructureDataLimits = StructureDataLimits {
        sample_count: u64::MAX,
        segment_index_count: u64::MAX,
        subsegment_count: u64::MAX,
    };

    /// Returns the movie of `tracks`
    pub(crate) fn movie(tracks: impl IntoIterator<Item = TrackBox>) -> MovieBox {
        let mut movie = MovieBox::default();
        movie.trak = tracks.into_iter().collect();
        movie
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::{
        CompactSampleSizeBox, CompositionOffsetBox, CompositionOffsetEntry, DataEntry,
        DataEntryImdaBox, DataEntryUrlBox, MovieBox, SampleDependencyTypeBox, SampleFlags,
        SampleSizeVariant, SampleToChunkBox, SyncSampleBox, TimeToSampleBox, TrackBox,
    };
    use isobmff_core::{BoxDecode, BoxDefinition, Utf8String};

    use super::fixtures::{UNLIMITED, movie, track};
    use super::{StructureDataLimits, StructureDataReader};
    use crate::error::DemuxError;
    use crate::sample::{SampleLocation, SampleProperties};

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
    fn every_sample_of_a_track_without_an_stss_is_a_sync_sample() {
        assert_eq!(
            resolve([track(1, 10, &[1, 1], &[(1, 2)], &[0])]),
            Ok(Vec::from([sample(0, 0..1, 0, 0), sample(10, 1..2, 0, 0)]))
        );
    }

    #[test]
    fn a_table_counting_other_samples_than_the_stsz_is_refused() {
        let mut short_stts = track(7, 10, &[1, 1], &[(1, 2)], &[0]);
        short_stts
            .mdia
            .minf
            .stbl
            .stts
            .as_mut()
            .unwrap()
            .entries
            .first_mut()
            .unwrap()
            .sample_count = 1;
        let mut long_stts = track(7, 10, &[1, 1], &[(1, 2)], &[0]);
        long_stts
            .mdia
            .minf
            .stbl
            .stts
            .as_mut()
            .unwrap()
            .entries
            .first_mut()
            .unwrap()
            .sample_count = 3;
        let chunk_past_the_samples = track(7, 10, &[1, 1], &[(1, 2)], &[0, 9]);
        let mut sync_past_the_samples = track(7, 10, &[1, 1], &[(1, 2)], &[0]);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = Vec::from([1, 3]);
        sync_past_the_samples.mdia.minf.stbl.stss = Some(stss);

        for (trak, box_type) in [
            (short_stts, TimeToSampleBox::BOX_TYPE),
            (long_stts, TimeToSampleBox::BOX_TYPE),
            (chunk_past_the_samples, SampleToChunkBox::BOX_TYPE),
            (sync_past_the_samples, SyncSampleBox::BOX_TYPE),
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
