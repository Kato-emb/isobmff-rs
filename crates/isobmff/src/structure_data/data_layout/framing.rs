//! The framing of the samples of a sample table: the chunk each lies in, its data offset and its size
//!
//! The sample-to-chunk table (`stsc`), the chunk offset table (`stco` or
//! `co64`) and the sample size table (`stsz` or `stz2`), ISO/IEC 14496-12
//! §8.7.3 to §8.7.5, give each sample its framing, §7.2.2.

use core::iter::FusedIterator;
use core::num::{NonZeroU32, NonZeroU64};
use core::{ptr, slice};

use isobmff_boxes::{
    ChunkOffsetVariant, DataReferenceBox, SampleDescriptionBox, SampleSizeVariant, SampleSizes,
    SampleToChunkBox, SampleToChunkEntry,
};

use super::data_origin::sample_table_data_origin;
use crate::structure_data::Error;

/// Where the bytes of one sample of a sample table lie in its chunk, with its sample entry
///
/// Its framing, its position and size (ISO/IEC 14496-12 §7.2.2), as the
/// track data layout structures (§8.7) give it, and the sample entry of its
/// chunk (§8.7.4.1), which names the data reference entry specifying the
/// container the offset is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::structure_data) struct SampleFraming {
    /// `sample_description_index`, from 1, of the sample entry of the chunk
    pub(in crate::structure_data) sample_description_index: u32,
    /// Data offset of the sample: the `chunk_offset` of its chunk and the
    /// sizes of the samples before it in the chunk
    pub(in crate::structure_data) offset: u64,
    /// Number of the bytes of the sample
    pub(in crate::structure_data) size: u32,
}

/// Returns the [`SampleFraming`] of each of the first `sample_count` samples the tables declare, in decoding order
///
/// Chunk n takes the entry of `stsc` reached by moving on from the entry of
/// the chunk before it while the next entry's `first_chunk` is at most n; so
/// the first chunks take the first entry whatever its `first_chunk`, an
/// entry the `first_chunk` of the next entry is not after is passed over,
/// and one past the chunks is never reached. [`check_sample_to_chunk`]
/// refuses such entries. A chunk of no sample is passed over.
///
/// The samples end where the chunks, the sizes or the entries of `stsc` do,
/// if those declare fewer. Where the bytes of a sample end past what a `u64`
/// counts, that sample is returned, at its offset, and the samples end after
/// it. A variant of the chunk offset table or the sample size table this
/// crate does not read declares no sample.
pub(in crate::structure_data) fn sample_framings<'a>(
    stsc: &'a SampleToChunkBox,
    chunk_offset: &'a ChunkOffsetVariant,
    sample_size: &'a SampleSizeVariant,
    sample_count: u32,
) -> SampleFramings<'a> {
    SampleFramings {
        chunks: ChunkEntries::new(
            stsc,
            ChunkOffsets::new(chunk_offset).unwrap_or(ChunkOffsets::ChunkOffset([].iter())),
        ),
        sample_description_index: 0,
        remaining: 0,
        offset: 0,
        entry_sizes: EntrySizes::new(sample_size).unwrap_or(EntrySizes::Table([].iter())),
        sample_count,
    }
}

/// The [`SampleFraming`] of each sample of a sample table, as [`sample_framings`] returns them
#[derive(Debug, Clone)]
pub(in crate::structure_data) struct SampleFramings<'a> {
    /// The chunks after the current one
    chunks: ChunkEntries<'a>,
    /// `sample_description_index` of the entry of `stsc` of the current chunk
    sample_description_index: u32,
    /// Number of the samples of the current chunk not yet taken
    remaining: u32,
    /// Data offset of the next sample of the current chunk
    offset: u64,
    entry_sizes: EntrySizes<'a>,
    /// Number of the samples not yet taken
    sample_count: u32,
}

impl SampleFramings<'_> {
    /// Returns the number of samples left to take, which fewer chunks, sizes or entries of `stsc` cut short
    pub(in crate::structure_data) const fn sample_count(&self) -> u32 {
        self.sample_count
    }
}

impl Iterator for SampleFramings<'_> {
    type Item = SampleFraming;

    fn next(&mut self) -> Option<SampleFraming> {
        self.sample_count = self.sample_count.checked_sub(1)?;
        // Why the loop ends: each chunk opened takes one `chunk_offset`.
        while self.remaining == 0 {
            let (offset, entry) = self.chunks.next()?;
            (self.offset, self.remaining, self.sample_description_index) = (
                offset,
                entry.samples_per_chunk,
                entry.sample_description_index,
            );
        }
        let size = self.entry_sizes.next()?;
        let framing = SampleFraming {
            sample_description_index: self.sample_description_index,
            offset: self.offset,
            size,
        };
        self.remaining = self.remaining.saturating_sub(1);
        match self.offset.checked_add(u64::from(size)) {
            Some(offset) => self.offset = offset,
            None => self.sample_count = 0,
        }
        Some(framing)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, usize::try_from(self.sample_count).ok())
    }
}

impl FusedIterator for SampleFramings<'_> {}

/// Refuses the first `sample_count` samples the tables declare where the sample entry of their chunk, in `stsd`, names no data reference entry of `dref` a sample table may use, or where their bytes end past the last data offset
///
/// The tables are read as [`sample_framings`] reads them, for a
/// sample-to-chunk table [`check_sample_to_chunk`] lets through. Each entry
/// of `stsc` whose run holds a sample is looked up once: one whose run holds
/// no chunk, or whose `samples_per_chunk` is 0, names no sample entry. The
/// bytes of a chunk are summed from the sizes the sample size table stores,
/// or from the `sample_size` it gives every sample, once for the chunk; the
/// work grows with the entries the tables store, not with the samples they
/// declare.
///
/// # Errors
///
/// * The failures of [`sample_table_data_origin`], for the first entry of
///   `stsc` it fails for.
/// * [`DataOffsetOverflow`](Error::DataOffsetOverflow): the first sample
///   whose bytes end past the last data offset a `u64` counts.
pub(in crate::structure_data) fn check_sample_framings(
    stsc: &SampleToChunkBox,
    chunk_offset: &ChunkOffsetVariant,
    sample_size: &SampleSizeVariant,
    sample_count: u32,
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
) -> Result<(), Error> {
    let (Some(chunk_offsets), Some(mut sizes)) = (
        ChunkOffsets::new(chunk_offset),
        EntrySizes::new(sample_size),
    ) else {
        return Ok(());
    };
    // Why hold the entry looked up last rather than look the entry up for
    // each chunk: a run holds as many chunks as the chunk offset table, and
    // its entry is looked up once.
    let mut looked_up = None;
    let mut sample_number = 0_u32;
    for (first_offset, entry) in ChunkEntries::new(stsc, chunk_offsets) {
        let samples = entry
            .samples_per_chunk
            .min(sample_count.saturating_sub(sample_number));
        if samples == 0 {
            continue;
        }
        if !looked_up.is_some_and(|looked_up| ptr::eq(looked_up, entry)) {
            sample_table_data_origin(stsd, dref, entry.sample_description_index)?;
            looked_up = Some(entry);
        }
        let overflow = |taken: u32, offset, size| Error::DataOffsetOverflow {
            sample_number: sample_number.saturating_add(taken).saturating_add(1),
            offset,
            size,
        };
        match &mut sizes {
            EntrySizes::Constant { sample_size, .. } => {
                let size = NonZeroU64::from(*sample_size);
                // Why saturate: at most `u32::MAX` samples of at most
                // `u32::MAX` bytes each are fewer than `u64::MAX`.
                let bytes = u64::from(samples).saturating_mul(size.get());
                if first_offset.checked_add(bytes).is_none() {
                    let room = u64::MAX.saturating_sub(first_offset);
                    let taken = u32::try_from(room / size).unwrap_or(u32::MAX);
                    let offset = u64::from(taken).saturating_mul(size.get());
                    return Err(overflow(
                        taken,
                        first_offset.saturating_add(offset),
                        sample_size.get(),
                    ));
                }
            }
            EntrySizes::Table(entry_sizes) => {
                let mut offset = first_offset;
                for (taken, &size) in (0..samples).zip(entry_sizes.by_ref()) {
                    offset = offset
                        .checked_add(u64::from(size))
                        .ok_or_else(|| overflow(taken, offset, size))?;
                }
            }
        }
        sample_number = sample_number.saturating_add(samples);
    }
    Ok(())
}

/// The `chunk_offset` of each chunk, with the entry of the sample-to-chunk table whose run holds it, ISO/IEC 14496-12 §8.7.4
///
/// Chunk n takes the entry reached by moving on from the entry of the chunk
/// before it while the next entry's `first_chunk` is at most n, as
/// [`sample_framings`] sets out. The chunks end where the chunk offset table
/// does, or before the first where `stsc` has no entry.
#[derive(Debug, Clone)]
struct ChunkEntries<'a> {
    /// The entries of the sample-to-chunk table from that of the chunk before
    /// the next
    sample_to_chunk: &'a [SampleToChunkEntry],
    /// The `chunk_offset`s of the chunks not yet taken
    chunk_offsets: ChunkOffsets<'a>,
    /// Number, from 1, of the chunk taken last, or 0 before the first
    // Why hold the number rather than derive it from the offsets left: that
    // needs the number of the chunks besides.
    chunk_number: u32,
}

impl<'a> ChunkEntries<'a> {
    fn new(stsc: &'a SampleToChunkBox, chunk_offsets: ChunkOffsets<'a>) -> Self {
        Self {
            sample_to_chunk: &stsc.entries,
            chunk_offsets,
            chunk_number: 0,
        }
    }
}

impl<'a> Iterator for ChunkEntries<'a> {
    type Item = (u64, &'a SampleToChunkEntry);

    fn next(&mut self) -> Option<(u64, &'a SampleToChunkEntry)> {
        let chunk_offset = self.chunk_offsets.next()?;
        // Why saturate: there are at most `u32::MAX` chunks, the `entry_count`
        // of a chunk offset table.
        self.chunk_number = self.chunk_number.saturating_add(1);
        // Why the loop ends: each entry passed over shortens `sample_to_chunk`.
        while let [_, rest @ ..] = self.sample_to_chunk
            && let [next, ..] = rest
            && next.first_chunk <= self.chunk_number
        {
            self.sample_to_chunk = rest;
        }
        Some((chunk_offset, self.sample_to_chunk.first()?))
    }
}

/// The `chunk_offset` of each chunk, from a chunk offset table of 32 or 64 bits, ISO/IEC 14496-12 §8.7.5
#[derive(Debug, Clone)]
enum ChunkOffsets<'a> {
    ChunkOffset(slice::Iter<'a, u32>),
    ChunkLargeOffset(slice::Iter<'a, u64>),
}

impl<'a> ChunkOffsets<'a> {
    /// The offsets of `chunk_offset`, or `None` for a variant this crate does not read
    fn new(chunk_offset: &'a ChunkOffsetVariant) -> Option<Self> {
        match chunk_offset {
            ChunkOffsetVariant::ChunkOffset(stco) => {
                Some(Self::ChunkOffset(stco.chunk_offsets.iter()))
            }
            ChunkOffsetVariant::ChunkLargeOffset(co64) => {
                Some(Self::ChunkLargeOffset(co64.chunk_offsets.iter()))
            }
            _ => None,
        }
    }

    /// Number of the chunks not yet taken
    fn chunk_count(&self) -> u32 {
        match self {
            Self::ChunkOffset(offsets) => u32::try_from(offsets.len()).unwrap_or(u32::MAX),
            Self::ChunkLargeOffset(offsets) => u32::try_from(offsets.len()).unwrap_or(u32::MAX),
        }
    }
}

impl Iterator for ChunkOffsets<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        match self {
            Self::ChunkOffset(offsets) => offsets.next().copied().map(u64::from),
            Self::ChunkLargeOffset(offsets) => offsets.next().copied(),
        }
    }
}

/// The size of each sample, from the `entry_size`s of a sample size table, or the `sample_size` it gives every sample in their stead, ISO/IEC 14496-12 §8.7.3
#[derive(Debug, Clone)]
enum EntrySizes<'a> {
    Constant {
        sample_size: NonZeroU32,
        /// Number of the samples not yet taken
        // Why not `iter::repeat_n`: its length is a `usize`, which a target of
        // 16 bits cannot hold every `u32` `sample_count` in.
        sample_count: u32,
    },
    Table(slice::Iter<'a, u32>),
}

impl<'a> EntrySizes<'a> {
    /// The sizes `sample_size` gives, or `None` for a variant this crate does not read
    fn new(sample_size: &'a SampleSizeVariant) -> Option<Self> {
        match sample_size {
            SampleSizeVariant::SampleSize(stsz) => match &stsz.sample_sizes {
                SampleSizes::Constant {
                    sample_size,
                    sample_count,
                } => Some(Self::Constant {
                    sample_size: *sample_size,
                    sample_count: *sample_count,
                }),
                SampleSizes::Table { entry_sizes } => Some(Self::Table(entry_sizes.iter())),
                _ => None,
            },
            SampleSizeVariant::CompactSampleSize(stz2) => {
                Some(Self::Table(stz2.entry_sizes.iter()))
            }
            _ => None,
        }
    }

    /// Number of the sizes not yet taken
    fn sample_count(&self) -> u32 {
        match self {
            Self::Constant { sample_count, .. } => *sample_count,
            Self::Table(sizes) => u32::try_from(sizes.len()).unwrap_or(u32::MAX),
        }
    }
}

impl Iterator for EntrySizes<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        match self {
            Self::Constant {
                sample_size,
                sample_count,
            } => {
                *sample_count = sample_count.checked_sub(1)?;
                Some(sample_size.get())
            }
            Self::Table(sizes) => sizes.next().copied(),
        }
    }
}

/// Returns the number of samples `sample_size` declares, or `None` for a variant this crate does not read
///
/// A table of more entries than a `u32` counts declares `u32::MAX`.
pub(in crate::structure_data) fn sample_count(sample_size: &SampleSizeVariant) -> Option<u32> {
    EntrySizes::new(sample_size).map(|sizes| sizes.sample_count())
}

/// Returns the number of chunks `chunk_offset` gives, or `None` for a variant this crate does not read
///
/// A table of more entries than a `u32` counts gives `u32::MAX`.
pub(in crate::structure_data) fn chunk_count(chunk_offset: &ChunkOffsetVariant) -> Option<u32> {
    ChunkOffsets::new(chunk_offset).map(|offsets| offsets.chunk_count())
}

/// Returns the number of samples the `chunk_count` chunks hold, as the runs of chunks of `stsc` give them, or `u64::MAX` past what a `u64` counts
///
/// Each entry gives the chunks from its `first_chunk` to the `first_chunk`
/// of the next entry, or to the last chunk, `samples_per_chunk` samples
/// each, ISO/IEC 14496-12 §8.7.4.1. An entry whose run holds no chunk, as
/// one the `first_chunk` of the next entry is not after, gives none, and a run
/// ends at the last chunk, so an entry whose `first_chunk` is past it gives
/// none either.
///
/// For a table [`check_sample_to_chunk`] lets through, this is the number of
/// samples [`sample_framings`] returns when the sizes and `sample_count`
/// cover all the samples and no sample's end offset exceeds `u64::MAX`.
pub(in crate::structure_data) fn chunk_sample_count(
    stsc: &SampleToChunkBox,
    chunk_count: u32,
) -> u64 {
    let ends = stsc
        .entries
        .iter()
        .skip(1)
        .map(|next| u64::from(next.first_chunk))
        .chain([u64::MAX]);
    let past_last_chunk = u64::from(chunk_count).saturating_add(1);
    stsc.entries
        .iter()
        .zip(ends)
        .fold(0_u64, |total, (entry, end)| {
            let first_chunk = u64::from(entry.first_chunk).min(past_last_chunk);
            let chunks = end.min(past_last_chunk).saturating_sub(first_chunk);
            total.saturating_add(chunks.saturating_mul(u64::from(entry.samples_per_chunk)))
        })
}

/// Refuses a sample-to-chunk table whose `first_chunk`s do not start at 1, go up strictly, and name a chunk of the `chunk_count`
///
/// Each entry is checked against the chunks first, then against 1 for the
/// first entry and the entry before it for the others, so an entry that
/// fails more than one is reported out of range.
///
/// # Errors
///
/// * [`FirstChunkOutOfRange`](Error::FirstChunkOutOfRange): the first entry
///   whose `first_chunk` is 0 or past `chunk_count`, ISO/IEC 14496-12
///   §8.7.4.3.
/// * [`FirstChunkNotOne`](Error::FirstChunkNotOne): the first entry names
///   another chunk than 1, §8.7.4.3.
/// * [`FirstChunkNotIncreasing`](Error::FirstChunkNotIncreasing): the first
///   entry whose `first_chunk` is not after that of the entry before it,
///   §8.7.4.1.
pub(in crate::structure_data) fn check_sample_to_chunk(
    stsc: &SampleToChunkBox,
    chunk_count: u32,
) -> Result<(), Error> {
    let mut previous_first_chunk = None;
    for (entry, sample_to_chunk) in stsc.entries.iter().enumerate() {
        let entry = u64::try_from(entry).unwrap_or(u64::MAX);
        let first_chunk = sample_to_chunk.first_chunk;
        if first_chunk == 0 || first_chunk > chunk_count {
            return Err(Error::FirstChunkOutOfRange {
                entry,
                first_chunk,
                chunk_count,
            });
        }
        match previous_first_chunk {
            None if first_chunk != 1 => return Err(Error::FirstChunkNotOne { first_chunk }),
            Some(previous_first_chunk) if first_chunk <= previous_first_chunk => {
                return Err(Error::FirstChunkNotIncreasing {
                    entry,
                    first_chunk,
                    previous_first_chunk,
                });
            }
            None | Some(_) => {}
        }
        previous_first_chunk = Some(first_chunk);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompactSampleSizeBox,
        SampleSizeBox, SampleSizeVariant, SampleSizes, SampleToChunkBox, SampleToChunkEntry,
    };
    use isobmff_test_support::fixtures::track;

    use super::{
        Error, SampleFraming, check_sample_framings, check_sample_to_chunk, chunk_count,
        chunk_sample_count, sample_count, sample_framings,
    };

    /// `stsc` of the runs of chunks `(first_chunk, samples_per_chunk, sample_description_index)`
    fn stsc(runs: &[(u32, u32, u32)]) -> SampleToChunkBox {
        let mut stsc = SampleToChunkBox::default();
        stsc.entries = runs
            .iter()
            .map(
                |&(first_chunk, samples_per_chunk, sample_description_index)| {
                    let mut entry = SampleToChunkEntry::default();
                    (
                        entry.first_chunk,
                        entry.samples_per_chunk,
                        entry.sample_description_index,
                    ) = (first_chunk, samples_per_chunk, sample_description_index);
                    entry
                },
            )
            .collect();
        stsc
    }

    fn stco(chunk_offsets: &[u32]) -> ChunkOffsetVariant {
        let mut stco = ChunkOffsetBox::default();
        stco.chunk_offsets = chunk_offsets.to_vec();
        ChunkOffsetVariant::ChunkOffset(stco)
    }

    fn co64(chunk_offsets: &[u64]) -> ChunkOffsetVariant {
        let mut co64 = ChunkLargeOffsetBox::default();
        co64.chunk_offsets = chunk_offsets.to_vec();
        ChunkOffsetVariant::ChunkLargeOffset(co64)
    }

    fn stsz(entry_sizes: &[u32]) -> SampleSizeVariant {
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Table {
            entry_sizes: entry_sizes.to_vec(),
        };
        SampleSizeVariant::SampleSize(stsz)
    }

    /// `stsz` of `sample_count` samples of `sample_size` bytes each
    fn constant(sample_size: u32, sample_count: u32) -> SampleSizeVariant {
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Constant {
            sample_size: sample_size.try_into().unwrap(),
            sample_count,
        };
        SampleSizeVariant::SampleSize(stsz)
    }

    fn framing(sample_description_index: u32, offset: u64, size: u32) -> SampleFraming {
        SampleFraming {
            sample_description_index,
            offset,
            size,
        }
    }

    #[test]
    fn the_samples_of_a_chunk_lie_one_after_another_from_its_offset_and_take_the_entry_of_its_run()
    {
        let (stsc, stco, stsz) = (
            stsc(&[(1, 2, 1), (3, 1, 2)]),
            stco(&[100, 200, 300, 400]),
            stsz(&[10, 20, 30, 40, 50]),
        );
        assert_eq!(
            sample_framings(&stsc, &stco, &stsz, 5).collect::<Vec<_>>(),
            [
                framing(1, 100, 10),
                framing(1, 110, 20),
                framing(1, 200, 30),
                framing(1, 230, 40),
                framing(2, 300, 50)
            ]
        );
    }

    #[test]
    fn a_size_for_every_sample_and_offsets_of_64_bits_are_read_alike() {
        let constant = constant(7, 3);
        let mut compact = CompactSampleSizeBox::default();
        compact.entry_sizes = [7, 7, 7].to_vec();
        let compact = SampleSizeVariant::CompactSampleSize(compact);
        let (stsc, co64) = (stsc(&[(1, 2, 1)]), co64(&[1 << 40, 1 << 41]));
        let expected = [
            framing(1, 1 << 40, 7),
            framing(1, (1 << 40) + 7, 7),
            framing(1, 1 << 41, 7),
        ];
        assert_eq!(
            sample_framings(&stsc, &co64, &constant, 3).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            sample_framings(&stsc, &co64, &compact, 3).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn the_samples_end_at_the_sample_count_or_where_the_chunks_the_sizes_or_the_runs_do() {
        let (runs, chunks) = (stsc(&[(1, 2, 1)]), stco(&[0, 100]));
        let count =
            |stsc: &SampleToChunkBox, stco: &ChunkOffsetVariant, sizes: &[u32], sample_count| {
                sample_framings(stsc, stco, &stsz(sizes), sample_count).count()
            };
        assert_eq!(count(&runs, &chunks, &[1; 4], 3), 3);
        assert_eq!(count(&runs, &chunks, &[1; 9], 9), 4);
        assert_eq!(count(&runs, &chunks, &[1; 2], 9), 2);
        assert_eq!(count(&stsc(&[]), &chunks, &[1; 4], 4), 0);
        assert_eq!(count(&runs, &chunks, &[1; 4], 0), 0);
        assert_eq!(
            sample_framings(&runs, &chunks, &constant(1, 2), 9).count(),
            2
        );
    }

    #[test]
    fn the_first_chunks_take_the_first_entry_whatever_its_first_chunk() {
        let (stsc, stco, stsz) = (stsc(&[(2, 1, 7)]), stco(&[0, 10]), stsz(&[1, 1]));
        assert_eq!(
            sample_framings(&stsc, &stco, &stsz, 2).collect::<Vec<_>>(),
            [framing(7, 0, 1), framing(7, 10, 1)]
        );
    }

    #[test]
    fn an_entry_the_next_first_chunk_is_not_after_is_passed_over() {
        let stsz = stsz(&[1; 9]);
        let indexes = |runs: &[(u32, u32, u32)], chunks: &[u32]| {
            sample_framings(&stsc(runs), &stco(chunks), &stsz, 9)
                .map(|framing| framing.sample_description_index)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            indexes(&[(1, 1, 1), (1, 1, 2), (1, 1, 3)], &[0, 10]),
            [3, 3]
        );
        assert_eq!(
            indexes(&[(1, 1, 1), (3, 1, 2), (2, 1, 3)], &[0, 10, 20, 30]),
            [1, 1, 3, 3]
        );
        assert_eq!(indexes(&[(1, 1, 1), (9, 1, 2)], &[0, 10]), [1, 1]);
    }

    #[test]
    fn a_chunk_of_no_sample_is_passed_over() {
        let (stsc, stco, stsz) = (
            stsc(&[(1, 1, 1), (2, 0, 2), (3, 1, 3)]),
            stco(&[0, 10, 20]),
            stsz(&[1, 1]),
        );
        assert_eq!(
            sample_framings(&stsc, &stco, &stsz, 2).collect::<Vec<_>>(),
            [framing(1, 0, 1), framing(3, 20, 1)]
        );
    }

    #[test]
    fn a_sample_that_ends_past_what_a_u64_counts_is_the_last() {
        let (stsc, co64, stsz) = (
            stsc(&[(1, 2, 1)]),
            co64(&[u64::MAX - 5, 0]),
            stsz(&[10, 1, 1, 1]),
        );
        let mut framings = sample_framings(&stsc, &co64, &stsz, 4);
        assert_eq!(framings.next(), Some(framing(1, u64::MAX - 5, 10)));
        assert_eq!(framings.next(), None);
        assert_eq!(framings.next(), None);
    }

    #[test]
    fn the_tables_declare_their_samples_and_chunks() {
        let mut compact = CompactSampleSizeBox::default();
        compact.entry_sizes = [1, 2].to_vec();
        assert_eq!(
            (
                sample_count(&constant(7, 9)),
                sample_count(&stsz(&[1, 2, 3])),
                sample_count(&SampleSizeVariant::CompactSampleSize(compact)),
            ),
            (Some(9), Some(3), Some(2))
        );
        assert_eq!(
            (chunk_count(&stco(&[1, 2])), chunk_count(&co64(&[1, 2, 3]))),
            (Some(2), Some(3))
        );
    }

    #[test]
    fn the_chunks_of_each_run_hold_its_samples_per_chunk_and_a_run_of_no_chunk_none() {
        assert_eq!(
            chunk_sample_count(&stsc(&[(1, 2, 1), (3, 5, 1)]), 4),
            2 * 2 + 2 * 5
        );
        assert_eq!(chunk_sample_count(&stsc(&[(1, 2, 1), (1, 9, 1)]), 2), 9 * 2);
        assert_eq!(chunk_sample_count(&stsc(&[(1, 2, 1), (4, 9, 1)]), 2), 2 * 2);
        assert_eq!(chunk_sample_count(&stsc(&[]), 2), 0);
        assert_eq!(
            chunk_sample_count(&stsc(&[(1, 1, 1)]), u32::MAX),
            u64::from(u32::MAX)
        );
    }

    #[test]
    fn the_chunks_of_a_table_the_check_lets_through_hold_the_samples_the_framings_return() {
        let stsc = stsc(&[(1, 2, 1), (3, 1, 2), (4, 0, 3), (5, 3, 4)]);
        let (stco, stsz) = (stco(&[0, 10, 20, 30, 40, 50]), stsz(&[1; 64]));
        assert_eq!(check_sample_to_chunk(&stsc, 6), Ok(()));
        assert_eq!(
            chunk_sample_count(&stsc, 6),
            u64::try_from(sample_framings(&stsc, &stco, &stsz, 64).count()).unwrap()
        );
    }

    #[test]
    fn a_first_chunk_that_names_no_chunk_is_not_1_first_or_is_not_after_the_one_before_is_refused()
    {
        let check = |runs: &[(u32, u32, u32)]| check_sample_to_chunk(&stsc(runs), 4);
        assert_eq!(check(&[(1, 1, 1), (3, 1, 1)]), Ok(()));
        assert_eq!(
            check(&[(1, 1, 1), (5, 1, 1)]),
            Err(Error::FirstChunkOutOfRange {
                entry: 1,
                first_chunk: 5,
                chunk_count: 4
            })
        );
        assert_eq!(
            check(&[(0, 1, 1)]),
            Err(Error::FirstChunkOutOfRange {
                entry: 0,
                first_chunk: 0,
                chunk_count: 4
            })
        );
        assert_eq!(
            check(&[(2, 1, 1)]),
            Err(Error::FirstChunkNotOne { first_chunk: 2 })
        );
        assert_eq!(
            check(&[(1, 1, 1), (3, 1, 1), (3, 1, 1)]),
            Err(Error::FirstChunkNotIncreasing {
                entry: 2,
                first_chunk: 3,
                previous_first_chunk: 3
            })
        );
        assert_eq!(
            check(&[(1, 1, 1), (3, 1, 1), (2, 1, 1)]),
            Err(Error::FirstChunkNotIncreasing {
                entry: 2,
                first_chunk: 2,
                previous_first_chunk: 3
            })
        );
    }

    #[test]
    fn the_first_sample_of_a_constant_size_to_end_past_the_last_data_offset_is_refused() {
        let trak = track(1, 1, &[], &[], &[]);
        let (stsd, dref) = (&trak.mdia.minf.stbl.stsd, &trak.mdia.minf.dinf.dref);
        let check = |second_chunk_offset| {
            check_sample_framings(
                &stsc(&[(1, 4, 1)]),
                &co64(&[0, second_chunk_offset]),
                &constant(10, 8),
                8,
                stsd,
                dref,
            )
        };
        let overflow = |sample_number, offset| {
            Err(Error::DataOffsetOverflow {
                sample_number,
                offset,
                size: 10,
            })
        };
        assert_eq!(check(u64::MAX - 25), overflow(7, u64::MAX - 5));
        assert_eq!(check(u64::MAX - 39), overflow(8, u64::MAX - 9));
        assert_eq!(check(u64::MAX - 40), Ok(()));
    }
}
