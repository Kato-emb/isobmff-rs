//! [`NonFragmentedMuxFsm`], a non-fragmented movie file laid down as the samples come

use alloc::vec::Vec;
use core::mem;

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox};
use isobmff_core::{BoxDefinition, BoxType, FourCC};
use isobmff_sample::{Sample, SampleTableWriter};
use isobmff_sequence::EventBytes;

use super::{NonFragmentedDisposition, NonFragmentedStructure};
use crate::mux_output::MuxOutput;
use crate::{Error, compact_box_header, whole_box_header, whole_payload};

/// Lays a non-fragmented movie file down, taking the samples as they come
///
/// The mirror of [`MovieDemuxFsm`](crate::MovieDemuxFsm): it
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
/// mode of the mux FSM.
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
/// * The movie is handed over before any chunk, though its bytes go down
///   last: a chunk opened, or a sample handed over, before it is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder) of the `mdat`.
/// * The `ftyp` handed over is laid down as it stands. Where none was handed
///   over, the mux FSM lays its own down before the `moov`: `iso4` as its
///   `major_brand` and its one `compatible_brands` entry, with
///   `minor_version` 0, the brand the widest layout it lays down requires
///   (Annex E.7).
/// * The movie handed to [`handle_movie`](Self::handle_movie) is a template:
///   what it declares of each track is laid down as it stands, but for the
///   sample tables and the durations. The mux FSM fills the sample tables in
///   from the samples of that track
///   at [`finish`](Self::finish) — the `stsd` kept, the four tables laying
///   the samples out replaced, and every other box the `stbl` carried
///   dropped. A track no sample was handed over to keeps the sample tables
///   it was handed over with. With the tables in, the durations of the
///   `mdhd`, `tkhd` and `mvhd` are stated from them, as
///   [`MovieBox::state_durations`] states them (ISO/IEC 14496-12 §8.4.2.3,
///   §8.3.2.3, §8.2.2.3): an edit list handed over is laid down as it
///   stands and the track lasts the sum of its edits, and the media of a
///   track whose tables are empty lasts 0, as does the track unless an edit
///   list says otherwise. The samples are checked against the movie as they
///   are handed over, as [`SampleTableWriter`] checks them.
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
///   they are taken, so writing on without polling has the mux FSM hold the
///   whole file. The samples of the chunk that is open are held until it is
///   laid down.
/// * An `Err` leaves the mux FSM failed for good,
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
/// use isobmff_structure::{MovieDemuxFsm, NonFragmentedMuxFsm};
/// # use isobmff_test_support::{file_type, unfragmented_movie};
/// // A file opening with its brands, whose movie declares one track and no sample yet
/// let mut mux_fsm = NonFragmentedMuxFsm::new();
/// mux_fsm.handle_file_type(file_type())?;
/// mux_fsm.handle_movie(unfragmented_movie())?;
///
/// // Two chunks of track 1, each laid down as its own `mdat`
/// mux_fsm.begin_chunk()?;
/// mux_fsm.handle_sample(Sample::new(1, 0, 3_000, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// mux_fsm.handle_sample(Sample::new(1, 3_000, 3_000, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// mux_fsm.begin_chunk()?;
/// mux_fsm.handle_sample(Sample::new(1, 6_000, 3_000, 0, SampleFlags::ZERO, 1, b"LAST".to_vec()))?;
/// mux_fsm.finish()?;
///
/// // The bytes are drained as the mux FSM hands them over
/// let mut file = Vec::new();
/// while let Some(written) = mux_fsm.poll_output() {
///     file.extend_from_slice(&written);
/// }
///
/// // The file opens with the brands
/// assert_eq!(&file[4..8], b"ftyp");
///
/// // Read back, the samples come out as they were laid down
/// let mut demux_fsm = MovieDemuxFsm::new();
/// while let Some(wanted) = demux_fsm.wanted_input() {
///     let start = (wanted.offset() as usize).min(file.len());
///     let end = wanted.length().map_or(file.len(), |length| start + length as usize);
///     if start == end {
///         demux_fsm.finish()?;
///     } else {
///         demux_fsm.handle_input(wanted.offset(), &file[start..end])?;
///     }
/// }
/// let read_back: Vec<Vec<u8>> = core::iter::from_fn(|| demux_fsm.poll_sample())
///     .map(Sample::into_data)
///     .collect();
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec(), b"LAST".to_vec()]);
/// # Ok::<(), isobmff_structure::Error>(())
/// ```
#[derive(Debug)]
pub struct NonFragmentedMuxFsm {
    output: MuxOutput,
    structure: NonFragmentedStructure,
    movie: Option<(MovieBox, SampleTableWriter)>,
    chunk: Vec<Vec<u8>>,
}

impl NonFragmentedMuxFsm {
    /// Creates a mux FSM waiting at the start of a non-fragmented movie file
    #[must_use]
    pub const fn new() -> Self {
        Self {
            output: MuxOutput::new(),
            structure: NonFragmentedStructure::new(),
            movie: None,
            chunk: Vec::new(),
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
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.output.writing()?;
        let laid_down = self.lay_down_file_type(&file_type);

        self.output.record(laid_down)
    }

    /// Takes the movie as a template, to be laid down last with its sample tables filled in and its durations stated from them
    ///
    /// The movie takes its place in the order of the boxes here — a second
    /// one is refused, and brands after it are out of order — and its bytes
    /// go down at [`finish`](Self::finish), once the samples have. The
    /// samples handed over from here on are checked against it.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): a movie
    ///   was handed over already.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.output.writing()?;
        let admit_movie = || -> Result<(), Error> {
            if self.structure.is_at_start() {
                self.lay_down_file_type(&default_file_type())?;
            }
            // Why not placing the movie in the order at `finish`: a second movie
            // is refused where it is handed over, before chunks are laid down
            // against the first, and the structure places a `moov` the same
            // before the media data as after it.
            self.admit(MovieBox::BOX_TYPE)?;
            let samples = SampleTableWriter::new(&movie);
            self.movie = Some((movie, samples));

            Ok(())
        };
        let admitted = admit_movie();

        self.output.record(admitted)
    }

    /// Opens a chunk, which the samples handed over next are laid out in, laying down the chunk open before it
    ///
    /// The `mdat` of the chunk takes its place in the order of the boxes
    /// here, and its bytes go down when the next chunk is opened or the file
    /// is declared over.
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Box`](crate::ErrorKind::Box): the chunk before this one
    ///   is longer than the `size` field of an `mdat` can state.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn begin_chunk(&mut self) -> Result<(), Error> {
        self.output.writing()?;
        let mut open_chunk = || -> Result<(), Error> {
            self.samples()?;
            self.lay_down_chunk()?;
            // Why not measuring the header once the chunk is whole: the chunk
            // offset is stated before it is, so the chunk goes down under the
            // compact header whatever its length, and is refused where that form
            // cannot declare it.
            self.admit(MediaDataBox::BOX_TYPE)?;
            let header = compact_box_header(MediaDataBox::BOX_TYPE, 0)?;
            // Why not checked_add: the framing already carries where the file
            // ends in 64 bits, and a compact header is eight bytes past it.
            let chunk_offset = self
                .output
                .position()
                .saturating_add(header.encoded_len() as u64);

            self.samples()?.begin_chunk(chunk_offset)?;

            Ok(())
        };
        let begun = open_chunk();

        self.output.record(begun)
    }

    /// Takes a sample, and places it at the end of the chunk that is open
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): the
    ///   movie was not handed over first.
    /// * [`Sample`](crate::ErrorKind::Sample): what the sample layer
    ///   makes of the sample.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.output.writing()?;
        let placed = self
            .samples()
            .and_then(|samples| samples.handle_sample(sample).map_err(Error::from))
            .map(|data| self.chunk.push(data));

        self.output.record(placed)
    }

    /// Hands over the bytes the file has been laid down as so far
    ///
    /// Reports `None` once they are used up: more samples are needed, or the
    /// file is over. Failure is reported by the calls that take the boxes and
    /// the samples, so this one never fails — a failed mux FSM hands over the
    /// bytes it had already made, then nothing from there on.
    pub fn poll_output(&mut self) -> Option<EventBytes> {
        self.output.poll_output()
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
    ///   `stsc` entry counts among them.
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the movie was never handed over, so the file laid down is not a
    ///   non-fragmented movie file.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the mux FSM keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.output.writing()?;
        let mut finish_file = || -> Result<(), Error> {
            self.lay_down_chunk()?;
            self.structure.finish()?;
            // Why not unreachable: the structure declared the file over only with
            // the movie in it, so one was handed over, and the fallback is the
            // structure's own answer to a file without one, in place of a panic
            // the lints forbid.
            let Some((mut movie, mut samples)) = self.movie.take() else {
                return Err(Error::missing_mandatory_box(MovieBox::BOX_TYPE));
            };
            let tables_per_track = samples.finish()?;
            for (track_id, tables) in tables_per_track {
                // Why not unreachable: the sample layer took a sample of a track
                // only where the movie declares it, and the fallback is its own
                // answer to one it does not, in place of a panic the lints forbid.
                let Some(track) = movie.trak_mut(track_id) else {
                    return Err(isobmff_sample::Error::unknown_track_id(track_id).into());
                };
                let stbl = track.mdia_mut().minf_mut().stbl_mut();
                *stbl = tables.into_sample_table(stbl.stsd().clone());
            }
            movie.state_durations();
            let payload = whole_payload(&movie)?;
            let header = whole_box_header(MovieBox::BOX_TYPE, payload.len() as u64)?;
            self.output.frame(header, [payload])?;
            self.output.finish()
        };
        let finished = finish_file();

        self.output.record(finished)
    }

    /// Lays `file_type` down as the first box of the file
    fn lay_down_file_type(&mut self, file_type: &FileTypeBox) -> Result<(), Error> {
        let payload = whole_payload(file_type)?;
        let header = whole_box_header(FileTypeBox::BOX_TYPE, payload.len() as u64)?;
        self.admit(FileTypeBox::BOX_TYPE)?;

        self.output.frame(header, [payload])
    }

    /// Returns the sample layer, made once the movie was handed over
    fn samples(&mut self) -> Result<&mut SampleTableWriter, Error> {
        match &mut self.movie {
            Some((_movie, samples)) => Ok(samples),
            None => Err(Error::box_out_of_order(MediaDataBox::BOX_TYPE)),
        }
    }

    /// Admits the box `box_type` names into the file where the structure places it
    ///
    /// A box the structure passes over is refused as
    /// [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
    fn admit(&mut self, box_type: BoxType) -> Result<(), Error> {
        match self.structure.handle_box_type(box_type)? {
            NonFragmentedDisposition::FileType
            | NonFragmentedDisposition::Movie
            | NonFragmentedDisposition::MediaData => Ok(()),
            NonFragmentedDisposition::Skip => Err(Error::box_out_of_order(box_type)),
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
        let header = compact_box_header(MediaDataBox::BOX_TYPE, media_data_len)?;

        self.output.frame(header, media_data)
    }
}

impl Default for NonFragmentedMuxFsm {
    fn default() -> Self {
        Self::new()
    }
}

/// Brands the mux FSM declares where none were handed over, those the widest layout it lays down requires
fn default_file_type() -> FileTypeBox {
    FileTypeBox::new(FourCC::new(*b"iso4"), 0, alloc::vec![FourCC::new(*b"iso4")])
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox, SampleFlags};
    use isobmff_core::{BoxDecode, BoxDefinition};
    use isobmff_sample::Sample;
    use isobmff_test_support::{file_type, unfragmented_movie};

    use super::{Error, NonFragmentedMuxFsm, default_file_type};
    use crate::ErrorKind;

    /// A sample of the track the movie declares
    fn sample() -> Sample {
        Sample::new(1, 0, 3_000, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())
    }

    /// The bytes the mux FSM has laid down, drained to the end
    fn drained(mux_fsm: &mut NonFragmentedMuxFsm) -> Vec<u8> {
        let mut file = Vec::new();
        while let Some(written) = mux_fsm.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn a_file_handed_no_brands_opens_with_the_brands_the_mux_fsm_declares() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_movie(unfragmented_movie()).unwrap();
        mux_fsm.finish().unwrap();
        let file = drained(&mut mux_fsm);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((default_file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn the_brands_handed_over_are_laid_down_as_they_stand() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_file_type(file_type()).unwrap();
        mux_fsm.handle_movie(unfragmented_movie()).unwrap();
        mux_fsm.finish().unwrap();
        let file = drained(&mut mux_fsm);

        assert_eq!(
            FileTypeBox::decode(&file).map(|(file_type, rest)| (file_type, rest.get(4..8))),
            Ok((file_type(), Some(b"moov".as_slice())))
        );
    }

    #[test]
    fn a_chunk_no_sample_was_handed_over_to_leaves_no_media_data_box() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_movie(unfragmented_movie()).unwrap();
        mux_fsm.begin_chunk().unwrap();
        mux_fsm.begin_chunk().unwrap();
        mux_fsm.handle_sample(sample()).unwrap();
        mux_fsm.finish().unwrap();
        let file = drained(&mut mux_fsm);

        assert_eq!(file.get(24..28), Some(b"mdat".as_slice()));
        assert_eq!(file.get(36..40), Some(b"moov".as_slice()));
    }

    #[test]
    fn brands_handed_over_after_a_chunk_was_opened_are_out_of_order() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_movie(unfragmented_movie()).unwrap();
        mux_fsm.begin_chunk().unwrap();

        assert_eq!(
            mux_fsm.handle_file_type(file_type()),
            Err(Error::box_out_of_order(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_second_movie_is_rejected() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_movie(unfragmented_movie()).unwrap();

        assert_eq!(
            mux_fsm.handle_movie(unfragmented_movie()),
            Err(Error::duplicate_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_chunk_opened_or_a_sample_handed_over_before_the_movie_is_rejected() {
        let mut opened = NonFragmentedMuxFsm::new();
        let mut handed_a_sample = NonFragmentedMuxFsm::new();

        assert_eq!(
            opened.begin_chunk(),
            Err(Error::box_out_of_order(MediaDataBox::BOX_TYPE))
        );
        assert_eq!(
            handed_a_sample.handle_sample(sample()),
            Err(Error::box_out_of_order(MediaDataBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_sample_handed_over_while_no_chunk_is_open_is_rejected() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_movie(unfragmented_movie()).unwrap();

        assert_eq!(
            mux_fsm.handle_sample(sample()).map_err(Error::kind),
            Err(ErrorKind::Sample(isobmff_sample::ErrorKind::NoChunkOpen))
        );
    }

    #[test]
    fn a_sample_the_movie_does_not_resolve_is_rejected_where_it_is_handed_over() {
        let refused = |sample: Sample| {
            let mut mux_fsm = NonFragmentedMuxFsm::new();
            mux_fsm.handle_movie(unfragmented_movie()).unwrap();
            mux_fsm.begin_chunk().unwrap();

            mux_fsm.handle_sample(sample).map_err(Error::kind)
        };

        assert_eq!(
            refused(Sample::new(
                999,
                0,
                3_000,
                0,
                SampleFlags::ZERO,
                1,
                b"SAMP".to_vec()
            )),
            Err(ErrorKind::Sample(isobmff_sample::ErrorKind::UnknownTrackId))
        );
        assert_eq!(
            refused(Sample::new(
                1,
                0,
                3_000,
                0,
                SampleFlags::ZERO,
                2,
                b"SAMP".to_vec()
            )),
            Err(ErrorKind::Sample(
                isobmff_sample::ErrorKind::UnknownSampleDescriptionIndex
            ))
        );
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_file_type(file_type()).unwrap();

        assert_eq!(
            mux_fsm.finish(),
            Err(Error::missing_mandatory_box(MovieBox::BOX_TYPE))
        );
    }

    #[test]
    fn anything_handed_over_after_finishing_is_rejected() {
        let mut mux_fsm = NonFragmentedMuxFsm::new();

        mux_fsm.handle_file_type(file_type()).unwrap();
        mux_fsm.handle_movie(unfragmented_movie()).unwrap();
        mux_fsm.finish().unwrap();

        assert_eq!(mux_fsm.begin_chunk(), Err(Error::already_finished()));
        assert_eq!(
            mux_fsm.handle_sample(sample()),
            Err(Error::already_finished())
        );
        assert_eq!(mux_fsm.finish(), Err(Error::already_finished()));
    }
}
