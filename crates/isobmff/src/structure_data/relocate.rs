//! The offsets a `moov` gives into the file it is in, moved to where the caller maps them

use alloc::vec::Vec;

use isobmff_boxes::{
    ChunkOffsetBox, MovieBox, SampleAuxiliaryInformationOffsetsBox, SampleToChunkBox, TrackBox,
};
use isobmff_core::BoxDefinition;

use super::Error;
use super::sample::entry_data_references;
use super::sample_table::{chunk_offsets, chunk_sample_count, length_of};
use super::sample_table_writer::chunk_offset_table;

/// Moves the chunk offsets and the sample auxiliary information offsets of every track of `movie` that count in the file the `moov` is in to where `relocate` maps them
///
/// The offsets are file offsets of the file the `moov` is in, before and
/// after `relocate` maps them. Only the values of each `stco` or `co64`
/// (ISO/IEC 14496-12 §8.7.5) and of each `saio` of a sample table (§8.7.9)
/// change; every other box and table is left as it is. A chunk whose sample
/// entry names a data reference entry that places the media data elsewhere
/// than in the same file (another file, or an `IdentifiedMediaDataBox`, whose
/// offsets count from its payload, §8.7.2) keeps its offset. A `saio` offset
/// counts as the chunk it is the offset of does; one `saio` offset for every
/// chunk counts as the first chunk does (§8.7.9.3).
///
/// A table whose offsets are mapped is written in the shorter form when
/// every offset of it fits 32 bits, and in the longer form otherwise: `stco`
/// or `co64`, `saio` version 0 or 1. The offsets the movie fragments of a
/// movie with a `mvex` give are not in the `moov` and are not mapped.
///
/// Every offset is mapped before any is written: when one is refused,
/// `movie` is left as it was.
///
/// # Errors
/// * [`Error::OffsetNotRelocated`]: `relocate` returns [`None`] for an
///   offset.
/// * [`Error::MissingTable`]: a track has no chunk offset table, or has
///   chunks and no sample-to-chunk table.
/// * [`Error::InvalidEntry`]: an entry of the sample-to-chunk table has a
///   `first_chunk` that does not start at 1 and increase within the chunks,
///   or names no sample entry with a `data_reference_index`; or a `saio`
///   holds neither one offset nor one for each chunk.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::MovieBox;
/// use isobmff::structure_data::relocate_offsets;
///
/// let mut movie = MovieBox::default();
/// // Every offset of every track moves 1,000 bytes further into the file.
/// relocate_offsets(&mut movie, |offset| offset.checked_add(1_000))?;
/// # Ok::<(), isobmff::structure_data::Error>(())
/// ```
pub fn relocate_offsets(
    movie: &mut MovieBox,
    relocate: impl Fn(u64) -> Option<u64>,
) -> Result<(), Error> {
    let relocated = movie
        .trak
        .iter()
        .map(|trak| relocated(trak, &relocate))
        .collect::<Result<Vec<_>, _>>()?;
    for (trak, (chunk_offsets, saio)) in movie.trak.iter_mut().zip(relocated) {
        let stbl = &mut trak.mdia.minf.stbl;
        if let Some(offsets) = chunk_offsets {
            stbl.chunk_offset = Some(chunk_offset_table(offsets.into_iter()));
        }
        for (saio, offsets) in stbl.saio.iter_mut().zip(saio) {
            if let Some(offsets) = offsets {
                saio.version =
                    u8::from(offsets.iter().any(|&offset| u32::try_from(offset).is_err()));
                saio.offsets = offsets;
            }
        }
    }
    Ok(())
}

/// The chunk offsets and the offsets of each `saio` of a track, mapped, or `None` for each table with no offset in the same file
type Relocated = (Option<Vec<u64>>, Vec<Option<Vec<u64>>>);

fn relocated(trak: &TrackBox, relocate: &impl Fn(u64) -> Option<u64>) -> Result<Relocated, Error> {
    let track_id = trak.tkhd.track_id;
    let stbl = &trak.mdia.minf.stbl;
    let offsets = chunk_offsets(stbl).ok_or(Error::MissingTable {
        track_id,
        box_type: ChunkOffsetBox::BOX_TYPE,
    })?;
    let in_same_file = chunks_in_same_file(trak, offsets.len())?;
    let chunk_offsets = mapped(
        offsets.iter().zip(in_same_file.iter().copied()),
        track_id,
        relocate,
    )?;
    let saio = stbl
        .saio
        .iter()
        .map(|saio| {
            let one_for_every_chunk = saio.offsets.len() == 1;
            if !one_for_every_chunk && saio.offsets.len() != in_same_file.len() {
                return Err(Error::InvalidEntry {
                    track_id,
                    box_type: SampleAuxiliaryInformationOffsetsBox::BOX_TYPE,
                    entry: 0,
                });
            }
            let counts_as = |chunk: usize| {
                in_same_file
                    .get(if one_for_every_chunk { 0 } else { chunk })
                    .copied()
                    .unwrap_or(false)
            };
            mapped(
                saio.offsets
                    .iter()
                    .enumerate()
                    .map(|(chunk, &offset)| (offset, counts_as(chunk))),
                track_id,
                relocate,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((chunk_offsets, saio))
}

/// Each offset of `offsets` mapped by `relocate` where it is in the same file, kept where not, or `None` where none is in the same file
fn mapped(
    offsets: impl Iterator<Item = (u64, bool)>,
    track_id: u32,
    relocate: &impl Fn(u64) -> Option<u64>,
) -> Result<Option<Vec<u64>>, Error> {
    let mut any_in_same_file = false;
    let mapped = offsets
        .map(|(offset, in_same_file)| {
            if !in_same_file {
                return Ok(offset);
            }
            any_in_same_file = true;
            relocate(offset).ok_or(Error::OffsetNotRelocated { track_id, offset })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(any_in_same_file.then_some(mapped))
}

/// Whether the media data of each of the `chunk_count` chunks of `trak` is in the same file, as the data reference entry its sample entry names says
fn chunks_in_same_file(trak: &TrackBox, chunk_count: usize) -> Result<Vec<bool>, Error> {
    let track_id = trak.tkhd.track_id;
    let stbl = &trak.mdia.minf.stbl;
    if chunk_count == 0 {
        return Ok(Vec::new());
    }
    let stsc = stbl.stsc.as_ref().ok_or(Error::MissingTable {
        track_id,
        box_type: SampleToChunkBox::BOX_TYPE,
    })?;
    let invalid = |box_type, entry: usize| Error::InvalidEntry {
        track_id,
        box_type,
        entry: length_of(entry),
    };
    chunk_sample_count(stsc, chunk_count, invalid)?;
    let entries = entry_data_references(&stbl.stsd, &trak.mdia.minf.dinf.dref);
    let in_same_file = stsc
        .entries
        .iter()
        .enumerate()
        .map(|(entry, stsc_entry)| {
            let (_, in_same_file) = usize::try_from(stsc_entry.sample_description_index)
                .ok()
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| entries.get(index))
                .filter(|(data_reference_index, _)| *data_reference_index != 0)
                .ok_or_else(|| invalid(SampleToChunkBox::BOX_TYPE, entry))?;
            Ok((stsc_entry.first_chunk, *in_same_file))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    // Why not look each chunk up from the first entry: the table is checked to
    // start at chunk 1 and increase, so a cursor that only moves on finds the
    // entry of every chunk, and the walk stays linear in the chunks.
    let mut current = 0_usize;
    Ok((1_u32..)
        .take(chunk_count)
        .map(|chunk| {
            while in_same_file
                .get(current.saturating_add(1))
                .is_some_and(|(first_chunk, _)| *first_chunk <= chunk)
            {
                current = current.saturating_add(1);
            }
            in_same_file
                .get(current)
                .is_some_and(|(_, in_same_file)| *in_same_file)
        })
        .collect())
}
