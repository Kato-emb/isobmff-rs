//! The samples the sample table of a track declares, where their bytes lie, and how long they last

use alloc::vec::Vec;
use core::iter;

use isobmff_boxes::{
    ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox, SampleDependencyTypeBox,
    SampleDescriptionBox, SampleFlags, SampleSizeBox, SampleSizeVariant, SampleSizes,
    SampleTableBox, SampleToChunkBox, SyncSampleBox, TimeToSampleBox, TrackBox,
};
use isobmff_core::{BoxDefinition, BoxType};

use super::Error;
use super::reader::Reading;
use super::sample::is_in_same_file;
use crate::{SampleLocation, SampleProperties};

/// The size of each sample, as the sample size table of a track gives it
enum Sizes<'a> {
    Constant(u32),
    Table(&'a [u32]),
}

impl Sizes<'_> {
    fn get(&self, index: usize) -> Option<u32> {
        match self {
            Self::Constant(size) => Some(*size),
            Self::Table(sizes) => sizes.get(index).copied(),
        }
    }
}

/// Returns the number of samples the sample size table of `track` declares, and their sizes
fn sizes(track: &TrackBox) -> Result<(u64, Sizes<'_>), Error> {
    let track_id = track.tkhd.track_id;
    match &track.mdia.minf.stbl.sample_size {
        Some(SampleSizeVariant::SampleSize(stsz)) => match &stsz.sample_sizes {
            SampleSizes::Constant {
                sample_size,
                sample_count,
            } => Ok((u64::from(*sample_count), Sizes::Constant(sample_size.get()))),
            SampleSizes::Table { entry_sizes } => {
                Ok((length_of(entry_sizes.len()), Sizes::Table(entry_sizes)))
            }
            _ => Err(Error::InvalidEntry {
                track_id,
                box_type: SampleSizeBox::BOX_TYPE,
                entry: 0,
            }),
        },
        Some(SampleSizeVariant::CompactSampleSize(stz2)) => Ok((
            length_of(stz2.entry_sizes.len()),
            Sizes::Table(&stz2.entry_sizes),
        )),
        Some(_) | None => Err(Error::MissingTable {
            track_id,
            box_type: SampleSizeBox::BOX_TYPE,
        }),
    }
}

/// Returns the number of samples the sample size table of `track` declares
pub(super) fn sample_count(track: &TrackBox) -> Result<u64, Error> {
    sizes(track).map(|(sample_count, _)| sample_count)
}

/// Appends the samples of `track` to `locations`, in decoding order, and returns the decoding time they end at, or `None` for no sample
///
/// A table that counts another number of samples than the sample size
/// table, or an entry of the sync sample table that names no sample, is
/// refused as the specification reads it, and read lossily as
/// [`StructureDataReader::handle_movie_lossy`](super::StructureDataReader::handle_movie_lossy)
/// sets out. A sample table under which no sample can be placed is refused
/// either way.
pub(super) fn resolve(
    track: &TrackBox,
    locations: &mut Vec<SampleLocation>,
    reading: Reading,
) -> Result<Option<u64>, Error> {
    let track_id = track.tkhd.track_id;
    let stbl = &track.mdia.minf.stbl;
    let missing = |box_type| Error::MissingTable { track_id, box_type };
    let invalid = |box_type, entry: usize| Error::InvalidEntry {
        track_id,
        box_type,
        entry: length_of(entry),
    };
    let overflow = Error::Overflow { track_id };

    let (sample_count, sizes) = sizes(track)?;
    let stts = stbl
        .stts
        .as_ref()
        .ok_or_else(|| missing(TimeToSampleBox::BOX_TYPE))?;
    let mismatch = |box_type, table_sample_count| Error::SampleCountMismatch {
        track_id,
        box_type,
        sample_count,
        table_sample_count,
    };
    let agrees = |box_type, table_sample_count: u64| match reading {
        Reading::Specification if table_sample_count != sample_count => {
            Err(mismatch(box_type, table_sample_count))
        }
        Reading::Specification | Reading::Lossy => Ok(()),
    };
    let stts_sample_count =
        total(stts.entries.iter().map(|entry| entry.sample_count)).ok_or(overflow)?;
    agrees(TimeToSampleBox::BOX_TYPE, stts_sample_count)?;
    if let Some(ctts) = &stbl.ctts {
        agrees(
            CompositionOffsetBox::BOX_TYPE,
            total(ctts.entries.iter().map(|entry| entry.sample_count)).ok_or(overflow)?,
        )?;
    }
    if let Some(sdtp) = &stbl.sdtp {
        agrees(
            SampleDependencyTypeBox::BOX_TYPE,
            length_of(sdtp.entries.len()),
        )?;
    }
    // Why read past the entries of the sync sample table that name no sample
    // or go back rather than drop the table when read lossily: they give no
    // sample a value, and the entries that follow still mark sync samples.
    let sync_samples = match &stbl.stss {
        None => None,
        Some(stss) => {
            // Why not reserve a slot for every entry: the box, not the limit,
            // decides how many entries there are, while the numbers taken go
            // up strictly and none is past `sample_count`, which the reader
            // holds against its limit.
            let mut followed = Vec::with_capacity(
                stss.sample_numbers
                    .len()
                    .min(usize::try_from(sample_count).unwrap_or(usize::MAX)),
            );
            for (entry, &sample_number) in stss.sample_numbers.iter().enumerate() {
                let previous = followed.last().copied().unwrap_or(0);
                if sample_number > previous && u64::from(sample_number) <= sample_count {
                    followed.push(sample_number);
                } else if reading == Reading::Specification {
                    return Err(invalid(SyncSampleBox::BOX_TYPE, entry));
                }
            }
            Some(followed)
        }
    };
    let stsc = stbl
        .stsc
        .as_ref()
        .ok_or_else(|| missing(SampleToChunkBox::BOX_TYPE))?;
    let chunk_offsets = chunk_offsets(stbl).ok_or_else(|| missing(ChunkOffsetBox::BOX_TYPE))?;
    let chunk_count = chunk_offsets.len();
    let chunk_samples = chunk_sample_count(stsc, chunk_count, invalid)?;
    agrees(SampleToChunkBox::BOX_TYPE, chunk_samples)?;
    if sample_count == 0 {
        return Ok(None);
    }
    if chunk_samples == 0 {
        return Err(mismatch(SampleToChunkBox::BOX_TYPE, chunk_samples));
    }
    if stts_sample_count == 0 {
        return Err(mismatch(TimeToSampleBox::BOX_TYPE, stts_sample_count));
    }
    let placed = sample_count.min(chunk_samples).min(stts_sample_count);
    let check_run = |entry: usize, sample_description_index: u32| {
        let data_reference_index = data_reference_index(&stbl.stsd, sample_description_index)
            .ok_or_else(|| invalid(SampleToChunkBox::BOX_TYPE, entry))?;
        if is_in_same_file(&track.mdia.minf.dinf.dref, data_reference_index) {
            Ok(())
        } else {
            Err(Error::MediaDataElsewhere {
                track_id,
                data_reference_index,
            })
        }
    };
    if reading == Reading::Specification {
        for (entry, stsc_entry) in stsc.entries.iter().enumerate() {
            check_run(entry, stsc_entry.sample_description_index)?;
        }
    }

    let mut durations = stts.entries.iter().flat_map(|entry| {
        iter::repeat_n(
            entry.sample_delta,
            usize::try_from(entry.sample_count).unwrap_or(0),
        )
    });
    let mut composition_offsets = stbl.ctts.iter().flat_map(|ctts| {
        ctts.entries.iter().flat_map(|entry| {
            iter::repeat_n(
                entry.sample_offset,
                usize::try_from(entry.sample_count).unwrap_or(0),
            )
        })
    });
    let mut dependencies = stbl.sdtp.iter().flat_map(|sdtp| sdtp.entries.iter());
    let mut sync_samples = sync_samples
        .as_ref()
        .map(|numbers| numbers.iter().peekable());

    let mut decoding_time = 0_u64;
    let mut sample_number = 0_u32;
    let mut current = 0_usize;
    // Why not check every run up front when read lossily: a run no sample
    // placed uses is not read.
    let mut checked_run = None;
    'chunks: for (chunk, chunk_offset) in (1_u32..).zip(chunk_offsets.iter()) {
        while stsc
            .entries
            .get(current.saturating_add(1))
            .is_some_and(|next| next.first_chunk <= chunk)
        {
            current = current.saturating_add(1);
        }
        let Some(stsc_entry) = stsc.entries.get(current) else {
            break;
        };
        let mut offset = chunk_offset;
        for _ in 0..stsc_entry.samples_per_chunk {
            if u64::from(sample_number) >= placed {
                break 'chunks;
            }
            if reading == Reading::Lossy && checked_run != Some(current) {
                check_run(current, stsc_entry.sample_description_index)?;
                checked_run = Some(current);
            }
            let index = usize::try_from(sample_number).map_err(|_| overflow)?;
            sample_number = sample_number.checked_add(1).ok_or(overflow)?;
            let (Some(size), Some(sample_duration)) = (sizes.get(index), durations.next()) else {
                break 'chunks;
            };
            let is_non_sync_sample = match &mut sync_samples {
                None => false,
                Some(numbers) => numbers.next_if_eq(&&sample_number).is_none(),
            };
            let mut sample_flags = SampleFlags::default();
            if let Some(entry) = dependencies.next() {
                sample_flags.is_leading = entry.is_leading;
                sample_flags.sample_depends_on = entry.sample_depends_on;
                sample_flags.sample_is_depended_on = entry.sample_is_depended_on;
                sample_flags.sample_has_redundancy = entry.sample_has_redundancy;
            }
            sample_flags.sample_is_non_sync_sample = is_non_sync_sample;
            locations.push(SampleLocation {
                properties: SampleProperties {
                    track_id,
                    decoding_time,
                    sample_duration,
                    sample_composition_time_offset: composition_offsets.next().unwrap_or(0),
                    sample_flags,
                    sample_description_index: stsc_entry.sample_description_index,
                },
                offset,
                size,
            });
            decoding_time = decoding_time
                .checked_add(u64::from(sample_duration))
                .ok_or(overflow)?;
            offset = offset.checked_add(u64::from(size)).ok_or(overflow)?;
        }
    }
    Ok(Some(decoding_time))
}

/// The `chunk_offset` of each chunk, as the chunk offset table of a track gives it
enum ChunkOffsets<'a> {
    Compact(&'a [u32]),
    Large(&'a [u64]),
}

impl ChunkOffsets<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Compact(offsets) => offsets.len(),
            Self::Large(offsets) => offsets.len(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        let (compact, large) = match self {
            Self::Compact(offsets) => (*offsets, &[][..]),
            Self::Large(offsets) => (&[][..], *offsets),
        };
        compact
            .iter()
            .copied()
            .map(u64::from)
            .chain(large.iter().copied())
    }
}

fn chunk_offsets(stbl: &SampleTableBox) -> Option<ChunkOffsets<'_>> {
    match stbl.chunk_offset.as_ref()? {
        ChunkOffsetVariant::ChunkOffset(stco) => Some(ChunkOffsets::Compact(&stco.chunk_offsets)),
        ChunkOffsetVariant::ChunkLargeOffset(co64) => {
            Some(ChunkOffsets::Large(&co64.chunk_offsets))
        }
        _ => None,
    }
}

/// Number of samples the chunks of a sample-to-chunk table hold, refusing an entry whose `first_chunk` does not increase from 1 or lies past the last chunk
fn chunk_sample_count(
    stsc: &SampleToChunkBox,
    chunk_count: usize,
    invalid: impl Fn(BoxType, usize) -> Error,
) -> Result<u64, Error> {
    let chunk_count =
        u32::try_from(chunk_count).map_err(|_| invalid(ChunkOffsetBox::BOX_TYPE, 0))?;
    let mut total = 0_u64;
    for (entry, stsc_entry) in stsc.entries.iter().enumerate() {
        let next_first = stsc
            .entries
            .get(entry.saturating_add(1))
            .map_or(chunk_count.saturating_add(1), |next| next.first_chunk);
        if (entry == 0 && stsc_entry.first_chunk != 1)
            || stsc_entry.first_chunk == 0
            || next_first <= stsc_entry.first_chunk
            || stsc_entry.first_chunk > chunk_count
        {
            return Err(invalid(SampleToChunkBox::BOX_TYPE, entry));
        }
        let chunks = u64::from(next_first.saturating_sub(stsc_entry.first_chunk));
        total = chunks
            .checked_mul(u64::from(stsc_entry.samples_per_chunk))
            .and_then(|samples| total.checked_add(samples))
            .ok_or_else(|| invalid(SampleToChunkBox::BOX_TYPE, entry))?;
    }
    Ok(total)
}

/// The `data_reference_index` of the sample entry `sample_description_index` names, from 1
fn data_reference_index(stsd: &SampleDescriptionBox, sample_description_index: u32) -> Option<u16> {
    let index = usize::try_from(sample_description_index.checked_sub(1)?).ok()?;
    stsd.sample_descriptions.get(index)?.data_reference_index()
}

/// Sum of the `sample_count`s of the runs of a table, or `None` past `u64`
fn total(mut counts: impl Iterator<Item = u32>) -> Option<u64> {
    counts.try_fold(0_u64, |total, count| total.checked_add(u64::from(count)))
}

fn length_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// Returns the duration of the media whose samples `stbl` declares: the sum of the durations of its samples, in the timescale of the media
///
/// The duration of the media is that of its samples laid end to end on the
/// decoding timeline, ISO/IEC 14496-12 §8.4.2, each lasting the
/// `sample_delta` the time-to-sample table gives it, §8.6.1.2. A table
/// without a time-to-sample table declares a duration of 0; one past what a
/// `u64` counts, `u64::MAX`.
///
/// ```
/// use isobmff::boxes::{SampleTableBox, TimeToSampleBox, TimeToSampleEntry};
/// use isobmff::structure_data::media_duration;
///
/// // 3 samples lasting 1,000 each, then 1 lasting 500
/// let mut stts = TimeToSampleBox::default();
/// stts.entries = [(3, 1_000), (1, 500)]
///     .into_iter()
///     .map(|(sample_count, sample_delta)| {
///         let mut run = TimeToSampleEntry::default();
///         (run.sample_count, run.sample_delta) = (sample_count, sample_delta);
///         run
///     })
///     .collect();
/// let mut stbl = SampleTableBox::default();
/// stbl.stts = Some(stts);
/// assert_eq!(media_duration(&stbl), 3_500);
/// ```
#[must_use]
pub fn media_duration(stbl: &SampleTableBox) -> u64 {
    stbl.stts.as_ref().map_or(0, |stts| {
        stts.entries.iter().fold(0_u64, |total, entry| {
            total.saturating_add(
                u64::from(entry.sample_count).saturating_mul(u64::from(entry.sample_delta)),
            )
        })
    })
}

/// Returns the latest time a sample of `stbl` is composed until: its decoding time, composition offset and duration added, in the timescale of the media, or 0 for no sample
///
/// The composition time of a sample is its decoding time, §8.6.1.2, plus
/// the offset the composition offset table gives it, §8.6.1.3; it lasts
/// its `sample_delta` from there. Without a composition offset table, the
/// samples are composed in decoding order and the end is the
/// [`media_duration`]. An end before 0 is 0; one past what a `u64` counts,
/// `u64::MAX`.
///
/// ```
/// use isobmff::boxes::{
///     CompositionOffsetBox, CompositionOffsetEntry, SampleTableBox, TimeToSampleBox,
///     TimeToSampleEntry,
/// };
/// use isobmff::structure_data::composition_end;
///
/// // 3 samples lasting 1 each, composed at 1, 3 and 2: the last ends at 4
/// let mut run = TimeToSampleEntry::default();
/// (run.sample_count, run.sample_delta) = (3, 1);
/// let mut stts = TimeToSampleBox::default();
/// stts.entries = vec![run];
/// let mut ctts = CompositionOffsetBox::default();
/// ctts.entries = [1, 2, 0]
///     .into_iter()
///     .map(|offset| {
///         let mut entry = CompositionOffsetEntry::default();
///         (entry.sample_count, entry.sample_offset) = (1, offset);
///         entry
///     })
///     .collect();
/// let mut stbl = SampleTableBox::default();
/// (stbl.stts, stbl.ctts) = (Some(stts), Some(ctts));
/// assert_eq!(composition_end(&stbl), 4);
/// ```
#[must_use]
pub fn composition_end(stbl: &SampleTableBox) -> u64 {
    let mut offsets = stbl
        .ctts
        .iter()
        .flat_map(|ctts| ctts.entries.iter())
        .map(|entry| (entry.sample_count, entry.sample_offset));
    let (mut offset_count, mut offset) = offsets.next().unwrap_or((0, 0));
    let (mut decoding_time, mut end) = (0_i128, 0_i128);
    for run in stbl.stts.iter().flat_map(|stts| stts.entries.iter()) {
        let (mut count, delta) = (run.sample_count, i128::from(run.sample_delta));
        // Why not look at each sample: within a run of one duration and one
        // composition offset, each sample ends later than the one before, so
        // the last of the run ends latest, and the walk is linear in runs.
        while count > 0 {
            while offset_count == 0 {
                (offset_count, offset) = offsets.next().unwrap_or((u32::MAX, 0));
            }
            let taken = count.min(offset_count);
            decoding_time = decoding_time.saturating_add(delta.saturating_mul(i128::from(taken)));
            end = end.max(decoding_time.saturating_add(i128::from(offset)));
            count = count.saturating_sub(taken);
            offset_count = offset_count.saturating_sub(taken);
        }
    }
    u64::try_from(end.max(0)).unwrap_or(u64::MAX)
}
