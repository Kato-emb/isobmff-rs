//! [`FragmentedDemuxer`] and [`FragmentedMuxer`], a fragmented movie file read off a source that seeks and written to a sink, ISO/IEC 14496-12 Annex A.8

use std::io::{Read, Seek, Write};

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentRandomAccessBox};
use isobmff_sample::{Sample, SegmentIndex};
use isobmff_structure::{FragmentedReader, FragmentedWriter};

use super::driver::{Demuxer, Muxer};
use crate::Error;

/// Reads the samples a fragmented movie file carries off a source that seeks
///
/// The driver of [`FragmentedReader`] over `std::io`: it reads the file off
/// the source a cut at a time and hands each over, fetches the bytes the
/// reader names as lacking wherever the file passed them by — the media data
/// of a fragment addressing bytes before it — by seeking to them, and yields
/// the samples as they come whole. A caller takes [`Sample`]s and nothing
/// else moves.
///
/// # Contract
///
/// * The file begins where the source stands when the demuxer is created,
///   and every seek is made from there: a file lying at some position in a
///   larger resource is read by seeking the source to it first.
/// * The samples come as `Iterator` items, in the order the reader completes
///   them — for a fragment followed by the media data it addresses, as the
///   file lays them down. The boxes the reader read into values are there to
///   read once they have come: [`file_type`](Self::file_type) and
///   [`movie`](Self::movie).
/// * The bytes fetched for a want come off the source a cut at a time, as
///   the file does — a cut reaching at least to the end of the want — so
///   every extent the cut reaches is filled by it. A
///   source ending before bytes the reader lacks is the reader's to report
///   at the end of the file, as
///   [`Structure`](crate::ErrorKind::Structure); one ending before
///   where it had already been read to is [`Io`](crate::ErrorKind::Io)
///   with [`UnexpectedEof`](std::io::ErrorKind::UnexpectedEof).
/// * A failure ends the iteration: the samples the reader had completed
///   before it come first, then the failure once, then `None` until the
///   reading is resumed. The end of the file is the same without
///   the failure.
/// * Where an index points is the caller's to choose. The indexes the file
///   carries are there to read once they have come:
///   [`segment_indexes`](Self::segment_indexes) and
///   [`movie_fragment_random_access`](Self::movie_fragment_random_access).
///   [`resume_at`](Self::resume_at) restarts the reading at an offset one of
///   them names, and
///   [`locate_movie_fragment_random_access`](Self::locate_movie_fragment_random_access)
///   finds the `mfra` at the end of the file when asked; the demuxer never
///   seeks an index out on its own.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::blocking::{FragmentedDemuxer, FragmentedMuxer};
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file of one fragment carrying two samples of track 1
/// let mut file = Vec::new();
/// let mut muxer = FragmentedMuxer::new(&mut file);
/// muxer.handle_file_type(file_type())?;
/// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The samples are read off the file as they were laid out
/// let mut demuxer = FragmentedDemuxer::new(Cursor::new(file))?;
/// let mut read_back = Vec::new();
/// for sample in &mut demuxer {
///     read_back.push(sample?.into_data());
/// }
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
///
/// // The brands and the movie the file declared are there to read
/// assert_eq!(demuxer.file_type().map(|ftyp| ftyp.major_brand()), Some(file_type().major_brand()));
/// assert_eq!(demuxer.movie().map(|moov| moov.trak().len()), Some(1));
/// # Ok::<(), isobmff_io::Error>(())
/// ```
#[derive(Debug)]
pub struct FragmentedDemuxer<S> {
    demuxer: Demuxer<S, FragmentedReader>,
}

impl<S: Read + Seek> FragmentedDemuxer<S> {
    /// Creates a demuxer over `source`, the file beginning where it stands
    ///
    /// The reader beneath is [`FragmentedReader::new`]; one holding the file
    /// to other limits is driven through [`with_reader`](Self::with_reader).
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn new(source: S) -> Result<Self, Error> {
        Self::with_reader(source, FragmentedReader::new())
    }

    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn with_reader(source: S, reader: FragmentedReader) -> Result<Self, Error> {
        Ok(Self {
            demuxer: Demuxer::new(source, reader)?,
        })
    }

    /// Returns the brands the file declares itself readable as, once they have come
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.demuxer.reader().file_type()
    }

    /// Returns the movie the fragments of the file continue, once it has come
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.demuxer.reader().movie()
    }

    /// Returns the subsegments of every `sidx` read so far, as [`FragmentedReader::segment_indexes`] holds them
    #[must_use]
    pub fn segment_indexes(&self) -> &[SegmentIndex] {
        self.demuxer.reader().segment_indexes()
    }

    /// Returns the random access tables of the file once its `mfra` has been read, as [`FragmentedReader::movie_fragment_random_access`] holds them
    #[must_use]
    pub const fn movie_fragment_random_access(&self) -> Option<&MovieFragmentRandomAccessBox> {
        self.demuxer.reader().movie_fragment_random_access()
    }

    /// Restarts the reading at `offset` of the file, a place an index names
    ///
    /// The reader is resumed at `offset` as [`FragmentedReader::resume_at`]
    /// resumes it, and the source is sought there from where the file
    /// begins: the samples not yet taken are dropped, and the ones that come
    /// next are those the file carries from `offset` on — the `moof`, `sidx`
    /// or `mfra` it is to start with. The demuxer resumes from reading and from
    /// the end of the file alike, and from a failure of the source. Resuming
    /// before the `moov` has been read leaves the fragments with no movie to
    /// continue, which the reader reports as the file ends.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): `offset`, counted from where the
    ///   source stood when the demuxer was created, lies past `u64::MAX`,
    ///   which leaves the demuxer as it was, or the source does not seek
    ///   there.
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedReader::resume_at`] makes of the call.
    ///
    /// A failure after the offset is checked ends the samples, until a
    /// resume succeeds.
    pub fn resume_at(&mut self, offset: u64) -> Result<(), Error> {
        self.demuxer.resume_at(offset)
    }

    /// Finds the offset of the `mfra` closing the file, by the `mfro` in its last 16 bytes
    ///
    /// The source is read at its end and sought back to where it stood, so
    /// the samples read on as before. The file ends where the source does,
    /// and closes with an `mfra` when its last bytes are an `mfro` whose
    /// `size` steps back to the header of an `mfra` spanning the rest of it
    /// (§8.8.11); `None` comes back otherwise. The `mfra` is not read into a
    /// value here: resume at its offset and read the file to its end, and
    /// [`movie_fragment_random_access`](Self::movie_fragment_random_access)
    /// returns it.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not seek from its
    ///   end, does not seek or read where the `mfro` and the `mfra` lie, or
    ///   does not seek back to where it stood, which ends the samples.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io::Cursor;
    ///
    /// use isobmff_io::blocking::FragmentedDemuxer;
    /// use isobmff_sample::movie_fragment_random_access::sync_sample_at;
    /// # use isobmff_test_support::indexed_fragmented_file;
    /// # let file = indexed_fragmented_file();
    /// # let (bytes, time) = (file.bytes, file.fragment_samples[1][0].decode_time());
    /// let mut demuxer = FragmentedDemuxer::new(Cursor::new(bytes))?;
    ///
    /// // The movie is read as the first sample comes
    /// demuxer.next().expect("the file carries samples")?;
    ///
    /// // The `mfra` closing the file is read, with no sample coming out of it
    /// let mfra = demuxer.locate_movie_fragment_random_access()?.expect("the file closes with an mfra");
    /// demuxer.resume_at(mfra)?;
    /// assert!(demuxer.next().is_none());
    ///
    /// // The fragment holding the last sync sample at or before `time` is read from its `moof` on
    /// let tfra = &demuxer.movie_fragment_random_access().expect("the mfra has been read").tfra()[0];
    /// let sync_sample = sync_sample_at(tfra, time).expect("a sync sample lies at or before");
    /// demuxer.resume_at(sync_sample.moof_offset())?;
    /// let resumed = demuxer.next().expect("the fragment carries samples")?;
    /// assert_eq!(resumed.decode_time(), time);
    /// # Ok::<(), isobmff_io::Error>(())
    /// ```
    pub fn locate_movie_fragment_random_access(&mut self) -> Result<Option<u64>, Error> {
        self.demuxer.locate_movie_fragment_random_access()
    }
}

impl<S: Read + Seek> Iterator for FragmentedDemuxer<S> {
    type Item = Result<Sample, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.demuxer.next()
    }
}

/// Lays a fragmented movie file down on a sink, taking the samples as they come
///
/// The driver of [`FragmentedWriter`] over `std::io`: it takes the boxes and
/// the samples as the writer does, and writes every byte the writer makes of
/// them to the sink before the call returns. A caller hands over boxes and
/// samples and nothing else moves.
///
/// # Contract
///
/// * The calls are the writer's, and what each takes and refuses is
///   [`FragmentedWriter`]'s contract, carried through as
///   [`Structure`](crate::ErrorKind::Structure). What the writer made
///   of a call is written before the call reports, the bytes made before a
///   refusal included; a sink refusing them is
///   [`Io`](crate::ErrorKind::Io), unless the writer refused the call
///   too, whose failure is the one reported. The sink is written to a box
///   header or a box payload at a time, as the writer hands them over — the
///   media data of a fragment whole; one that is costly to write to in small
///   pieces is the caller's to wrap in a `BufWriter`.
/// * [`finish`](Self::finish) declares the file over and flushes the sink.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::blocking::FragmentedMuxer;
/// use isobmff_sample::Sample;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// // A file opening with its brands and the movie its fragments continue
/// let mut file = Vec::new();
/// let mut muxer = FragmentedMuxer::new(&mut file);
/// muxer.handle_file_type(file_type())?;
/// muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO)))?;
///
/// // One fragment of two samples of track 1, written to the file as it is closed
/// muxer.begin_fragment(1)?;
/// muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec()))?;
/// muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec()))?;
/// muxer.finish_fragment()?;
/// muxer.finish()?;
///
/// // The file opens with the brands, and the media data holds the samples end to end
/// assert_eq!(&file[4..8], b"ftyp");
/// assert!(file.ends_with(b"SAMPDATA"));
/// # Ok::<(), isobmff_io::Error>(())
/// ```
#[derive(Debug)]
pub struct FragmentedMuxer<W> {
    muxer: Muxer<W, FragmentedWriter>,
}

impl<W: Write> FragmentedMuxer<W> {
    /// Creates a muxer writing to `sink`, waiting at the start of a fragmented movie file
    #[must_use]
    pub const fn new(sink: W) -> Self {
        Self {
            muxer: Muxer::new(sink, FragmentedWriter::new()),
        }
    }

    /// Takes the brands the file declares itself readable as, and writes them
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedWriter::handle_file_type`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn handle_file_type(&mut self, file_type: FileTypeBox) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.handle_file_type(file_type))
    }

    /// Takes the movie the fragments continue, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedWriter::handle_movie`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_movie(movie))
    }

    /// Opens a fragment, which the samples handed over next are laid out in
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedWriter::begin_fragment`] makes of the call.
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment(sequence_number))
    }

    /// Opens a fragment in which every track continues where the samples written for it reach
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedWriter::begin_fragment_continuing`] makes of the call.
    pub fn begin_fragment_continuing(&mut self, sequence_number: u32) -> Result<(), Error> {
        self.muxer
            .drive(|writer| writer.begin_fragment_continuing(sequence_number))
    }

    /// Takes a sample, and places it in the fragment that is open
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedWriter::handle_sample`] makes of the call.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.muxer.drive(|writer| writer.handle_sample(sample))
    }

    /// Closes the fragment that is open, and writes it
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedWriter::finish_fragment`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        self.muxer.drive(FragmentedWriter::finish_fragment)
    }

    /// Declares the file over, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what
    ///   [`FragmentedWriter::finish`] makes of the call.
    /// * [`Io`](crate::ErrorKind::Io): the sink does not flush.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish(FragmentedWriter::finish)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use std::io::{self, Read, Seek, SeekFrom};

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, MovieFragmentRandomAccessBox,
        MovieFragmentRandomAccessOffsetBox, SampleFlags, TrackFragmentBox, TrackFragmentHeaderBox,
        TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
    };
    use isobmff_core::BoxDecode;
    use isobmff_sample::Sample;
    use isobmff_test_support::{
        file_type, fragmented_file_samples, fragmented_file_with_samples, indexed_fragmented_file,
        written,
    };

    use super::{FragmentedDemuxer, FragmentedMuxer};
    use crate::ErrorKind;

    /// Source that does not seek from its end
    struct Unmeasured(io::Cursor<Vec<u8>>);

    impl Read for Unmeasured {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            self.0.read(into)
        }
    }

    impl Seek for Unmeasured {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if let SeekFrom::End(_) = from {
                return Err(io::Error::from(io::ErrorKind::Unsupported));
            }

            self.0.seek(from)
        }
    }

    /// The fragmented file, followed by a fragment addressing the bytes of its last sample again
    fn file_addressing_back() -> Vec<u8> {
        let mut file = fragmented_file_with_samples();
        let sample_len = fragmented_file_samples()
            .last()
            .map(|sample| sample.data().len())
            .unwrap();
        let media_data_start = u64::try_from(file.len().saturating_sub(sample_len)).unwrap();
        let track_fragment = TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(
                TrackFragmentHeaderFlags::ZERO,
                1,
                Some(media_data_start),
                None,
                None,
                Some(u32::try_from(sample_len).unwrap()),
                None,
            ),
            vec![
                TrackRunBox::new(
                    Some(0),
                    None,
                    vec![TrackRunSample::new(None, None, None, None)],
                )
                .unwrap(),
            ],
        );
        let fragment = MovieFragmentBox::new(MovieFragmentHeaderBox::new(2), vec![track_fragment]);
        file.extend_from_slice(&written(&fragment));

        file
    }

    #[test]
    fn a_fragment_addressing_media_data_before_it_has_the_bytes_fetched() {
        let read_back: Vec<Sample> =
            FragmentedDemuxer::new(io::Cursor::new(file_addressing_back()))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();

        let mut expected = fragmented_file_samples();
        let repeated = expected.last().unwrap();
        let read_again = Sample::new(
            1,
            repeated
                .decode_time()
                .saturating_add(u64::from(repeated.sample_duration())),
            repeated.sample_duration(),
            0,
            SampleFlags::ZERO,
            1,
            repeated.data().to_vec(),
        );
        expected.push(read_again);

        assert_eq!(read_back, expected);
    }

    #[test]
    fn a_file_closing_with_no_mfra_has_none_located() {
        let mut demuxer =
            FragmentedDemuxer::new(io::Cursor::new(fragmented_file_with_samples())).unwrap();

        assert_eq!(demuxer.locate_movie_fragment_random_access().unwrap(), None);
    }

    #[test]
    fn a_located_mfra_is_the_box_closing_the_file() {
        let file = indexed_fragmented_file();
        let mut demuxer = FragmentedDemuxer::new(io::Cursor::new(file.bytes.clone())).unwrap();

        let located = demuxer
            .locate_movie_fragment_random_access()
            .unwrap()
            .unwrap();

        let closing = file.bytes.get(usize::try_from(located).unwrap()..).unwrap();
        let (_mfra, rest) = MovieFragmentRandomAccessBox::decode(closing).unwrap();
        assert_eq!(rest, []);
    }

    #[test]
    fn an_mfro_whose_size_steps_back_to_no_mfra_has_none_located() {
        let mut file = fragmented_file_with_samples();
        file.extend_from_slice(&written(&MovieFragmentRandomAccessOffsetBox::new(24)));
        let mut reaching_past_the_start = fragmented_file_with_samples();
        reaching_past_the_start
            .extend_from_slice(&written(&MovieFragmentRandomAccessOffsetBox::new(u32::MAX)));

        for file in [file, reaching_past_the_start] {
            let mut demuxer = FragmentedDemuxer::new(io::Cursor::new(file)).unwrap();

            assert_eq!(demuxer.locate_movie_fragment_random_access().unwrap(), None);
        }
    }

    #[test]
    fn a_source_that_does_not_seek_from_its_end_fails_the_locate_as_the_source_and_the_samples_read_on()
     {
        let mut demuxer =
            FragmentedDemuxer::new(Unmeasured(io::Cursor::new(indexed_fragmented_file().bytes)))
                .unwrap();

        assert_eq!(
            demuxer
                .locate_movie_fragment_random_access()
                .map_err(|failure| failure.kind()),
            Err(ErrorKind::Io(io::ErrorKind::Unsupported))
        );
        assert_eq!(
            demuxer.collect::<Result<Vec<_>, _>>().unwrap(),
            indexed_fragmented_file().fragment_samples.concat()
        );
    }

    #[test]
    fn the_bytes_the_writer_made_of_a_call_are_written_before_the_call_reports() {
        let mut file = Vec::new();
        let mut muxer = FragmentedMuxer::new(&mut file);

        muxer.handle_file_type(file_type()).unwrap();

        assert_eq!(file, written(&file_type()));
    }
}
