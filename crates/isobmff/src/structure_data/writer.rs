//! [`SampleTableWriter`], which writes the sample table of a track from where its samples lie

use alloc::vec::Vec;
use core::num::NonZeroU32;

use super::Error;
use crate::SampleLocation;
use isobmff_boxes::{
    ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox,
    CompositionOffsetEntry, SampleDependencyTypeBox, SampleDependencyTypeEntry,
    SampleDescriptionBox, SampleFlags, SampleSizeBox, SampleSizeVariant, SampleSizes,
    SampleTableBox, SampleToChunkBox, SampleToChunkEntry, SyncSampleBox, TimeToSampleBox,
    TimeToSampleEntry,
};

/// What the sample table holds of a sample
#[derive(Debug, Clone, Copy)]
struct Sample {
    offset: u64,
    size: u32,
    sample_description_index: u32,
    sample_duration: u32,
    sample_composition_time_offset: i32,
    sample_flags: SampleFlags,
}

/// Writer of the `sample table` of a track, from where each of its samples lies
///
/// [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1
///
/// [`handle_location`](Self::handle_location) takes the samples of one track
/// in decoding order, and [`finish`](Self::finish) returns the `stbl` that
/// declares them, with the sample entries the caller gives. The tables are
/// the shortest the samples allow:
///
/// * `stts` (§8.6.1.2): one entry for each run of samples of one
///   `sample_duration`.
/// * `ctts` (§8.6.1.3): one entry for each run of samples of one
///   `sample_composition_time_offset`, written only where an offset is not 0,
///   in version 1 only where an offset is negative.
/// * `stsc` (§8.7.4) and `stco` or `co64` (§8.7.5): a chunk is a run of
///   samples whose bytes follow one another and that one sample entry
///   describes; `co64` only where a chunk lies past 32 bits.
/// * `stsz` (§8.7.3.2): one `sample_size` where every sample is of one size
///   other than 0.
/// * `stss` (§8.6.2): written only where a sample is not a sync sample.
/// * `sdtp` (§8.6.4): written only where a sample declares `is_leading`,
///   `sample_depends_on`, `sample_is_depended_on` or `sample_has_redundancy`.
///
/// The `stts` declares the decoding time of each sample as the sum of the
/// `sample_duration`s before it, from 0, so the writer takes a sample only at
/// that decoding time, and only of the track of the first sample. The sample
/// padding value and the degradation priority of `sample_flags` have
/// no place in these tables and are not written. Where the samples lie is up
/// to the caller; the writer does not write the other children of a `stbl`,
/// such as sample groups.
///
/// ```
/// use isobmff::structure_data::{SampleTableWriter, StructureDataReader};
/// use isobmff::demuxer::Event;
/// use isobmff::{Demuxer, DemuxerConfig};
///
/// // Read the `moov` of a file and where the samples of its first track lie
/// let file: &[u8] = include_bytes!("../../tests/fixtures/movie_moov_first.mp4");
/// let mut demuxer = Demuxer::new(DemuxerConfig::default());
/// let (mut movie, mut locations) = (None, Vec::new());
/// loop {
///     while let Some(event) = demuxer.poll_event() {
///         match event {
///             Event::Movie(moov) => movie = Some(*moov),
///             Event::SampleLocation(location) if location.properties.track_id == 1 => {
///                 locations.push(location);
///             }
///             _ => {}
///         }
///     }
///     let Some(required) = demuxer.required_input() else { break };
///     match file.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
///         _ => demuxer.finish(),
///     }
/// }
/// let movie = movie.ok_or("no moov")?;
///
/// // Write the sample table of the track from them
/// let mut writer = SampleTableWriter::new(1024);
/// for &location in &locations {
///     writer.handle_location(location)?;
/// }
/// let trak = &movie.trak[0];
/// let stbl = writer.finish(&trak.mdia.minf.stbl.stsd)?;
///
/// // The table declares the samples where they lie
/// let mut written = movie.clone();
/// written.trak = vec![trak.clone()];
/// written.trak[0].mdia.minf.stbl = stbl;
/// let resolved: Vec<_> = StructureDataReader::new(1024).handle_movie(&written)?.collect();
/// assert_eq!(resolved, locations);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct SampleTableWriter {
    sample_count_limit: u64,
    // Why keep each sample rather than the tables as they grow: a chunk ends
    // where the next sample does not follow the last, and the end of the
    // last is its offset and size, which the `stsc` and `stco` alone do not
    // hold.
    samples: Vec<Sample>,
    /// `track_ID` of the track of the samples taken, while there are any
    track_id: u32,
    // Why not sum the `sample_duration`s of the samples held: that sum on
    // every sample makes taking a track's samples quadratic in their number.
    /// Decoding time the next sample is to have
    decoding_time: u64,
}

impl SampleTableWriter {
    /// Creates a writer that takes at most `sample_count_limit` samples
    ///
    /// The writer keeps some tens of bytes for each sample until
    /// [`finish`](Self::finish). A table counts its samples in 32 bits, so
    /// the writer takes no more than `u32::MAX` whatever the limit.
    #[must_use]
    pub const fn new(sample_count_limit: u64) -> Self {
        Self {
            sample_count_limit,
            samples: Vec::new(),
            track_id: 0,
            decoding_time: 0,
        }
    }

    /// Takes the next sample of the track, in decoding order
    ///
    /// # Errors
    /// * [`Error::SampleCountOverLimit`]: the writer holds as many samples
    ///   as its limit, or as a table can count; the sample is not taken.
    /// * [`Error::OtherTrack`]: the sample is of another track than the
    ///   first sample taken; the sample is not taken.
    /// * [`Error::DecodingTimeGap`]: the decoding time of the sample is not
    ///   that of the sample before it plus its `sample_duration`, or 0 for
    ///   the first; the sample is not taken.
    /// * [`Error::Overflow`]: the bytes of the sample lie past the last
    ///   offset a file can have, or the decoding time after it past the last
    ///   a track can have; the sample is not taken.
    /// * [`Error::CompositionOffsetOutOfRange`]: the composition offset of
    ///   the sample does not fit the 32 bits of a `ctts` entry, signed; the
    ///   sample is not taken.
    pub fn handle_location(&mut self, location: SampleLocation) -> Result<(), Error> {
        let limit = self.sample_count_limit.min(u64::from(u32::MAX));
        let sample_count = u64::try_from(self.samples.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        if sample_count > limit {
            return Err(Error::SampleCountOverLimit {
                sample_count,
                limit,
            });
        }
        let properties = location.properties;
        if !self.samples.is_empty() && self.track_id != properties.track_id {
            return Err(Error::OtherTrack {
                track_id: self.track_id,
                found: properties.track_id,
            });
        }
        if properties.decoding_time != self.decoding_time {
            return Err(Error::DecodingTimeGap {
                track_id: properties.track_id,
                expected: self.decoding_time,
                found: properties.decoding_time,
            });
        }
        let overflow = Error::Overflow {
            track_id: properties.track_id,
        };
        location
            .offset
            .checked_add(u64::from(location.size))
            .ok_or(overflow)?;
        let decoding_time = self
            .decoding_time
            .checked_add(u64::from(properties.sample_duration))
            .ok_or(overflow)?;
        let sample_composition_time_offset =
            i32::try_from(properties.sample_composition_time_offset).map_err(|_| {
                Error::CompositionOffsetOutOfRange {
                    sample_composition_time_offset: properties.sample_composition_time_offset,
                }
            })?;
        self.samples.push(Sample {
            offset: location.offset,
            size: location.size,
            sample_description_index: properties.sample_description_index,
            sample_duration: properties.sample_duration,
            sample_composition_time_offset,
            sample_flags: properties.sample_flags,
        });
        self.track_id = properties.track_id;
        self.decoding_time = decoding_time;
        Ok(())
    }

    /// Returns the `stbl` of the samples taken, with `stsd` as its sample entries, and starts anew
    ///
    /// The writer then holds no sample, and takes those of another track. A
    /// call that fails leaves the writer as it was.
    ///
    /// # Errors
    /// * [`Error::SampleDescriptionIndexOutOfRange`]: a sample names a sample
    ///   entry `stsd` does not hold.
    pub fn finish(&mut self, stsd: &SampleDescriptionBox) -> Result<SampleTableBox, Error> {
        if let Some(sample) = self.samples.iter().find(|sample| {
            usize::try_from(sample.sample_description_index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| stsd.sample_descriptions.get(index))
                .is_none()
        }) {
            return Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index: sample.sample_description_index,
            });
        }
        let samples = self.samples.as_slice();
        let count = |run: &[Sample]| u32::try_from(run.len()).unwrap_or(u32::MAX);

        let mut stts = TimeToSampleBox::default();
        stts.entries = samples
            .chunk_by(|earlier, later| earlier.sample_duration == later.sample_duration)
            .map(|run| {
                let mut entry = TimeToSampleEntry::default();
                entry.sample_count = count(run);
                entry.sample_delta = run.first().map_or(0, |sample| sample.sample_duration);
                entry
            })
            .collect();

        let mut ctts = CompositionOffsetBox::default();
        ctts.entries = samples
            .chunk_by(|earlier, later| {
                earlier.sample_composition_time_offset == later.sample_composition_time_offset
            })
            .map(|run| {
                let mut entry = CompositionOffsetEntry::default();
                entry.sample_count = count(run);
                entry.sample_offset = run
                    .first()
                    .map_or(0, |sample| i64::from(sample.sample_composition_time_offset));
                entry
            })
            .collect();
        ctts.version = u8::from(ctts.entries.iter().any(|entry| entry.sample_offset < 0));

        let chunks: Vec<&[Sample]> = samples
            .chunk_by(|earlier, later| {
                earlier.sample_description_index == later.sample_description_index
                    && earlier.offset.checked_add(u64::from(earlier.size)) == Some(later.offset)
            })
            .collect();
        let mut stsc = SampleToChunkBox::default();
        stsc.entries = (1_u32..)
            .zip(&chunks)
            .map(|(first_chunk, chunk)| {
                let mut entry = SampleToChunkEntry::default();
                entry.first_chunk = first_chunk;
                entry.samples_per_chunk = count(chunk);
                entry.sample_description_index = chunk
                    .first()
                    .map_or(0, |sample| sample.sample_description_index);
                entry
            })
            .collect();
        stsc.entries.dedup_by(|later, earlier| {
            later.samples_per_chunk == earlier.samples_per_chunk
                && later.sample_description_index == earlier.sample_description_index
        });
        let offsets = chunks
            .iter()
            .map(|chunk| chunk.first().map_or(0, |sample| sample.offset));
        let chunk_offset = match offsets.clone().map(u32::try_from).collect() {
            Ok(compact) => {
                let mut stco = ChunkOffsetBox::default();
                stco.chunk_offsets = compact;
                ChunkOffsetVariant::ChunkOffset(stco)
            }
            Err(_) => {
                let mut co64 = ChunkLargeOffsetBox::default();
                co64.chunk_offsets = offsets.collect();
                ChunkOffsetVariant::ChunkLargeOffset(co64)
            }
        };

        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = match samples
            .first()
            .and_then(|sample| NonZeroU32::new(sample.size))
        {
            Some(sample_size)
                if samples
                    .iter()
                    .all(|sample| sample.size == sample_size.get()) =>
            {
                SampleSizes::Constant {
                    sample_size,
                    sample_count: count(samples),
                }
            }
            _ => SampleSizes::Table {
                entry_sizes: samples.iter().map(|sample| sample.size).collect(),
            },
        };

        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = (1_u32..)
            .zip(samples)
            .filter(|(_, sample)| !sample.sample_flags.sample_is_non_sync_sample)
            .map(|(sample_number, _)| sample_number)
            .collect();

        let mut sdtp = SampleDependencyTypeBox::default();
        sdtp.entries = samples
            .iter()
            .map(|sample| {
                let flags = sample.sample_flags;
                let mut entry = SampleDependencyTypeEntry::default();
                entry.is_leading = flags.is_leading;
                entry.sample_depends_on = flags.sample_depends_on;
                entry.sample_is_depended_on = flags.sample_is_depended_on;
                entry.sample_has_redundancy = flags.sample_has_redundancy;
                entry
            })
            .collect();

        let mut stbl = SampleTableBox::default();
        stbl.stsd = stsd.clone();
        stbl.stts = Some(stts);
        stbl.ctts = ctts
            .entries
            .iter()
            .any(|entry| entry.sample_offset != 0)
            .then_some(ctts);
        stbl.stsc = Some(stsc);
        stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
        stbl.chunk_offset = Some(chunk_offset);
        stbl.stss = (stss.sample_numbers.len() != samples.len()).then_some(stss);
        stbl.sdtp = sdtp
            .entries
            .iter()
            .any(|entry| *entry != SampleDependencyTypeEntry::default())
            .then_some(sdtp);

        self.samples.clear();
        *self = Self {
            samples: core::mem::take(&mut self.samples),
            ..Self::new(self.sample_count_limit)
        };
        Ok(stbl)
    }
}
