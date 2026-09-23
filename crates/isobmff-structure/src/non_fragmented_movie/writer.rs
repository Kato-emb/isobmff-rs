//! [`NonFragmentedWriter`], a non-fragmented movie file laid down as the samples come

use alloc::vec::Vec;
use core::mem;

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox};
use isobmff_core::{BoxDefinition, BoxHeader, BoxType, FourCC};
use isobmff_sample::{Sample, SampleTableWriter};
use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

use super::{NonFragmentedDisposition, NonFragmentedStructure};
use crate::{Error, compact_box_header, whole_box_header, whole_payload};

/// Lays a non-fragmented movie file down, taking the samples as they come
///
/// The mirror of [`NonFragmentedReader`](crate::NonFragmentedReader): it
/// wires the layers that write a non-fragmented movie file — the structure
/// that holds the order of the top-level boxes, the writing of each box
/// whole, the laying out of the samples as the sample tables of the movie
/// (ISO/IEC 14496-12 §8.7), and the framing of the file — so a caller hands
/// over boxes and samples and takes bytes. It holds no rule of its own, and
/// reaches for no destination: when to write and to where stay with the
/// caller.
///
/// The file is only ever appended to. The media data goes down as the
/// samples come, one `mdat` per chunk with its length declared (§8.1.1), and
/// the movie goes down last, once every chunk offset it states is known
/// (§8.7.5): a file with its movie first is a transform of this one, not a
/// mode of the writer.
///
/// # Contract
///
/// * The order of the boxes is the structure's, held to as they are handed
///   over — a box takes its place in the order where it is handed over or
///   opened, whether its bytes go down then or later: the `ftyp` first,
///   before any chunk, the `moov` once. A box handed over out of that
///   order is [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder) or
///   [`DuplicateBox`](crate::ErrorKind::DuplicateBox), and a file
///   declared over without a `moov` is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
/// * The `ftyp` handed over is laid down as it stands. Where none was handed
///   over, the writer lays its own down before the `moov` or before any
///   chunk, whichever takes its place first: `iso4`, minor version 0,
///   compatible `iso4`, the brand the widest layout it lays down requires
///   (§8.6.4, Annex E.7).
/// * The movie handed to [`handle_movie`](Self::handle_movie) is a template:
///   what it declares of each track is laid down as it stands, but for the
///   sample tables, which the writer fills in from the samples of that track
///   at [`finish`](Self::finish) — the `stsd` kept, the four tables laying
///   the samples out replaced, and every other box the `stbl` carried
///   dropped. A track no sample was handed over to keeps the sample tables
///   it was handed over with. Durations stay the caller's. A sample of a
///   track the movie does not declare is
///   [`Sample`](crate::ErrorKind::Sample) at
///   [`finish`](Self::finish), where the two meet.
/// * A chunk is opened by [`begin_chunk`](Self::begin_chunk), carries the
///   samples handed over next, and is laid down as one `mdat` when the next
///   chunk is opened or the file is declared over; a chunk no sample was
///   handed over to leaves no `mdat`, as it leaves no entry in the tables.
///   What the samples must hold to — one track per chunk, a decode timeline
///   that carries on from sample to sample, no composition offsets or flags
///   the four tables cannot state — is [`SampleTableWriter`]'s contract,
///   reported as [`Sample`](crate::ErrorKind::Sample).
/// * The bytes are taken from [`poll_output`](Self::poll_output), one
///   [`EventBytes`] a call, owned by whoever takes them: the media data of a
///   chunk comes sample by sample, each in the allocation it was handed over
///   in, an empty one passed over. The caller drains before handing over more: bytes are held until
///   they are taken, so writing on without polling has the writer hold the
///   whole file. The samples of the chunk that is open are held until it is
///   laid down.
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again. The bytes made before
///   it are still there to take.
/// * [`finish`](Self::finish) declares the file over. Bytes are still taken
///   after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::SampleFlags;
/// use isobmff_sample::Sample;
/// use isobmff_structure::{NonFragmentedReader, NonFragmentedWriter};
/// # use isobmff_test_support::{file_type, unfragmented_movie};
/// // A file opening with its brands, whose movie declares one track and no sample yet
/// let mut writer = NonFragmentedWriter::new();
/// writer.handle_file_type(file_type())?;
/// writer.handle_movie(unfragmented_movie())?;
///
/// // Two chunks of track 1, each laid down as its own `mdat`
/// writer.begin_chunk()?;
/// writer.handle_sample(Sample::new(1, 0, 3_000, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// writer.handle_sample(Sample::new(1, 3_000, 3_000, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// writer.begin_chunk()?;
/// writer.handle_sample(Sample::new(1, 6_000, 3_000, 0, SampleFlags::ZERO, 1, b"LAST".to_vec()))?;
/// writer.finish()?;
///
/// // The bytes are drained as the writer hands them over
/// let mut file = Vec::new();
/// while let Some(written) = writer.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file opens with the brands
/// assert_eq!(&file[4..8], b"ftyp");
///
/// // Read back, the samples come out as they were laid down
/// let mut reader = NonFragmentedReader::new();
/// reader.handle_input(&file)?;
/// while let Some(wanted) = reader.wanted_extent() {
///     reader.handle_data(wanted.start, &file[wanted.start as usize..wanted.end as usize])?;
/// }
/// reader.finish()?;
/// let read_back: Vec<Vec<u8>> = core::iter::from_fn(|| reader.poll_sample())
///     .map(Sample::into_data)
///     .collect();
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec(), b"LAST".to_vec()]);
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct NonFragmentedWriter {
    boxes: BoxWriter,
    structure: NonFragmentedStructure,
    samples: SampleTableWriter,
    movie: Option<MovieBox>,
    chunk: Vec<Vec<u8>>,
    state: State,
}

/// Where the writer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Laying the file down as the boxes and the samples come
    Writing,
    /// Told the file is over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

impl NonFragmentedWriter {
    /// Creates a writer waiting at the start of a non-fragmented movie file
    #[must_use]
    pub const fn new() -> Self {
        Self {
            boxes: BoxWriter::new(),
            structure: NonFragmentedStructure::new(),
            samples: SampleTableWriter::new(),
            movie: None,
            chunk: Vec::new(),
            state: State::Writing,
        }
    }

    /// Takes the brands the file declares itself readable as, and lays them down
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): a box
    ///   was handed over before them.
    /// * [`Box`](crate::ErrorKind::Box): the box does not write.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.writing()?;
        self.lay_down_file_type(&file_type)
    }

    /// Takes the movie as a template, to be laid down last with its sample tables filled in
    ///
    /// The movie takes its place in the order of the boxes here — a second
    /// one is refused, and brands after it are out of order — and its bytes
    /// go down at [`finish`](Self::finish), once the samples have.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): a movie
    ///   was handed over already.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.writing()?;
        if self.structure.is_at_start() {
            self.lay_down_file_type(&default_file_type())?;
        }
        // Why not placing the movie in the order at `finish`: a second movie
        // is refused where it is handed over, before chunks are laid down
        // against the first, and the structure places a `moov` the same
        // before the media data as after it.
        self.admit(MovieBox::BOX_TYPE)?;
        self.movie = Some(movie);

        Ok(())
    }

    /// Opens a chunk, which the samples handed over next are laid out in, laying down the chunk open before it
    ///
    /// The `mdat` of the chunk takes its place in the order of the boxes
    /// here, and its bytes go down when the next chunk is opened or the file
    /// is declared over.
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box): the chunk before this one
    ///   is longer than the `size` field of an `mdat` can state.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_chunk(&mut self) -> Result<(), Error> {
        self.writing()?;
        if self.structure.is_at_start() {
            self.lay_down_file_type(&default_file_type())?;
        }
        self.lay_down_chunk()?;
        // Why not measuring the header once the chunk is whole: the chunk
        // offset is stated before it is, so the chunk goes down under the
        // compact header whatever its length, and is refused where that form
        // cannot declare it.
        self.admit(MediaDataBox::BOX_TYPE)?;
        let header =
            compact_box_header(MediaDataBox::BOX_TYPE, 0).map_err(|failure| self.fail(failure))?;
        // Why not checked_add: the framing already carries where the file
        // ends in 64 bits, and a compact header is eight bytes past it.
        let chunk_offset = self
            .boxes
            .event_extent()
            .map_or(0, |extent| extent.end)
            .saturating_add(header.encoded_len() as u64);

        self.samples
            .begin_chunk(chunk_offset)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Takes a sample, and places it at the end of the chunk that is open
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.writing()?;
        let data = self
            .samples
            .handle_sample(sample)
            .map_err(|failure| self.fail(failure.into()))?;
        self.chunk.push(data);

        Ok(())
    }

    /// Hands over the bytes the file has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// file is over. Failure is reported by the calls that take the boxes and
    /// the samples, so this one never fails — a failed writer hands over the
    /// bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<EventBytes> {
        self.boxes.poll_output()
    }

    /// Declares the file over, laying down the chunk that is open and then the movie
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box): the chunk that was open is
    ///   longer than the `size` field of an `mdat` can state, or the movie
    ///   does not write.
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the samples as a whole — a chunk holding more samples than an
    ///   `stsc` entry counts among them — or a sample belongs to a track the
    ///   movie does not declare.
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the movie was never handed over, so the file laid down is not a
    ///   non-fragmented movie file.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.writing()?;
        self.lay_down_chunk()?;
        let tables_per_track = self
            .samples
            .finish()
            .map_err(|failure| self.fail(failure.into()))?;
        self.structure
            .finish()
            .map_err(|failure| self.fail(failure))?;
        // Why not unreachable: the structure declared the file over only with
        // the movie in it, so one was handed over, and the fallback is the
        // structure's own answer to a file without one, in place of a panic
        // the lints forbid.
        let Some(mut movie) = self.movie.take() else {
            return Err(self.fail(Error::missing_mandatory_box(MovieBox::BOX_TYPE)));
        };
        for (track_id, tables) in tables_per_track {
            let Some(mdia) = movie.mdia_mut(track_id) else {
                return Err(self.fail(isobmff_sample::Error::unknown_track_id(track_id).into()));
            };
            let stbl = mdia.minf_mut().stbl_mut();
            *stbl = tables.into_sample_table(stbl.stsd().clone());
        }
        let payload = whole_payload(&movie).map_err(|failure| self.fail(failure))?;
        let header = whole_box_header(MovieBox::BOX_TYPE, payload.len() as u64)
            .map_err(|failure| self.fail(failure))?;
        self.frame(header, alloc::vec![payload])?;
        self.boxes
            .finish()
            .map_err(|failure| self.fail(failure.into()))?;
        self.state = State::Finished;

        Ok(())
    }

    /// Lays `file_type` down as the first box of the file, failing the writer where it is refused
    fn lay_down_file_type(&mut self, file_type: &FileTypeBox) -> Result<(), Error> {
        let payload = whole_payload(file_type).map_err(|failure| self.fail(failure))?;
        let header = whole_box_header(FileTypeBox::BOX_TYPE, payload.len() as u64)
            .map_err(|failure| self.fail(failure))?;
        self.admit(FileTypeBox::BOX_TYPE)?;

        self.frame(header, alloc::vec![payload])
    }

    /// Admits the box `box_type` names into the file where the structure places it, failing the writer where it is refused
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
    fn admit(&mut self, box_type: BoxType) -> Result<(), Error> {
        match self
            .structure
            .handle_box_type(box_type)
            .map_err(|failure| self.fail(failure))?
        {
            NonFragmentedDisposition::FileType
            | NonFragmentedDisposition::Movie
            | NonFragmentedDisposition::MediaData => Ok(()),
            NonFragmentedDisposition::Skip => Err(self.fail(Error::box_out_of_order(box_type))),
        }
    }

    /// Returns `Ok` while the writer still takes boxes and samples
    const fn writing(&self) -> Result<(), Error> {
        match self.state {
            State::Writing => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Lays the samples of the chunk that is open down as one `mdat`, if any were handed over
    fn lay_down_chunk(&mut self) -> Result<(), Error> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let media_data = mem::take(&mut self.chunk);
        // Why not checked_add: the samples were handed over as values that fit
        // in memory, so their lengths cannot sum past what 64 bits carry.
        let media_data_len = media_data.iter().fold(0_u64, |total, sample| {
            total.saturating_add(sample.len() as u64)
        });
        let header = compact_box_header(MediaDataBox::BOX_TYPE, media_data_len)
            .map_err(|failure| self.fail(failure))?;

        self.frame(header, media_data)
    }

    /// Hands one box over to the framing of the file, its payload in the pieces it came in
    fn frame(&mut self, header: BoxHeader, payload: Vec<Vec<u8>>) -> Result<(), Error> {
        self.lay_down_step(BoxEvent::Header(header))?;
        for piece in payload.into_iter().filter(|piece| !piece.is_empty()) {
            self.lay_down_step(BoxEvent::Payload(piece))?;
        }
        self.lay_down_step(BoxEvent::End)
    }

    /// Hands one step of the framing over, failing the writer where it is refused
    fn lay_down_step(&mut self, step: BoxEvent) -> Result<(), Error> {
        self.boxes
            .handle_event(step)
            .map_err(|failure| self.fail(failure.into()))
    }

    /// Fails the writer for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for NonFragmentedWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Brands the writer declares where none were handed over, those the widest layout it lays down requires
fn default_file_type() -> FileTypeBox {
    FileTypeBox::new(FourCC::new(*b"iso4"), 0, alloc::vec![FourCC::new(*b"iso4")])
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{FileTypeBox, MovieBox, SampleFlags};
    use isobmff_core::{BoxDecode, BoxDefinition};
    use isobmff_sample::Sample;
    use isobmff_test_support::{file_type, unfragmented_movie};

    use super::{Error, NonFragmentedWriter, default_file_type};
    use crate::ErrorKind;

    /// A sample of the track the movie declares
    fn sample() -> Sample {
        Sample::new(1, 0, 3_000, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())
    }

    /// The bytes the writer has laid down, drained to the end
    fn drained(writer: &mut NonFragmentedWriter) -> Vec<u8> {
        let mut file = Vec::new();
        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn a_file_handed_no_brands_opens_with_the_brands_the_writer_declares() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_movie(unfragmented_movie()).unwrap();
        writer.finish().unwrap();
        let file = drained(&mut writer);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((default_file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn a_file_handed_no_brands_whose_chunk_comes_first_opens_with_the_brands_the_writer_declares() {
        let mut writer = NonFragmentedWriter::new();

        writer.begin_chunk().unwrap();
        writer.handle_sample(sample()).unwrap();
        writer.handle_movie(unfragmented_movie()).unwrap();
        writer.finish().unwrap();
        let file = drained(&mut writer);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((default_file_type(), Some(b"mdat".as_slice())))
        );
    }

    #[test]
    fn the_brands_handed_over_are_laid_down_as_they_stand() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(unfragmented_movie()).unwrap();
        writer.finish().unwrap();
        let file = drained(&mut writer);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn a_chunk_no_sample_was_handed_over_to_leaves_no_media_data_box() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_movie(unfragmented_movie()).unwrap();
        writer.begin_chunk().unwrap();
        writer.begin_chunk().unwrap();
        writer.handle_sample(sample()).unwrap();
        writer.finish().unwrap();
        let file = drained(&mut writer);

        assert_eq!(file.get(24..28), Some(b"mdat".as_slice()));
        assert_eq!(file.get(36..40), Some(b"moov".as_slice()));
    }

    #[test]
    fn brands_handed_over_after_a_chunk_was_opened_are_out_of_order() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_movie(unfragmented_movie()).unwrap();
        writer.begin_chunk().unwrap();

        assert_eq!(
            writer.handle_file_type(file_type()),
            Err(Error::box_out_of_order(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_second_movie_is_rejected() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_movie(unfragmented_movie()).unwrap();

        assert_eq!(
            writer.handle_movie(unfragmented_movie()),
            Err(Error::duplicate_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_sample_handed_over_while_no_chunk_is_open_is_rejected() {
        let mut writer = NonFragmentedWriter::new();

        assert_eq!(
            writer.handle_sample(sample()).map_err(Error::kind),
            Err(ErrorKind::Sample(isobmff_sample::ErrorKind::NoChunkOpen))
        );
    }

    #[test]
    fn a_sample_of_a_track_the_movie_does_not_declare_is_rejected_when_the_file_is_declared_over() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_movie(unfragmented_movie()).unwrap();
        writer.begin_chunk().unwrap();
        writer
            .handle_sample(Sample::new(
                7,
                0,
                3_000,
                0,
                SampleFlags::ZERO,
                1,
                b"SAMP".to_vec(),
            ))
            .unwrap();

        assert_eq!(
            writer.finish().map_err(Error::kind),
            Err(ErrorKind::Sample(isobmff_sample::ErrorKind::UnknownTrackId))
        );
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();

        assert_eq!(
            writer.finish(),
            Err(Error::missing_mandatory_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_failed_writer_reports_the_same_failure_for_every_call_after_it() {
        let mut writer = NonFragmentedWriter::new();
        let failure = Error::box_out_of_order(FileTypeBox::BOX_TYPE);

        writer.handle_movie(unfragmented_movie()).unwrap();

        assert_eq!(writer.handle_file_type(file_type()), Err(failure));
        assert_eq!(writer.begin_chunk(), Err(failure));
        assert_eq!(writer.handle_sample(sample()), Err(failure));
        assert_eq!(writer.finish(), Err(failure));
    }

    #[test]
    fn a_failed_writer_hands_over_the_bytes_it_had_already_laid_down() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();

        assert!(writer.handle_file_type(file_type()).is_err());

        assert_eq!(*writer.poll_output().unwrap(), *b"\0\0\0\x18ftyp");
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut writer = NonFragmentedWriter::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(unfragmented_movie()).unwrap();
        writer.finish().unwrap();

        assert_eq!(writer.begin_chunk(), Err(Error::already_finished()));
        assert_eq!(
            writer.handle_sample(sample()),
            Err(Error::already_finished())
        );
        assert_eq!(writer.finish(), Err(Error::already_finished()));
    }
}
