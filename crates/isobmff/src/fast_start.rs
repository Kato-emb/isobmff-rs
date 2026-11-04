//! [`FastStart`], which lays a movie file out again with its `moov` before its media data

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use isobmff_boxes::{
    ChunkLargeOffsetBox, ChunkOffsetVariant, FileTypeBox, MetaBox, MovieBox, MovieFragmentBox,
    TrackBox,
};
use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error};

use crate::convert::to_u64;
use crate::framing::{BoxEvent, BoxReader, BoxWriter};
use crate::structure_data::{chunk_offsets, data_reference_index};
use crate::{ByteRange, DemuxError, FastStartError, Output};

/// Limits a [`FastStart`] reads a file up to
///
/// # Examples
///
/// ```
/// use isobmff::FastStartLimits;
///
/// let limits = FastStartLimits {
///     movie_bytes: 64 * 1024 * 1024,
///     box_count: 65_536,
/// };
/// assert_eq!(limits.box_count, 65_536);
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "a limit added later is one every caller has to set, not one that quietly takes a default"
)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FastStartLimits {
    /// Length of the payload of the `moov`, and of a `meta` at file level, the machine gathers up to
    pub movie_bytes: u64,
    /// Number of boxes at file level the machine keeps the places of up to
    pub box_count: u64,
}

/// What the machine outputs next of the file laid out again
#[derive(Debug)]
enum Piece {
    /// The `moov`, laid out again
    Movie(Vec<u8>),
    /// A box at file level of the file read, its bytes passed through as they arrive, at the file offsets of the range: to the end of the file where the range has no length
    Box(BoxType, ByteRange),
}

/// Machine that lays a movie file that is not fragmented out again with its `moov` before its media data
///
/// A player that reads a file as it arrives needs the `moov` before the
/// media data it locates. The machine reads the boxes at file level of a
/// file a caller can seek in, as a [`Demuxer`](crate::Demuxer) does: it names
/// the bytes it wants, takes them with the file offset they start at, and
/// is told when the file ends. It gathers the `moov` and skips the others.
/// It then outputs the file again, each [`Output`] at the end of those
/// before it: the `ftyp`, the `moov`, and the other boxes in their order.
/// It wants each box but the `moov` once more, and outputs its bytes as they
/// arrive, holding none of them.
///
/// The chunk offsets of the `moov` (§8.7.5), and the offsets of its sample
/// auxiliary information (§8.7.9), move with the box they lie in. A chunk
/// offset that no longer fits 32 bits takes a `co64`, and an offset of
/// sample auxiliary information a `saio` of version 1. Offsets of samples in
/// another file, or relative to an `IdentifiedMediaDataBox`, stay as they
/// are. An offset into the `moov` itself is refused, as the `moov` laid out
/// again need not keep its boxes in place. A file whose media data lies
/// after its `moov` comes out as it is.
///
/// ```
/// use isobmff::{FastStart, FastStartLimits};
///
/// // `ftyp`, an `mdat` of two samples, and the `moov` that locates them
/// let file: &[u8] = include_bytes!("../tests/data/movie_last.mp4");
/// let limits = FastStartLimits { movie_bytes: 1024 * 1024, box_count: 1024 };
/// let mut fast_start = FastStart::new(limits);
///
/// // Seek to the offset the machine wants, read from there, and write what it outputs
/// let mut laid_out = Vec::new();
/// while let Some(wanted) = fast_start.wanted_input() {
///     match file.get(usize::try_from(wanted.offset())?..).unwrap_or_default() {
///         [] => fast_start.finish()?,
///         input => fast_start.handle_input(wanted.offset(), input)?,
///     }
///     while let Some(output) = fast_start.poll_output() {
///         assert_eq!(output.offset(), u64::try_from(laid_out.len())?);
///         laid_out.extend_from_slice(output.bytes());
///     }
/// }
/// // The same file, its `moov` before its `mdat`
/// assert_eq!(laid_out, include_bytes!("../tests/data/movie.mp4"));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct FastStart {
    limits: FastStartLimits,
    reader: BoxReader,
    /// Type and place of each box at file level read, in their order
    boxes: Vec<(BoxType, ByteRange)>,
    movie: Option<MovieBox>,
    /// What is left to output of the file laid out again, once the file has ended
    pieces: VecDeque<Piece>,
    /// Bytes of the box first in `pieces` output already
    passed: u64,
    /// File offset in the file laid out again of the next output
    output_end: u64,
    outputs: VecDeque<Output>,
    /// Whether the file has ended
    finished: bool,
    failed: Option<FastStartError>,
}

impl FastStart {
    /// Creates the machine, which wants the file from its first byte
    #[must_use]
    pub const fn new(limits: FastStartLimits) -> Self {
        Self {
            limits,
            reader: BoxReader::new(),
            boxes: Vec::new(),
            movie: None,
            pieces: VecDeque::new(),
            passed: 0,
            output_end: 0,
            outputs: VecDeque::new(),
            finished: false,
            failed: None,
        }
    }

    /// Returns the file offset of the bytes the machine wants next, and how many it wants where it knows, or `None` once it has output the whole file laid out again or has failed
    ///
    /// Until the file ends, the machine wants the boxes at file level, past
    /// the payloads it skips. It wants no input past a box that runs to the
    /// end of the file, which ends only with the file:
    /// [`handle_input`](Self::handle_input) then finishes the file itself, as
    /// [`finish`](Self::finish) does. Once the file has ended, it wants the
    /// rest of each box it passes through, in the order of the file laid out
    /// again.
    #[must_use]
    pub fn wanted_input(&self) -> Option<ByteRange> {
        if self.failed.is_some() {
            return None;
        }
        if !self.finished {
            return self.reader.wanted_input();
        }
        match self.pieces.front() {
            Some(Piece::Box(_, place)) => Some(ByteRange::new(
                place.offset().saturating_add(self.passed),
                place
                    .length()
                    .map(|length| length.saturating_sub(self.passed)),
            )),
            Some(Piece::Movie(_)) | None => None,
        }
    }

    /// Takes `input`, the bytes of the file from file offset `offset`, and reads the boxes at file level they hold, or, once the file has ended, outputs the bytes of the box it passes through
    ///
    /// `input` starts at or before the offset
    /// [`wanted_input`](Self::wanted_input) names, and the bytes before it
    /// are passed over. A box whose size runs to the end of the file ends the
    /// file, as [`finish`](Self::finish) does. Once the file has ended, the
    /// bytes of `input` the machine wants, up to the end of the box it passes
    /// through, come out as one [`Output`] from
    /// [`poll_output`](Self::poll_output): a copy of them, which the machine
    /// holds until it is taken, so a caller takes the outputs after each
    /// call.
    ///
    /// # Errors
    ///
    /// * [`Read`](FastStartError::Read) of
    ///   [`UnwantedInput`](DemuxError::UnwantedInput): the file has ended,
    ///   and `input` starts past the offset the machine wants.
    /// * [`Read`](FastStartError::Read): the boxes do not read, as for
    ///   [`BoxReader::handle_input`]; the `moov`, or a `meta` at file level,
    ///   is longer than [`FastStartLimits::movie_bytes`]; or the file has a
    ///   second `moov` ([`DuplicateBox`](DemuxError::DuplicateBox)).
    /// * [`FragmentedMovie`](FastStartError::FragmentedMovie): the file has a
    ///   `moof`, or its `moov` an `mvex`.
    /// * [`ItemLocationsInFile`](FastStartError::ItemLocationsInFile): a
    ///   `meta` locates items by file offsets in the file itself.
    /// * [`BoxCountLimitExceeded`](FastStartError::BoxCountLimitExceeded):
    ///   the file has more boxes at file level than
    ///   [`FastStartLimits::box_count`].
    /// * The failure the machine failed with before.
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), FastStartError> {
        self.check_failed()?;
        if self.finished {
            return self
                .pass_through(offset, input)
                .map_err(|error| self.fail(error));
        }
        loop {
            let event = self.reader.handle_input(offset, input);
            match self.read_event(event) {
                Ok(true) => {}
                Ok(false) if self.reader.wanted_input().is_none() => return self.finish(),
                Ok(false) => return Ok(()),
                Err(error) => return Err(self.fail(error)),
            }
        }
    }

    /// Ends the file at the offset the machine wants, and lays the file out again, or, once the file has ended, ends the box that runs to the end of the file that the machine passes through
    ///
    /// The `moov` laid out again is output at once, and so is each box the
    /// machine passes through, as its bytes arrive. A call once the box the
    /// machine passes through has ended does nothing.
    ///
    /// # Errors
    ///
    /// * [`MissingMovie`](FastStartError::MissingMovie): the file has no
    ///   `moov`.
    /// * [`OffsetOutOfFile`](FastStartError::OffsetOutOfFile): an offset of
    ///   media data in the file lies in no box of it.
    /// * [`OffsetInMovie`](FastStartError::OffsetInMovie): the `moov` is laid
    ///   out again, and an offset of media data lies in it.
    /// * [`Write`](FastStartError::Write): the `moov` laid out again does not
    ///   write.
    /// * The failures of [`handle_input`](Self::handle_input), for the box
    ///   the end of the file ends.
    /// * [`Read`](FastStartError::Read) of
    ///   [`UnfinishedBox`](DemuxError::UnfinishedBox): the file ends inside a
    ///   box the machine passes through, before the total its header
    ///   declares.
    pub fn finish(&mut self) -> Result<(), FastStartError> {
        self.check_failed()?;
        let finished = if self.finished {
            self.end_pass_through()
        } else {
            self.finished = true;
            let event = self.reader.finish();
            self.read_event(event).and_then(|_| {
                self.pieces = self.lay_out()?;
                self.output_movie();
                Ok(())
            })
        };
        finished.map_err(|error| self.fail(error))
    }

    /// Returns the next output of the file laid out again, or `None` when there is none left
    ///
    /// The outputs come once [`finish`](Self::finish) has laid the file out,
    /// each at the end of those before it.
    pub fn poll_output(&mut self) -> Option<Output> {
        self.outputs.pop_front()
    }

    /// Outputs the bytes of `input`, from file offset `offset`, that the box first in the pieces wants
    ///
    /// # Errors
    ///
    /// * [`Read`](FastStartError::Read) of
    ///   [`UnwantedInput`](DemuxError::UnwantedInput): `input` starts past
    ///   the offset the machine wants.
    fn pass_through(&mut self, offset: u64, input: &[u8]) -> Result<(), FastStartError> {
        let Some(wanted) = self.wanted_input() else {
            return Ok(());
        };
        let skipped = wanted
            .offset()
            .checked_sub(offset)
            .ok_or(FastStartError::Read {
                error: DemuxError::UnwantedInput { offset, wanted },
            })?;
        let rest = usize::try_from(skipped)
            .ok()
            .and_then(|skipped| input.get(skipped..))
            .unwrap_or_default();
        let taken = wanted.length().map_or(rest, |length| {
            rest.get(..usize::try_from(length).unwrap_or(usize::MAX))
                .unwrap_or(rest)
        });
        if taken.is_empty() {
            return Ok(());
        }
        let taken_len = to_u64(taken.len());
        self.outputs
            .push_back(Output::new(self.output_end, taken.to_vec()));
        self.output_end = self.output_end.saturating_add(taken_len);
        self.passed = self.passed.saturating_add(taken_len);
        if wanted.length() == Some(taken_len) {
            self.next_piece();
        }
        Ok(())
    }

    /// Ends the box first in the pieces at the end of the file, where its range runs to it
    ///
    /// # Errors
    ///
    /// * [`Read`](FastStartError::Read) of
    ///   [`UnfinishedBox`](DemuxError::UnfinishedBox): the box has a length,
    ///   which the bytes passed through fall short of.
    fn end_pass_through(&mut self) -> Result<(), FastStartError> {
        match self.pieces.front() {
            Some(&Piece::Box(box_type, place)) => {
                if let Some(needed_bytes) = place.length() {
                    return Err(FastStartError::Read {
                        error: DemuxError::UnfinishedBox {
                            box_type,
                            needed_bytes,
                            available_bytes: self.passed,
                        },
                    });
                }
                self.next_piece();
                Ok(())
            }
            Some(Piece::Movie(_)) | None => Ok(()),
        }
    }

    /// Ends the box first in the pieces, and outputs the `moov` laid out again where it comes next
    fn next_piece(&mut self) {
        self.pieces.pop_front();
        self.passed = 0;
        self.output_movie();
    }

    /// Outputs the `moov` laid out again, where it comes next
    fn output_movie(&mut self) {
        if !matches!(self.pieces.front(), Some(Piece::Movie(_))) {
            return;
        }
        if let Some(Piece::Movie(bytes)) = self.pieces.pop_front() {
            let length = to_u64(bytes.len());
            self.outputs.push_back(Output::new(self.output_end, bytes));
            self.output_end = self.output_end.saturating_add(length);
        }
    }

    /// Returns the failure the machine failed with, if it has
    fn check_failed(&self) -> Result<(), FastStartError> {
        self.failed.map_or(Ok(()), Err)
    }

    /// Fails the machine with `error`, and returns it
    fn fail(&mut self, error: FastStartError) -> FastStartError {
        self.failed = Some(error);
        error
    }

    /// Reads `event`, and returns whether one came
    fn read_event(
        &mut self,
        event: Result<Option<BoxEvent<'_>>, DemuxError>,
    ) -> Result<bool, FastStartError> {
        let read = |error| FastStartError::Read { error };
        match event.map_err(read)? {
            None => Ok(false),
            Some(BoxEvent::Header(header)) => {
                let limit_boxes = self.limits.box_count;
                if to_u64(self.boxes.len()) >= limit_boxes {
                    return Err(FastStartError::BoxCountLimitExceeded { limit_boxes });
                }
                let box_type = header.box_type();
                if box_type == MovieFragmentBox::BOX_TYPE {
                    return Err(FastStartError::FragmentedMovie);
                }
                if box_type == MovieBox::BOX_TYPE
                    && self.boxes.iter().any(|&(earlier, _)| earlier == box_type)
                {
                    return Err(read(DemuxError::DuplicateBox { box_type }));
                }
                if let Some(place) = self.reader.open_box() {
                    self.boxes.push((box_type, place));
                }
                if box_type == MovieBox::BOX_TYPE || box_type == MetaBox::BOX_TYPE {
                    self.reader.gather(self.limits.movie_bytes).map_err(read)?;
                } else {
                    self.reader.skip().map_err(read)?;
                }
                Ok(true)
            }
            Some(BoxEvent::End) => {
                if let Some(gathered) = self.reader.gathered_box() {
                    let box_type = gathered.header().box_type();
                    let read = |error: Error| FastStartError::Read {
                        error: DemuxError::Box {
                            error: error.in_container(box_type),
                        },
                    };
                    if box_type == MetaBox::BOX_TYPE {
                        let meta = MetaBox::decode_payload(gathered.payload()).map_err(read)?;
                        check_item_locations(Some(&meta))?;
                    } else {
                        let movie = MovieBox::decode_payload(gathered.payload()).map_err(read)?;
                        if movie.mvex.is_some() {
                            return Err(FastStartError::FragmentedMovie);
                        }
                        check_item_locations(movie.meta.as_ref())?;
                        for trak in &movie.trak {
                            check_item_locations(trak.meta.as_ref())?;
                        }
                        self.movie = Some(movie);
                    }
                }
                Ok(true)
            }
            Some(_) => Ok(true),
        }
    }

    /// Returns the pieces of the file laid out again: the `ftyp`, the `moov` and the other boxes in their order, or the file as it is where its media data lies after its `moov`
    fn lay_out(&mut self) -> Result<VecDeque<Piece>, FastStartError> {
        let mut movie = self.movie.take().ok_or(FastStartError::MissingMovie)?;
        let movie_position = self
            .boxes
            .iter()
            .position(|&(box_type, _)| box_type == MovieBox::BOX_TYPE)
            .ok_or(FastStartError::MissingMovie)?;
        let movie_start = self
            .boxes
            .get(movie_position)
            .map_or(0, |(_, place)| place.offset());
        let tracks: Vec<TrackOffsets> = movie.trak.iter().map(TrackOffsets::of).collect();
        if !tracks
            .iter()
            .any(|track| track.in_file().any(|offset| offset < movie_start))
        {
            return Ok(self
                .boxes
                .iter()
                .map(|&(box_type, place)| Piece::Box(box_type, place))
                .collect());
        }

        let is_file_type = |index: &usize| {
            self.boxes
                .get(*index)
                .is_some_and(|&(box_type, _)| box_type == FileTypeBox::BOX_TYPE)
        };
        let mut order: Vec<usize> = (0..self.boxes.len()).filter(is_file_type).collect();
        order.push(movie_position);
        order.extend(
            (0..self.boxes.len()).filter(|index| *index != movie_position && !is_file_type(index)),
        );
        let places: Vec<ByteRange> = order
            .iter()
            .filter_map(|&index| self.boxes.get(index).map(|&(_, place)| place))
            .collect();
        // Why not a fixed number of passes: a pass that widens a table leaves it wide, so the passes number at most one more than the tables.
        loop {
            let movie_len = movie.encoded_len();
            let mut position = 0_u64;
            let moved: Vec<(ByteRange, u64)> = places
                .iter()
                .map(|&place| {
                    let at = position;
                    let length = if place.offset() == movie_start {
                        movie_len
                    } else {
                        place.length().unwrap_or(u64::MAX)
                    };
                    position = position.saturating_add(length);
                    (place, at)
                })
                .collect();
            if !move_offsets(&tracks, &mut movie, &moved, movie_start)? {
                break;
            }
        }

        let mut bytes = Some(
            BoxWriter::new()
                .write_box(&movie)
                .map_err(|error| FastStartError::Write { error })?
                .into_bytes(),
        );
        Ok(order
            .iter()
            .filter_map(|&index| {
                if index == movie_position {
                    return bytes.take().map(Piece::Movie);
                }
                self.boxes
                    .get(index)
                    .map(|&(box_type, place)| Piece::Box(box_type, place))
            })
            .collect())
    }
}

/// Refuses `meta` where its `iloc` locates an item by file offsets in the file itself (§8.11.3)
fn check_item_locations(meta: Option<&MetaBox>) -> Result<(), FastStartError> {
    let in_file = meta
        .and_then(|meta| meta.iloc.as_ref())
        .is_some_and(|iloc| {
            iloc.items
                .iter()
                .any(|item| item.construction_method == 0 && item.data_reference_index == 0)
        });
    if in_file {
        return Err(FastStartError::ItemLocationsInFile);
    }
    Ok(())
}

/// Offsets a track of the file read states to media data, each with whether that media data is in the file itself, as the data reference of the sample entry of its chunk names
#[derive(Debug)]
struct TrackOffsets {
    track_id: u32,
    /// Chunk offsets, in chunk order
    chunks: Vec<(u64, bool)>,
    /// Offsets of each `saio`, for each chunk in chunk order
    auxiliary: Vec<Vec<(u64, bool)>>,
}

impl TrackOffsets {
    /// Returns the offsets `trak` states
    fn of(trak: &TrackBox) -> Self {
        let stbl = &trak.mdia.minf.stbl;
        let entries = stbl
            .stsc
            .as_ref()
            .map_or(&[][..], |stsc| stsc.entries.as_slice());
        let mut entry = 0_usize;
        let chunks: Vec<(u64, bool)> = chunk_offsets(stbl)
            .enumerate()
            .map(|(chunk, offset)| {
                let number = u32::try_from(chunk).unwrap_or(u32::MAX).saturating_add(1);
                while entries
                    .get(entry.saturating_add(1))
                    .is_some_and(|next| next.first_chunk <= number)
                {
                    entry = entry.saturating_add(1);
                }
                let description = entries
                    .get(entry)
                    .filter(|current| current.first_chunk <= number)
                    .map_or(1, |current| current.sample_description_index);
                (offset, data_reference_index(trak, description).is_ok())
            })
            .collect();
        let in_file = |chunk: usize| {
            chunks
                .get(chunk)
                .or_else(|| chunks.first())
                .is_none_or(|&(_, in_file)| in_file)
        };
        let auxiliary = stbl
            .saio
            .iter()
            .map(|saio| {
                saio.offsets
                    .iter()
                    .enumerate()
                    .map(|(chunk, &offset)| (offset, in_file(chunk)))
                    .collect()
            })
            .collect();

        Self {
            track_id: trak.tkhd.track_id,
            chunks,
            auxiliary,
        }
    }

    /// Returns the offsets to media data in the file itself
    fn in_file(&self) -> impl Iterator<Item = u64> + '_ {
        self.chunks
            .iter()
            .chain(self.auxiliary.iter().flatten())
            .filter(|&&(_, in_file)| in_file)
            .map(|&(offset, _)| offset)
    }
}

/// Moves the offsets `tracks` state to media data in the file itself into `movie`, each with the box it lies in, which `moved` places at its new file offset; and returns whether a table was widened to hold them
///
/// # Errors
///
/// * [`OffsetOutOfFile`](FastStartError::OffsetOutOfFile): an offset lies
///   in no box of `moved`.
/// * [`OffsetInMovie`](FastStartError::OffsetInMovie): an offset lies in
///   the `moov`, which starts at file offset `movie_start`.
fn move_offsets(
    tracks: &[TrackOffsets],
    movie: &mut MovieBox,
    moved: &[(ByteRange, u64)],
    movie_start: u64,
) -> Result<bool, FastStartError> {
    let mut widened = false;
    for (track, trak) in tracks.iter().zip(&mut movie.trak) {
        let to = |&(offset, in_file): &(u64, bool)| {
            if !in_file {
                return Ok(offset);
            }
            let track_id = track.track_id;
            let (place, position) = moved
                .iter()
                .find(|(place, _)| {
                    offset
                        .checked_sub(place.offset())
                        .is_some_and(|into| place.length().is_none_or(|length| into < length))
                })
                .ok_or(FastStartError::OffsetOutOfFile { track_id, offset })?;
            // Why not move an offset into the moov by its place in the box: the moov laid out again may order its boxes, and widen its tables, differently.
            if place.offset() == movie_start {
                return Err(FastStartError::OffsetInMovie { track_id, offset });
            }
            offset
                .checked_sub(place.offset())
                .and_then(|into| position.checked_add(into))
                .ok_or(FastStartError::OffsetOutOfFile { track_id, offset })
        };
        let stbl = &mut trak.mdia.minf.stbl;
        let chunk_offsets = track.chunks.iter().map(to).collect::<Result<Vec<_>, _>>()?;
        match &mut stbl.chunk_offset {
            Some(ChunkOffsetVariant::ChunkOffset(stco)) => {
                match chunk_offsets
                    .iter()
                    .map(|&offset| u32::try_from(offset))
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(compact) => stco.chunk_offsets = compact,
                    Err(_) => {
                        let mut co64 = ChunkLargeOffsetBox::default();
                        co64.chunk_offsets = chunk_offsets;
                        stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkLargeOffset(co64));
                        widened = true;
                    }
                }
            }
            Some(ChunkOffsetVariant::ChunkLargeOffset(co64)) => co64.chunk_offsets = chunk_offsets,
            _ => {}
        }
        for (offsets, saio) in track.auxiliary.iter().zip(&mut stbl.saio) {
            saio.offsets = offsets.iter().map(to).collect::<Result<Vec<_>, _>>()?;
            if saio.version == 0
                && saio
                    .offsets
                    .iter()
                    .any(|&offset| u32::try_from(offset).is_err())
            {
                saio.version = 1;
                widened = true;
            }
        }
    }
    Ok(widened)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{
        FileTypeBox, ItemLocation, ItemLocationBox, MetaBox, MovieBox, MovieExtendsBox,
        SampleAuxiliaryInformationOffsetsBox,
    };
    use isobmff_core::{BoxDefinition, BoxHeader, BoxType};

    use super::{FastStart, FastStartLimits};
    use crate::error::{DemuxError, FastStartError};
    use crate::output::Output;
    use crate::structure_data::fixtures::{encoded, in_another_file, movie, track};

    const LIMITS: FastStartLimits = FastStartLimits {
        movie_bytes: 1 << 20,
        box_count: 16,
    };

    /// Returns the header of a box of `box_type` whose payload is `payload_len` bytes, which a test does not hold
    fn header(box_type: &[u8; 4], payload_len: u64) -> Vec<u8> {
        let header = BoxHeader::with_payload_len(BoxType::compact(*box_type), payload_len).unwrap();
        let mut bytes = alloc::vec![0; usize::try_from(header.encoded_len()).unwrap()];
        header.encode(&mut bytes).unwrap();
        bytes
    }

    /// Lays out again the file whose bytes are `pieces`, each at its file offset, the file ending at the end of the last, and returns what the machine outputs up to its failure or the end
    fn outputs_of(
        pieces: &[(u64, Vec<u8>)],
        limits: FastStartLimits,
    ) -> (Vec<Output>, Result<(), FastStartError>) {
        let mut fast_start = FastStart::new(limits);
        let mut outputs = Vec::new();
        let mut ended = Ok(());
        while let Some(wanted) = fast_start.wanted_input() {
            let piece = pieces.iter().find_map(|(offset, bytes)| {
                let into = usize::try_from(wanted.offset().checked_sub(*offset)?).ok()?;
                bytes.get(into..).filter(|rest| !rest.is_empty())
            });
            ended = match piece {
                Some(input) => fast_start.handle_input(wanted.offset(), input),
                None => fast_start.finish(),
            };
            outputs.extend(core::iter::from_fn(|| fast_start.poll_output()));
            if ended.is_err() {
                break;
            }
        }
        (outputs, ended)
    }

    /// Lays out again the file whose bytes are `pieces`, as [`outputs_of`] does, and returns what the machine outputs
    fn lay_out(
        pieces: &[(u64, Vec<u8>)],
        limits: FastStartLimits,
    ) -> Result<Vec<Output>, FastStartError> {
        let (outputs, ended) = outputs_of(pieces, limits);
        ended.map(|()| outputs)
    }

    /// Returns the file offset and the length of each of `outputs`
    fn places(outputs: &[Output]) -> Vec<(u64, u64)> {
        outputs
            .iter()
            .map(|output| {
                (
                    output.offset(),
                    u64::try_from(output.bytes().len()).unwrap(),
                )
            })
            .collect()
    }

    /// Returns the bytes of the `moov` among `outputs`
    fn movie_bytes(outputs: &[Output]) -> Vec<u8> {
        outputs
            .iter()
            .find(|output| output.bytes().get(4..8) == Some(b"moov".as_slice()))
            .map(|output| output.bytes().to_vec())
            .unwrap()
    }

    /// Returns the bytes of a file of an `ftyp`, then `boxes` laid end to end
    fn file(boxes: &[&[u8]]) -> Vec<u8> {
        [encoded(&FileTypeBox::default()).as_slice()]
            .iter()
            .chain(boxes)
            .flat_map(|bytes| bytes.iter().copied())
            .collect()
    }

    /// Returns the movie of one track of one sample of 4 bytes, in a chunk at `chunk_offset`
    fn one_chunk(chunk_offset: u64) -> MovieBox {
        let mut movie = movie([track(1, 10, &[4], &[(1, 1)], &[chunk_offset])]);
        movie.mvhd.next_track_id = 3;
        movie
    }

    /// Returns the chunk offsets of the tracks of the `moov` in `outputs`
    fn chunk_offsets(outputs: &[Output]) -> Vec<Vec<u64>> {
        let bytes = movie_bytes(outputs);
        let (raw, _) = isobmff_core::RawBox::split_first(&bytes).unwrap();
        let movie = <MovieBox as isobmff_core::BoxDecode>::decode_payload(raw.payload()).unwrap();
        movie
            .trak
            .iter()
            .map(|trak| super::chunk_offsets(&trak.mdia.minf.stbl).collect())
            .collect()
    }

    /// Lays out again `file`, handing the machine at most `piece_len` bytes at a time from where it wants, and returns the file laid out again, each output written where its offset says
    fn laid_out_in_pieces(file: &[u8], piece_len: usize) -> Vec<u8> {
        let mut fast_start = FastStart::new(LIMITS);
        let mut laid_out = Vec::new();
        while let Some(wanted) = fast_start.wanted_input() {
            let rest = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            match rest.get(..piece_len.min(rest.len())).unwrap_or_default() {
                [] => fast_start.finish().unwrap(),
                input => fast_start.handle_input(wanted.offset(), input).unwrap(),
            }
            while let Some(output) = fast_start.poll_output() {
                assert_eq!(output.offset(), u64::try_from(laid_out.len()).unwrap());
                laid_out.extend_from_slice(output.bytes());
            }
        }
        laid_out
    }

    #[test]
    fn a_file_laid_out_again_is_the_same_whatever_pieces_its_bytes_arrive_in() {
        let moved: &[u8] = include_bytes!("../tests/data/movie_last.mp4");
        let kept: &[u8] = include_bytes!("../tests/data/movie.mp4");

        let laid_out = [moved, kept].map(|file| {
            [1, 7, 64 * 1024, usize::MAX].map(|piece_len| laid_out_in_pieces(file, piece_len))
        });

        assert_eq!(laid_out, [[kept; 4], [kept; 4]]);
    }

    #[test]
    fn input_past_the_box_passed_through_is_refused_and_the_machine_wants_nothing_once_it_has_output_the_file()
     {
        let file: &[u8] = include_bytes!("../tests/data/movie_last.mp4");
        let finished = || {
            let mut fast_start = FastStart::new(LIMITS);
            while !fast_start.finished {
                let wanted = fast_start.wanted_input().unwrap();
                let input = file
                    .get(usize::try_from(wanted.offset()).unwrap()..)
                    .unwrap();
                match input {
                    [] => fast_start.finish().unwrap(),
                    input => fast_start.handle_input(wanted.offset(), input).unwrap(),
                }
            }
            fast_start
        };
        let mut past = finished();
        let mut whole = finished();
        let wanted = past.wanted_input().unwrap();

        let refused = past.handle_input(wanted.offset() + 1, file.get(1..).unwrap());
        while let Some(wanted) = whole.wanted_input() {
            let input = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap();
            whole.handle_input(wanted.offset(), input).unwrap();
        }

        assert_eq!(
            (
                refused,
                past.wanted_input(),
                whole.wanted_input(),
                whole.finish()
            ),
            (
                Err(FastStartError::Read {
                    error: DemuxError::UnwantedInput {
                        offset: wanted.offset() + 1,
                        wanted
                    }
                }),
                None,
                None,
                Ok(())
            )
        );
    }

    #[test]
    fn a_moov_after_its_media_data_moves_after_the_ftyp_and_its_chunk_offsets_with_the_mdat() {
        let ftyp_len = 16;
        let mdat = header(b"mdat", 4);
        let moov = encoded(&one_chunk(ftyp_len + 8));
        let bytes = file(&[&mdat, b"ABCD", &moov]);
        let moov_len = u64::try_from(moov.len()).unwrap();

        let outputs = lay_out(&[(0, bytes.clone())], LIMITS).unwrap();

        assert_eq!(chunk_offsets(&outputs), [[ftyp_len + moov_len + 8]]);
        assert_eq!(
            places(&outputs),
            [
                (0, ftyp_len),
                (ftyp_len, moov_len),
                (ftyp_len + moov_len, 12)
            ]
        );
        assert_eq!(
            [outputs.first(), outputs.last()].map(|output| output.unwrap().bytes().to_vec()),
            [
                bytes.get(..16).unwrap().to_vec(),
                bytes.get(16..28).unwrap().to_vec()
            ]
        );
    }

    #[test]
    fn a_file_whose_media_data_lies_after_its_moov_comes_out_as_it_is() {
        let free = header(b"free", 0);
        let moov_len = u64::try_from(encoded(&one_chunk(0)).len()).unwrap();
        let moov = encoded(&one_chunk(16 + 8 + moov_len + 8));
        let bytes = file(&[&free, &moov, &header(b"mdat", 4), b"ABCD"]);

        let outputs = lay_out(&[(0, bytes.clone())], LIMITS).unwrap();

        assert_eq!(
            places(&outputs),
            [(0, 16), (16, 8), (24, moov_len), (24 + moov_len, 12)]
        );
        assert_eq!(
            outputs
                .into_iter()
                .flat_map(Output::into_bytes)
                .collect::<Vec<_>>(),
            bytes
        );
    }

    #[test]
    fn the_header_of_a_box_moved_is_copied_as_it_is_and_one_that_runs_to_the_end_of_the_file_stays_last()
     {
        let large = [0, 0, 0, 1, b'm', b'd', b'a', b't', 0, 0, 0, 0, 0, 0, 0, 20];
        let moved = encoded(&one_chunk(16 + 16));
        let moov_len = u64::try_from(moved.len()).unwrap();
        let to_end = [0, 0, 0, 0, b'm', b'd', b'a', b't'];
        let first = encoded(&one_chunk(16 + moov_len + 8));

        assert_eq!(
            [
                lay_out(&[(0, file(&[&large, b"ABCD", &moved]))], LIMITS)
                    .map(|outputs| outputs.last().cloned()),
                lay_out(&[(0, file(&[&first, &to_end, b"ABCD"]))], LIMITS)
                    .map(|outputs| outputs.last().cloned()),
            ],
            [
                Ok(Some(Output::new(
                    16 + moov_len,
                    [large.as_slice(), b"ABCD"].concat()
                ))),
                Ok(Some(Output::new(
                    16 + moov_len,
                    [to_end.as_slice(), b"ABCD"].concat()
                ))),
            ]
        );
    }

    #[test]
    fn chunk_offsets_moved_past_32_bits_take_a_co64_and_the_moov_grows_by_it() {
        let mdat = header(b"mdat", 1 << 32);
        let near_end = (1 << 32) - 4;
        let moov = encoded(&one_chunk(near_end));
        let moov_at = 16 + 16 + (1 << 32);

        let (outputs, ended) = outputs_of(&[(0, file(&[&mdat])), (moov_at, moov.clone())], LIMITS);

        let moov_len = u64::try_from(movie_bytes(&outputs).len()).unwrap();
        assert_eq!(
            ended,
            Err(FastStartError::Read {
                error: DemuxError::UnfinishedBox {
                    box_type: BoxType::compact(*b"mdat"),
                    needed_bytes: 16 + (1 << 32),
                    available_bytes: 16
                }
            })
        );
        assert_eq!(moov_len, u64::try_from(moov.len()).unwrap() + 4);
        assert_eq!(chunk_offsets(&outputs), [[near_end + moov_len]]);
    }

    #[test]
    fn chunks_in_another_file_and_offsets_of_sample_auxiliary_information_are_moved_as_their_data_is()
     {
        let mut moving = one_chunk(24);
        let mut saio = SampleAuxiliaryInformationOffsetsBox::default();
        saio.offsets = Vec::from([26]);
        if let Some(trak) = moving.trak.first_mut() {
            trak.mdia.minf.stbl.saio = Vec::from([saio]);
        }
        moving
            .trak
            .push(in_another_file(track(2, 10, &[4], &[(1, 1)], &[24])));
        let moov = encoded(&moving);
        let moov_len = u64::try_from(moov.len()).unwrap();

        let outputs =
            lay_out(&[(0, file(&[&header(b"mdat", 4), b"ABCD", &moov]))], LIMITS).unwrap();

        let bytes = movie_bytes(&outputs);
        let (raw, _) = isobmff_core::RawBox::split_first(&bytes).unwrap();
        let laid_out =
            <MovieBox as isobmff_core::BoxDecode>::decode_payload(raw.payload()).unwrap();
        let saio_offsets: Vec<_> = laid_out
            .trak
            .iter()
            .flat_map(|trak| {
                trak.mdia
                    .minf
                    .stbl
                    .saio
                    .iter()
                    .flat_map(|saio| saio.offsets.clone())
            })
            .collect();
        assert_eq!(
            (chunk_offsets(&outputs), saio_offsets),
            (
                Vec::from([Vec::from([24 + moov_len]), Vec::from([24])]),
                Vec::from([26 + moov_len])
            )
        );
    }

    #[test]
    fn an_offset_into_the_moov_it_rewrites_is_refused() {
        let mut inside = one_chunk(24);
        let mut saio = SampleAuxiliaryInformationOffsetsBox::default();
        saio.offsets = Vec::from([28 + 20]);
        if let Some(trak) = inside.trak.first_mut() {
            trak.mdia.minf.stbl.saio = Vec::from([saio]);
        }
        let mdat = [header(b"mdat", 4), b"ABCD".to_vec()].concat();

        assert_eq!(
            lay_out(&[(0, file(&[&mdat, &encoded(&inside)]))], LIMITS),
            Err(FastStartError::OffsetInMovie {
                track_id: 1,
                offset: 48
            })
        );
    }

    #[test]
    fn a_file_the_machine_cannot_lay_out_again_is_refused() {
        let mut fragmented = one_chunk(24);
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = Vec::from([isobmff_boxes::TrackExtendsBox::default()]);
        fragmented.mvex = Some(mvex);
        let mut located = one_chunk(24);
        let mut iloc = ItemLocationBox::default();
        iloc.items = Vec::from([ItemLocation::default()]);
        let mut meta = MetaBox::default();
        meta.iloc = Some(iloc);
        located.meta = Some(meta);
        let mdat = [header(b"mdat", 4), b"ABCD".to_vec()].concat();
        let mut wide = movie([track(1, 10, &[4, 4], &[(1, 1)], &[24, 1 << 40])]);
        wide.mvhd.next_track_id = 2;
        let few = FastStartLimits {
            box_count: 2,
            ..LIMITS
        };

        assert_eq!(
            [
                lay_out(&[(0, file(&[&mdat, &encoded(&fragmented)]))], LIMITS),
                lay_out(&[(0, file(&[&header(b"moof", 0), &mdat]))], LIMITS),
                lay_out(&[(0, file(&[&mdat, &encoded(&located)]))], LIMITS),
                lay_out(&[(0, file(&[&mdat]))], LIMITS),
                lay_out(&[(0, file(&[&mdat, &encoded(&wide)]))], LIMITS),
                lay_out(&[(0, file(&[&mdat, &encoded(&one_chunk(24))]))], few),
                lay_out(
                    &[(
                        0,
                        file(&[&mdat, &encoded(&one_chunk(24)), &encoded(&one_chunk(24))])
                    )],
                    LIMITS
                ),
            ],
            [
                Err(FastStartError::FragmentedMovie),
                Err(FastStartError::FragmentedMovie),
                Err(FastStartError::ItemLocationsInFile),
                Err(FastStartError::MissingMovie),
                Err(FastStartError::OffsetOutOfFile {
                    track_id: 1,
                    offset: 1 << 40
                }),
                Err(FastStartError::BoxCountLimitExceeded { limit_boxes: 2 }),
                Err(FastStartError::Read {
                    error: DemuxError::DuplicateBox {
                        box_type: MovieBox::BOX_TYPE
                    }
                }),
            ]
        );
    }
}
