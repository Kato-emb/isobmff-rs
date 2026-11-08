//! The samples the sample table of a track declares, and where their bytes lie

use alloc::vec::Vec;
use core::iter;

use isobmff_boxes::{
    ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox, DataReferenceBox,
    SampleDependencyTypeBox, SampleDescription, SampleDescriptionBox, SampleFlags, SampleSizeBox,
    SampleSizeVariant, SampleSizes, SampleTableBox, SampleToChunkBox, SyncSampleBox,
    TimeToSampleBox, TrackBox,
};
use isobmff_core::{BoxDefinition, BoxType};

use super::Error;
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

/// Appends the samples of `track` to `locations`, in decoding order, refusing a sample table whose tables disagree
pub(super) fn resolve(track: &TrackBox, locations: &mut Vec<SampleLocation>) -> Result<(), Error> {
    let track_id = track.tkhd.track_id;
    let stbl = &track.mdia.minf.stbl;
    let missing = |box_type| Error::MissingTable { track_id, box_type };
    let mismatch = |box_type, sample_count, table_sample_count| Error::SampleCountMismatch {
        track_id,
        box_type,
        sample_count,
        table_sample_count,
    };
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
    let agrees = |box_type, counts: Option<u64>| match counts {
        Some(table_sample_count) if table_sample_count == sample_count => Ok(()),
        Some(table_sample_count) => Err(mismatch(box_type, sample_count, table_sample_count)),
        None => Err(overflow),
    };
    agrees(
        TimeToSampleBox::BOX_TYPE,
        total(stts.entries.iter().map(|entry| entry.sample_count)),
    )?;
    if let Some(ctts) = &stbl.ctts {
        agrees(
            CompositionOffsetBox::BOX_TYPE,
            total(ctts.entries.iter().map(|entry| entry.sample_count)),
        )?;
    }
    if let Some(sdtp) = &stbl.sdtp
        && length_of(sdtp.entries.len()) != sample_count
    {
        return Err(mismatch(
            SampleDependencyTypeBox::BOX_TYPE,
            sample_count,
            length_of(sdtp.entries.len()),
        ));
    }
    if let Some(stss) = &stbl.stss {
        let mut previous = 0;
        for (entry, &sample_number) in stss.sample_numbers.iter().enumerate() {
            if sample_number <= previous || u64::from(sample_number) > sample_count {
                return Err(invalid(SyncSampleBox::BOX_TYPE, entry));
            }
            previous = sample_number;
        }
    }
    let stsc = stbl
        .stsc
        .as_ref()
        .ok_or_else(|| missing(SampleToChunkBox::BOX_TYPE))?;
    let chunk_offsets = chunk_offsets(stbl).ok_or_else(|| missing(ChunkOffsetBox::BOX_TYPE))?;
    let chunk_count = chunk_offsets.len();
    let chunk_samples = chunk_sample_count(stsc, chunk_count, invalid)?;
    if chunk_samples != sample_count {
        return Err(mismatch(
            SampleToChunkBox::BOX_TYPE,
            sample_count,
            chunk_samples,
        ));
    }
    for (entry, stsc_entry) in stsc.entries.iter().enumerate() {
        let data_reference_index =
            data_reference_index(&stbl.stsd, stsc_entry.sample_description_index)
                .ok_or_else(|| invalid(SampleToChunkBox::BOX_TYPE, entry))?;
        if !is_in_same_file(&track.mdia.minf.dinf.dref, data_reference_index) {
            return Err(Error::MediaDataElsewhere {
                track_id,
                data_reference_index,
            });
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
    let mut sync_samples = stbl
        .stss
        .as_ref()
        .map(|stss| stss.sample_numbers.iter().peekable());

    let mut decoding_time = 0_u64;
    let mut sample_number = 0_u32;
    let mut current = 0_usize;
    for (chunk, chunk_offset) in (1_u32..).zip(chunk_offsets.iter()) {
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
            let index = usize::try_from(sample_number).map_err(|_| overflow)?;
            sample_number = sample_number.checked_add(1).ok_or(overflow)?;
            let (Some(size), Some(sample_duration)) = (sizes.get(index), durations.next()) else {
                return Err(mismatch(
                    SampleToChunkBox::BOX_TYPE,
                    sample_count,
                    chunk_samples,
                ));
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
    Ok(())
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
    match stsd.sample_descriptions.get(index)? {
        SampleDescription::Typed(entry) => Some(entry.data_reference_index),
        // Why read the bytes: an entry under a `uuid` type is kept unread,
        // and every sample entry opens with six reserved bytes and then
        // `data_reference_index`, §8.5.2.2.
        SampleDescription::Other(entry) => {
            let bytes = entry.raw_payload()?.get(6..8)?;
            Some(u16::from_be_bytes(bytes.try_into().ok()?))
        }
        _ => None,
    }
}

/// Whether the data reference entry `data_reference_index` names, from 1, places the media data in the same file
fn is_in_same_file(dref: &DataReferenceBox, data_reference_index: u16) -> bool {
    usize::from(data_reference_index)
        .checked_sub(1)
        .and_then(|index| dref.data_entries.get(index))
        .is_some_and(isobmff_boxes::DataEntry::is_in_same_file)
}

/// Sum of the `sample_count`s of the runs of a table, or `None` past `u64`
fn total(mut counts: impl Iterator<Item = u32>) -> Option<u64> {
    counts.try_fold(0_u64, |total, count| total.checked_add(u64::from(count)))
}

fn length_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
