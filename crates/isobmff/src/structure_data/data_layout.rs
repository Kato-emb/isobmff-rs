//! Where the bytes of the samples of a track lie, and what their data offsets count from
//!
//! The track data layout structures, ISO/IEC 14496-12 §8.7, give each sample
//! of a sample table its framing: the chunk it lies in (`stsc`, `stco` or
//! `co64`), its size (`stsz` or `stz2`), and the sample entry that names the
//! data reference entry specifying its container (`dref`). [`data_entry`]
//! looks that entry up.

use core::iter::FusedIterator;
use core::slice;

use isobmff_boxes::{
    ChunkOffsetVariant, DataEntry, DataReferenceBox, SampleDescriptionBox, SampleSizeVariant,
    SampleSizes, SampleToChunkBox, SampleToChunkEntry,
};

use super::Error;
use crate::DataOrigin;

/// Returns the number of the entries of a table of `length` entries, or `u32::MAX` past what a `u32` counts, the width of an `entry_count`
fn count(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

/// Returns the data reference entry the sample entry `sample_description_index`, from 1, of `stsd` names in `dref`
///
/// The data reference entry specifies the container of the data of the
/// samples the sample entry describes, ISO/IEC 14496-12 §6.1: the same
/// file, a resource a URL or a URN names, or an `IdentifiedMediaDataBox`.
///
/// ```
/// use isobmff::boxes::{
///     DataEntry, DataEntryUrlBox, DataReferenceBox, SampleDescription, SampleDescriptionBox,
///     SampleEntry,
/// };
/// use isobmff::FourCc;
/// use isobmff::structure_data::data_layout::data_entry;
///
/// // One sample entry, whose samples lie in the same file
/// let mut stsd = SampleDescriptionBox::default();
/// stsd.sample_descriptions = vec![SampleDescription::Typed(SampleEntry::new(
///     FourCc::new(*b"avc1"),
///     1,
///     Vec::new(),
/// ))];
/// let mut dref = DataReferenceBox::default();
/// dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
/// assert!(data_entry(&stsd, &dref, 1)?.is_in_same_file());
/// # Ok::<(), isobmff::structure_data::Error>(())
/// ```
///
/// # Errors
///
/// * [`SampleDescriptionIndexOutOfRange`](Error::SampleDescriptionIndexOutOfRange):
///   `sample_description_index` is 0, or past the entries of `stsd`.
/// * [`DataReferenceIndexMissing`](Error::DataReferenceIndexMissing): the
///   sample entry ends before its `data_reference_index`.
/// * [`DataReferenceIndexOutOfRange`](Error::DataReferenceIndexOutOfRange):
///   the `data_reference_index` is 0, or past the entries of `dref`.
pub fn data_entry<'a>(
    stsd: &SampleDescriptionBox,
    dref: &'a DataReferenceBox,
    sample_description_index: u32,
) -> Result<&'a DataEntry, Error> {
    let sample_entry = usize::try_from(sample_description_index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| stsd.sample_descriptions.get(index))
        .ok_or(Error::SampleDescriptionIndexOutOfRange {
            sample_description_index,
            sample_entry_count: count(stsd.sample_descriptions.len()),
        })?;
    let data_reference_index =
        sample_entry
            .data_reference_index()
            .ok_or(Error::DataReferenceIndexMissing {
                sample_description_index,
            })?;
    usize::from(data_reference_index)
        .checked_sub(1)
        .and_then(|index| dref.data_entries.get(index))
        .ok_or(Error::DataReferenceIndexOutOfRange {
            sample_description_index,
            data_reference_index,
            data_entry_count: count(dref.data_entries.len()),
        })
}

/// Returns what the data offsets of the samples of a sample table whose sample entry is `sample_description_index` count from
///
/// They count from the payload of an `IdentifiedMediaDataBox` where the data
/// reference entry is a `DataEntryImdaBox`, and from the start of the file
/// it identifies otherwise, ISO/IEC 14496-12 §8.7.5.3: "Otherwise, the value
/// of `chunk_offset` is relative to the start of the containing media file".
///
/// # Errors
///
/// * The failures of [`data_entry`].
/// * [`SequenceNumberOutsideMovieFragment`](Error::SequenceNumberOutsideMovieFragment):
///   the data reference entry is a `DataEntrySeqNumImdaBox`, whose samples
///   are each in a movie fragment, §8.7.2.1.
pub(super) fn sample_table_data_origin(
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
    sample_description_index: u32,
) -> Result<DataOrigin, Error> {
    match data_entry(stsd, dref, sample_description_index)? {
        DataEntry::Imda(_) => Ok(DataOrigin::IdentifiedMediaData),
        DataEntry::SeqNumImda(_) => Err(Error::SequenceNumberOutsideMovieFragment {
            sample_description_index,
        }),
        DataEntry::Url(_) | DataEntry::Urn(_) | DataEntry::Other(_) | _ => Ok(DataOrigin::File),
    }
}

/// Where the bytes of one sample of a sample table lie in its chunk, with its sample entry
///
/// Its framing, its position and size (ISO/IEC 14496-12 §7.2.2), as the
/// track data layout structures (§8.7) give it, and the sample entry of its
/// chunk (§8.7.4.1), which names the data reference entry specifying the
/// container the offset is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SampleFraming {
    /// `sample_description_index`, from 1, of the sample entry of the chunk
    pub(super) sample_description_index: u32,
    /// Data offset of the sample: the `chunk_offset` of its chunk and the
    /// sizes of the samples before it in the chunk
    pub(super) offset: u64,
    /// Number of the bytes of the sample
    pub(super) size: u32,
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
pub(super) fn sample_framings<'a>(
    stsc: &'a SampleToChunkBox,
    chunk_offset: &'a ChunkOffsetVariant,
    sample_size: &'a SampleSizeVariant,
    sample_count: u32,
) -> SampleFramings<'a> {
    SampleFramings {
        sample_to_chunk: &stsc.entries,
        chunk_offsets: ChunkOffsets::new(chunk_offset)
            .unwrap_or(ChunkOffsets::ChunkOffset([].iter())),
        chunk_number: 0,
        remaining: 0,
        offset: 0,
        entry_sizes: EntrySizes::new(sample_size).unwrap_or(EntrySizes::Table([].iter())),
        sample_count,
    }
}

/// The [`SampleFraming`] of each sample of a sample table, as [`sample_framings`] returns them
#[derive(Debug, Clone)]
pub(super) struct SampleFramings<'a> {
    /// The entries of the sample-to-chunk table from that of the current
    /// chunk
    sample_to_chunk: &'a [SampleToChunkEntry],
    /// The `chunk_offset`s of the chunks after the current one
    chunk_offsets: ChunkOffsets<'a>,
    /// Number, from 1, of the current chunk, or 0 before the first
    // Why hold the number rather than derive it from the offsets left: that
    // needs the number of the chunks besides.
    chunk_number: u32,
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
    pub(super) const fn sample_count(&self) -> u32 {
        self.sample_count
    }
}

impl Iterator for SampleFramings<'_> {
    type Item = SampleFraming;

    fn next(&mut self) -> Option<SampleFraming> {
        self.sample_count = self.sample_count.checked_sub(1)?;
        // Why the loops end: each chunk opened takes one `chunk_offset`, and
        // each entry passed over shortens `sample_to_chunk`.
        while self.remaining == 0 {
            self.offset = self.chunk_offsets.next()?;
            // Why saturate: there are at most `u32::MAX` chunks, the
            // `entry_count` of a chunk offset table.
            self.chunk_number = self.chunk_number.saturating_add(1);
            while let [_, rest @ ..] = self.sample_to_chunk
                && let [next, ..] = rest
                && next.first_chunk <= self.chunk_number
            {
                self.sample_to_chunk = rest;
            }
            self.remaining = self.sample_to_chunk.first()?.samples_per_chunk;
        }
        // Why the entry is there: a chunk is opened only where `stsc` has an
        // entry, and the slice keeps one entry at least once it has one.
        let [entry, ..] = self.sample_to_chunk else {
            return None;
        };
        let size = self.entry_sizes.next()?;
        let framing = SampleFraming {
            sample_description_index: entry.sample_description_index,
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

/// Refuses the samples of `framings` whose sample entry, in `stsd`, names no data reference entry of `dref` a sample table may use, or whose bytes end past the last data offset
///
/// Only the sample entries the samples take are looked up: an entry of
/// `stsc` that no sample takes, as one of a run of no chunk or of no sample,
/// names none.
///
/// # Errors
///
/// * The failures of [`sample_table_data_origin`], for the first sample it
///   fails for.
/// * [`DataOffsetOverflow`](Error::DataOffsetOverflow): the first sample
///   whose bytes end past the last data offset a `u64` counts.
pub(super) fn check_sample_framings(
    framings: SampleFramings<'_>,
    stsd: &SampleDescriptionBox,
    dref: &DataReferenceBox,
) -> Result<(), Error> {
    for (sample_number, framing) in (1..=u32::MAX).zip(framings) {
        sample_table_data_origin(stsd, dref, framing.sample_description_index)?;
        if framing
            .offset
            .checked_add(u64::from(framing.size))
            .is_none()
        {
            return Err(Error::DataOffsetOverflow {
                sample_number,
                offset: framing.offset,
                size: framing.size,
            });
        }
    }
    Ok(())
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
            Self::ChunkOffset(offsets) => count(offsets.len()),
            Self::ChunkLargeOffset(offsets) => count(offsets.len()),
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
        sample_size: u32,
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
                    sample_size: sample_size.get(),
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
            Self::Table(sizes) => count(sizes.len()),
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
                Some(*sample_size)
            }
            Self::Table(sizes) => sizes.next().copied(),
        }
    }
}

/// Returns the number of samples `sample_size` declares, or `None` for a variant this crate does not read
///
/// A table of more entries than a `u32` counts declares `u32::MAX`.
pub(super) fn sample_count(sample_size: &SampleSizeVariant) -> Option<u32> {
    EntrySizes::new(sample_size).map(|sizes| sizes.sample_count())
}

/// Returns the number of chunks `chunk_offset` gives, or `None` for a variant this crate does not read
///
/// A table of more entries than a `u32` counts gives `u32::MAX`.
pub(super) fn chunk_count(chunk_offset: &ChunkOffsetVariant) -> Option<u32> {
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
pub(super) fn chunk_sample_count(stsc: &SampleToChunkBox, chunk_count: u32) -> u64 {
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
pub(super) fn check_sample_to_chunk(
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
    use alloc::string::String;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompactSampleSizeBox, DataEntry,
        DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox,
        DataReferenceBox, SampleDescription, SampleDescriptionBox, SampleEntry, SampleSizeBox,
        SampleSizeVariant, SampleSizes, SampleToChunkBox, SampleToChunkEntry,
    };
    use isobmff_core::{AnyBox, BoxType, FourCc, Utf8String};

    use crate::DataOrigin;

    use super::{
        Error, SampleFraming, check_sample_to_chunk, chunk_count, chunk_sample_count, sample_count,
        sample_framings, sample_table_data_origin,
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

    /// `stsd` of one sample entry, and `dref` of `data_entry` alone
    fn track_of(data_entry: DataEntry) -> (SampleDescriptionBox, DataReferenceBox) {
        let mut stsd = SampleDescriptionBox::default();
        stsd.sample_descriptions = [SampleDescription::Typed(SampleEntry::new(
            FourCc::new(*b"avc1"),
            1,
            Vec::new(),
        ))]
        .to_vec();
        let mut dref = DataReferenceBox::default();
        dref.data_entries = [data_entry].to_vec();
        (stsd, dref)
    }

    #[test]
    fn the_samples_of_a_sample_table_count_from_an_imda_or_from_the_start_of_the_file_otherwise() {
        let origin = |data_entry| {
            let (stsd, dref) = track_of(data_entry);
            sample_table_data_origin(&stsd, &dref, 1)
        };
        let mut url = DataEntryUrlBox::default();
        url.location = Some(Utf8String::new(String::from("a.mp4")).unwrap());
        let mut urn = DataEntryUrnBox::default();
        urn.name = Utf8String::new(String::from("urn:x")).unwrap();
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = 7;
        let other = AnyBox::from_raw_bytes(BoxType::compact(*b"xxxx"), Vec::new());
        assert_eq!(
            [
                origin(DataEntry::Url(DataEntryUrlBox::default())),
                origin(DataEntry::Url(url)),
                origin(DataEntry::Urn(urn)),
                origin(DataEntry::Other(other)),
                origin(DataEntry::Imda(imdt)),
                origin(DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default())),
            ],
            [
                Ok(DataOrigin::File),
                Ok(DataOrigin::File),
                Ok(DataOrigin::File),
                Ok(DataOrigin::File),
                Ok(DataOrigin::IdentifiedMediaData),
                Err(Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 1
                }),
            ]
        );
        let (stsd, dref) = track_of(DataEntry::Url(DataEntryUrlBox::default()));
        assert_eq!(
            sample_table_data_origin(&stsd, &dref, 2),
            Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index: 2,
                sample_entry_count: 1
            })
        );
    }
}
