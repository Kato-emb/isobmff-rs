//! [`DemuxDriver`] and [`Muxer`], a demux FSM driven over a source that seeks and a mux FSM driven onto a sink

use alloc::vec;
use alloc::vec::Vec;
use std::io::{self, Read, Seek, SeekFrom, Write};

use isobmff_boxes::MovieFragmentRandomAccessOffsetBox;
use isobmff_core::BoxDecode;
use isobmff_sample::Sample;

use crate::Error;
use crate::stack::{CUT_LENGTH, Demux, PollOutput};

/// Drives a demux FSM over a source that seeks, and yields the samples it completes
///
/// The driver carries out what the FSM states, and holds nothing the FSM
/// holds: each [`next`](Iterator::next) takes a sample the FSM completed, or
/// reads for one and asks again — the bytes
/// [`wanted_extent`](Demux::wanted_extent) names, handed to
/// [`handle_data`](Demux::handle_data), or where none is named the file read
/// on from [`input_offset`](Demux::input_offset), handed to
/// [`handle_input`](Demux::handle_input).
///
/// # Contract
///
/// * The file begins where the source stands when the driver is created,
///   and every seek is made from there: a file lying at some position in a
///   larger resource is read by seeking the source to it first. The source is
///   sought only where it does not stand at the offset read next.
/// * A read is one `read` of the source, made again where it is
///   interrupted, of up to 1 MiB and of no more than a want is long: the FSM
///   takes the file cut anywhere, and names again what a short read left
///   lacking.
/// * The samples come as `Iterator` items, in the order the FSM completes
///   them. What the FSM read into values is there to read through
///   [`fsm`](Self::fsm).
/// * The source handing over no byte where the file is read on is the end of
///   the file: the FSM is declared over, the samples it completed come out,
///   then `None` until the reading is resumed. The source handing over no
///   byte where a want lies is [`Io`](crate::ErrorKind::Io) with
///   [`UnexpectedEof`](std::io::ErrorKind::UnexpectedEof): the file was read
///   past that offset before, so the source has shrunk since.
/// * A failure of the source leaves the FSM as it was, and the next call
///   makes the same read again. A failure of the FSM comes after the samples
///   it completed before failing, and is the FSM's to report again for every
///   call after it, as its contract has it.
/// * The FSM is reached between samples through [`fsm_mut`](Self::fsm_mut):
///   the `resume_at` of
///   [`FragmentedDemuxFsm`](isobmff_structure::FragmentedDemuxFsm::resume_at)
///   or [`MediaSegmentDemuxFsm`](isobmff_structure::MediaSegmentDemuxFsm::resume_at)
///   restarts the reading at an offset an index names, and the next call
///   reads from there.
///   [`locate_movie_fragment_random_access`](Self::locate_movie_fragment_random_access)
///   finds the `mfra` closing the file when asked; the driver never seeks an
///   index out on its own.
///
/// # Examples
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::blocking::{DemuxDriver, FragmentedMuxer};
/// use isobmff_sample::Sample;
/// use isobmff_structure::FragmentedDemuxFsm;
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
/// let mut driver = DemuxDriver::new(Cursor::new(file), FragmentedDemuxFsm::new())?;
/// let mut read_back = Vec::new();
/// for sample in &mut driver {
///     read_back.push(sample?.into_data());
/// }
/// assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
///
/// // The brands and the movie the file declared are there to read
/// assert_eq!(driver.fsm().file_type().map(|ftyp| ftyp.major_brand()), Some(file_type().major_brand()));
/// assert_eq!(driver.fsm().movie().map(|moov| moov.trak().len()), Some(1));
/// # Ok::<(), isobmff_io::Error>(())
/// ```
#[derive(Debug)]
pub struct DemuxDriver<S, D> {
    source: S,
    fsm: D,
    origin: u64,
    cursor: Option<u64>,
    buffer: Vec<u8>,
}

impl<S: Read + Seek, D: Demux> DemuxDriver<S, D> {
    /// Creates a driver of `fsm` over `source`
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub fn new(mut source: S, fsm: D) -> Result<Self, Error> {
        let origin = source.stream_position()?;

        Ok(Self {
            source,
            fsm,
            origin,
            cursor: Some(0),
            buffer: vec![0; CUT_LENGTH],
        })
    }

    /// Returns the FSM being driven, for what it read into values
    #[must_use]
    pub const fn fsm(&self) -> &D {
        &self.fsm
    }

    /// Returns the FSM being driven, for its verbs between samples
    #[must_use]
    pub const fn fsm_mut(&mut self) -> &mut D {
        &mut self.fsm
    }

    /// Finds the offset of the `mfra` closing the file, by the `mfro` in its last 16 bytes
    ///
    /// The file ends where the source does, and its last 16 bytes are to be
    /// an `mfro`, whose `size` steps back from the end of the file to where
    /// the `mfra` begins (ISO/IEC 14496-12 §8.8.11); `None` comes back for a
    /// file shorter than that, one closing with no `mfro`, or one whose
    /// `mfro` steps back past its start. That an `mfra` stands at the offset
    /// is not read here: resume at it and read the file to its end, and the
    /// FSM holds the `mfra` read. What stands there instead is read as the
    /// FSM reads any resume point: a `moof` or a `sidx` is read on from, and
    /// a box no index points at is
    /// [`BoxOutOfOrder`](isobmff_structure::ErrorKind::BoxOutOfOrder). The
    /// samples read on from where they stood, the next call seeking the
    /// source back there.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not seek from its
    ///   end, or does not seek or read where the `mfro` lies.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io::Cursor;
    ///
    /// use isobmff_io::blocking::DemuxDriver;
    /// use isobmff_sample::movie_fragment_random_access::sync_sample_at;
    /// use isobmff_structure::FragmentedDemuxFsm;
    /// # use isobmff_test_support::indexed_fragmented_file;
    /// # let file = indexed_fragmented_file();
    /// # let (bytes, time) = (file.bytes, file.fragment_samples[1][0].decode_time());
    /// let mut driver = DemuxDriver::new(Cursor::new(bytes), FragmentedDemuxFsm::new())?;
    ///
    /// // The movie is read as the first sample comes
    /// driver.next().expect("the file carries samples")?;
    ///
    /// // The `mfra` closing the file is read, with no sample coming out of it
    /// let mfra = driver.locate_movie_fragment_random_access()?.expect("the file closes with an mfro");
    /// driver.fsm_mut().resume_at(mfra)?;
    /// assert!(driver.next().is_none());
    ///
    /// // The fragment holding the last sync sample at or before `time` is read from its `moof` on
    /// let tfra = &driver.fsm().movie_fragment_random_access().expect("the mfra has been read").tfra()[0];
    /// let moof_offset = sync_sample_at(tfra, time).expect("a sync sample lies at or before").moof_offset();
    /// driver.fsm_mut().resume_at(moof_offset)?;
    /// let resumed = driver.next().expect("the fragment carries samples")?;
    /// assert_eq!(resumed.decode_time(), time);
    /// # Ok::<(), isobmff_io::Error>(())
    /// ```
    pub fn locate_movie_fragment_random_access(&mut self) -> Result<Option<u64>, Error> {
        self.cursor = None;
        let file_len = self
            .source
            .seek(SeekFrom::End(0))?
            .saturating_sub(self.origin);
        let mut mfro = [0; 16];
        let Some(mfro_start) = file_len.checked_sub(mfro.len() as u64) else {
            return Ok(None);
        };
        // Why not checked_add: the sum is where the source reported its end
        // less the length of an `mfro`, so it lies within 64 bits.
        self.source
            .seek(SeekFrom::Start(self.origin.saturating_add(mfro_start)))?;
        self.source.read_exact(&mut mfro)?;

        Ok(MovieFragmentRandomAccessOffsetBox::decode(&mfro)
            .ok()
            .and_then(|(mfro, _)| mfro.movie_fragment_random_access_start(file_len)))
    }

    /// Reads up to `length` bytes of the file at `offset` into the buffer, seeking the source there unless it stands there, and returns how many came
    fn read_at(&mut self, offset: u64, length: usize) -> io::Result<usize> {
        if self.cursor.take() != Some(offset) {
            let position = self
                .origin
                .checked_add(offset)
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            self.source.seek(SeekFrom::Start(position))?;
        }
        let into = self.buffer.get_mut(..length).unwrap_or_default();
        let read = loop {
            match self.source.read(into) {
                Err(failure) if failure.kind() == io::ErrorKind::Interrupted => {}
                read => break read?,
            }
        };
        self.cursor = u64::try_from(read)
            .ok()
            .and_then(|read| offset.checked_add(read));

        Ok(read)
    }
}

impl<S: Read + Seek, D: Demux> Iterator for DemuxDriver<S, D> {
    type Item = Result<Sample, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.fsm.poll_sample() {
                return Some(Ok(sample));
            }
            let handed = if let Some(wanted) = self.fsm.wanted_extent() {
                let length = usize::try_from(wanted.end.saturating_sub(wanted.start))
                    .map_or(CUT_LENGTH, |length| length.min(CUT_LENGTH));
                match self.read_at(wanted.start, length) {
                    Err(failure) => Err(failure.into()),
                    Ok(0) => Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
                    Ok(read) => self
                        .fsm
                        .handle_data(wanted.start, self.buffer.get(..read).unwrap_or_default())
                        .map_err(Error::from),
                }
            } else {
                match self.read_at(self.fsm.input_offset(), CUT_LENGTH) {
                    Err(failure) => Err(failure.into()),
                    Ok(0) => match self.fsm.finish() {
                        Err(failure)
                            if failure.kind() == isobmff_structure::ErrorKind::AlreadyFinished =>
                        {
                            return None;
                        }
                        finished => finished.map_err(Error::from),
                    },
                    Ok(read) => self
                        .fsm
                        .handle_input(self.buffer.get(..read).unwrap_or_default())
                        .map_err(Error::from),
                }
            };
            if let Err(failure) = handed {
                return Some(self.fsm.poll_sample().ok_or(failure));
            }
        }
    }
}

/// A writing stack driven onto a sink, a step at a time
///
/// What every muxer is beneath its own name: each verb is a step of the
/// writer, and what the writer made of it is written to the sink before the
/// step reports. The contract is each muxer's own.
#[derive(Debug)]
pub(crate) struct Muxer<S, W> {
    sink: S,
    writer: W,
}

impl<S: Write, W: PollOutput> Muxer<S, W> {
    /// Creates a muxer writing to `sink` what `writer` makes of each step
    pub(crate) const fn new(sink: S, writer: W) -> Self {
        Self { sink, writer }
    }

    /// Makes `step` of the writer, and writes what the writer made of it whether it failed or not
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what the writer
    ///   makes of the step, reported ahead of the sink's failure; the bytes
    ///   made before the refusal reach the sink all the same.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub(crate) fn drive(
        &mut self,
        step: impl FnOnce(&mut W) -> Result<(), isobmff_structure::Error>,
    ) -> Result<(), Error> {
        let stepped = step(&mut self.writer);
        let mut written = Ok(());
        while let Some(bytes) = self.writer.poll_output() {
            written = written.and_then(|()| self.sink.write_all(&bytes));
        }
        stepped?;
        written?;

        Ok(())
    }

    /// Makes `step` of the writer as the last, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what the writer
    ///   makes of the step.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub(crate) fn finish(
        &mut self,
        step: impl FnOnce(&mut W) -> Result<(), isobmff_structure::Error>,
    ) -> Result<(), Error> {
        self.drive(step)?;
        self.sink.flush()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use std::io::{self, Read, Seek, SeekFrom, Write};

    use isobmff_boxes::{
        MovieFragmentBox, MovieFragmentHeaderBox, MovieFragmentRandomAccessBox,
        MovieFragmentRandomAccessOffsetBox, SampleFlags, TrackFragmentBox, TrackFragmentHeaderBox,
        TrackFragmentHeaderFlags, TrackRunBox, TrackRunSample,
    };
    use isobmff_core::{BoxDecode, BoxType};
    use isobmff_sample::Sample;
    use isobmff_structure::{FragmentedDemuxFsm, MediaSegmentDemuxFsm, NonFragmentedDemuxFsm};
    use isobmff_test_support::{
        SAMPLE_DURATION, fragmented_file_samples, fragmented_file_with_samples,
        indexed_fragmented_file, non_fragmented_file, presentation_movie, segment_file_samples,
        segment_file_with_samples, written,
    };

    use super::{CUT_LENGTH, DemuxDriver, Muxer};

    use crate::stack::ResumeSamples;
    use crate::stack::tests::{Queued, Scripted, framed, sample};
    use crate::{Error, ErrorKind};

    /// What a source was asked to do
    #[derive(PartialEq, Debug)]
    enum Call {
        Seek(SeekFrom),
        Read(usize),
    }

    /// Source recording every seek and read made of it, failing the first read when asked to
    struct Recorded {
        file: io::Cursor<Vec<u8>>,
        calls: Vec<Call>,
        failing: bool,
    }

    impl Recorded {
        fn new(file: Vec<u8>) -> Self {
            Self {
                file: io::Cursor::new(file),
                calls: Vec::new(),
                failing: false,
            }
        }
    }

    impl Read for Recorded {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            self.calls.push(Call::Read(into.len()));
            if self.failing {
                self.failing = false;

                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }

            self.file.read(into)
        }
    }

    impl Seek for Recorded {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if from != SeekFrom::Current(0) {
                self.calls.push(Call::Seek(from));
            }

            self.file.seek(from)
        }
    }

    /// Source holding nothing past any position it is sought back to
    struct Shrinking(io::Cursor<Vec<u8>>);

    impl Read for Shrinking {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            self.0.read(into)
        }
    }

    impl Seek for Shrinking {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if let SeekFrom::Start(position) = from {
                if position < self.0.position() {
                    self.0
                        .get_mut()
                        .truncate(usize::try_from(position).unwrap());
                }
            }

            self.0.seek(from)
        }
    }

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

    /// Sink recording what was written to it, and whether it was flushed
    #[derive(Default, PartialEq, Debug)]
    struct Recording {
        written: Vec<u8>,
        flushed: bool,
    }

    impl Write for Recording {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(bytes);

            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;

            Ok(())
        }
    }

    /// What `driver` yields, kind for kind, until it returns `None`
    fn yielded<D: super::Demux>(
        driver: &mut DemuxDriver<impl Read + Seek, D>,
    ) -> Vec<Result<Sample, ErrorKind>> {
        driver
            .map(|sample| sample.map_err(|failure| failure.kind()))
            .collect()
    }

    /// What the next call to `driver` yields, kind for kind
    fn next_yielded<D: super::Demux>(
        driver: &mut DemuxDriver<impl Read + Seek, D>,
    ) -> Option<Result<Sample, ErrorKind>> {
        driver
            .next()
            .map(|sample| sample.map_err(|failure| failure.kind()))
    }

    /// The samples read to the end of `file`, driving `fsm`
    fn read_back<D: super::Demux>(file: Vec<u8>, fsm: D) -> Vec<Sample> {
        DemuxDriver::new(io::Cursor::new(file), fsm)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// `file`, followed by a fragment of `sequence_number` addressing the bytes of the last of `samples` again
    fn addressing_back(mut file: Vec<u8>, samples: &[Sample], sequence_number: u32) -> Vec<u8> {
        let sample_len = samples.last().map(|sample| sample.data().len()).unwrap();
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
        let fragment = MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(sequence_number),
            vec![track_fragment],
        );
        file.extend_from_slice(&written(&fragment));

        file
    }

    /// `samples`, and the last of them again, a sample duration later
    fn with_the_last_read_again(mut samples: Vec<Sample>) -> Vec<Sample> {
        let repeated = samples.last().unwrap();
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
        samples.push(read_again);

        samples
    }

    #[test]
    fn a_want_is_read_where_it_lies_and_the_file_read_on_from_the_input_offset_with_no_seek_where_the_source_stands()
     {
        let mut driver = DemuxDriver::new(
            Recorded::new(b"FILEBYTES".to_vec()),
            Scripted {
                wanted: Some(2..6),
                input_offset: 4,
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(
            driver.source.calls,
            [
                Call::Seek(SeekFrom::Start(2)),
                Call::Read(4),
                Call::Seek(SeekFrom::Start(4)),
                Call::Read(CUT_LENGTH),
                Call::Read(CUT_LENGTH),
                Call::Read(CUT_LENGTH),
            ]
        );
        assert_eq!(driver.fsm().data, [(2, b"LEBY".to_vec())]);
        assert_eq!(driver.fsm().inputs, [b"BYTES".to_vec()]);
    }

    #[test]
    fn a_want_longer_than_the_buffer_is_read_a_buffer_at_a_time() {
        let file = vec![0x11; CUT_LENGTH.saturating_add(8)];
        let mut driver = DemuxDriver::new(
            io::Cursor::new(file.clone()),
            Scripted {
                wanted: Some(0..u64::try_from(file.len()).unwrap()),
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(driver.fsm().data, [(0, vec![0x11; CUT_LENGTH])]);
    }

    #[test]
    fn the_file_begins_where_the_source_stands() {
        let mut source = io::Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut driver = DemuxDriver::new(
            source,
            Scripted {
                wanted: Some(1..3),
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec()]);
        assert_eq!(driver.fsm().data, [(1, b"IL".to_vec())]);
    }

    #[test]
    fn a_source_shrunk_below_a_want_is_reported_as_ending() {
        let mut driver = DemuxDriver::new(
            Shrinking(io::Cursor::new(b"FILE".to_vec())),
            Scripted::default(),
        )
        .unwrap();
        driver.next();
        driver.fsm_mut().wanted = Some(0..4);

        assert_eq!(
            next_yielded(&mut driver),
            Some(Err(ErrorKind::Io(io::ErrorKind::UnexpectedEof)))
        );
    }

    #[test]
    fn the_end_of_the_source_declares_the_file_over_and_the_samples_end_until_a_resume() {
        let mut driver =
            DemuxDriver::new(io::Cursor::new(b"FILE".to_vec()), Scripted::default()).unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert!(driver.fsm().finished);
        assert!(driver.next().is_none());

        driver.fsm_mut().resume_at(2).unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec(), b"LE".to_vec()]);
    }

    #[test]
    fn the_samples_completed_before_the_fsm_fails_come_first() {
        let failure = isobmff_structure::Error::missing_mandatory_box(BoxType::compact(*b"moov"));
        let mut driver = DemuxDriver::new(
            io::Cursor::new(b"FILE".to_vec()),
            Scripted {
                completed_by_input: vec![sample(b"S1"), sample(b"S2")],
                finish: Some(failure),
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(
            yielded(&mut driver),
            [
                Ok(sample(b"S1")),
                Ok(sample(b"S2")),
                Err(ErrorKind::Structure(failure.kind())),
            ]
        );
    }

    #[test]
    fn a_source_failing_a_read_has_the_same_read_made_by_the_next_call() {
        let mut source = Recorded::new(b"FILE".to_vec());
        source.failing = true;
        let mut driver = DemuxDriver::new(
            source,
            Scripted {
                completed_by_input: vec![sample(b"S1")],
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(
            next_yielded(&mut driver),
            Some(Err(ErrorKind::Io(io::ErrorKind::TimedOut)))
        );
        assert_eq!(yielded(&mut driver), [Ok(sample(b"S1"))]);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec()]);
        assert_eq!(
            driver.source.calls,
            [
                Call::Read(CUT_LENGTH),
                Call::Seek(SeekFrom::Start(0)),
                Call::Read(CUT_LENGTH),
                Call::Read(CUT_LENGTH),
                Call::Read(CUT_LENGTH),
            ]
        );
    }

    #[test]
    fn an_input_offset_past_what_a_seek_names_is_refused() {
        let mut source = io::Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut driver = DemuxDriver::new(source, Scripted::default()).unwrap();
        driver.fsm_mut().resume_at(u64::MAX).unwrap();

        assert_eq!(
            next_yielded(&mut driver),
            Some(Err(ErrorKind::Io(io::ErrorKind::InvalidInput)))
        );
    }

    #[test]
    fn a_fragment_addressing_media_data_before_it_has_the_bytes_read() {
        let file = addressing_back(
            fragmented_file_with_samples(),
            &fragmented_file_samples(),
            2,
        );

        assert_eq!(
            read_back(file, FragmentedDemuxFsm::new()),
            with_the_last_read_again(fragmented_file_samples())
        );
    }

    #[test]
    fn a_segment_fragment_addressing_media_data_before_it_has_the_bytes_read() {
        let segment = addressing_back(segment_file_with_samples(), &segment_file_samples(), 3);

        assert_eq!(
            read_back(segment, MediaSegmentDemuxFsm::new(presentation_movie())),
            with_the_last_read_again(segment_file_samples())
        );
    }

    #[test]
    fn a_movie_lying_after_its_media_data_has_the_bytes_read() {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);

        assert_eq!(
            read_back(file, NonFragmentedDemuxFsm::new()),
            [Sample::new(
                1,
                0,
                SAMPLE_DURATION,
                0,
                SampleFlags::ZERO,
                1,
                b"SAMP".to_vec()
            )]
        );
    }

    #[test]
    fn the_samples_a_read_completes_before_the_fsm_fails_on_it_come_before_the_failure() {
        let mut file = non_fragmented_file(&[&[b"SAMP"]], true);
        file.extend_from_slice(b"\0\0\0\x04free");
        let mut driver =
            DemuxDriver::new(io::Cursor::new(file), NonFragmentedDemuxFsm::new()).unwrap();

        assert_eq!(
            [next_yielded(&mut driver), next_yielded(&mut driver)],
            [
                Some(Ok(Sample::new(
                    1,
                    0,
                    SAMPLE_DURATION,
                    0,
                    SampleFlags::ZERO,
                    1,
                    b"SAMP".to_vec()
                ))),
                Some(Err(ErrorKind::Structure(
                    isobmff_structure::ErrorKind::Sequence(isobmff_sequence::ErrorKind::Box(
                        isobmff_core::ErrorKind::SizeBelowHeader
                    ))
                ))),
            ]
        );
    }

    #[test]
    fn a_located_mfra_is_the_box_closing_the_file() {
        let file = indexed_fragmented_file();
        let mut driver = DemuxDriver::new(
            io::Cursor::new(file.bytes.clone()),
            FragmentedDemuxFsm::new(),
        )
        .unwrap();

        let located = driver
            .locate_movie_fragment_random_access()
            .unwrap()
            .unwrap();

        let closing = file.bytes.get(usize::try_from(located).unwrap()..).unwrap();
        let (_mfra, rest) = MovieFragmentRandomAccessBox::decode(closing).unwrap();
        assert_eq!(rest, []);
    }

    #[test]
    fn a_file_closing_with_no_mfro_or_one_stepping_back_past_its_start_or_too_short_for_one_has_none_located()
     {
        let mut reaching_past_the_start = fragmented_file_with_samples();
        reaching_past_the_start
            .extend_from_slice(&written(&MovieFragmentRandomAccessOffsetBox::new(u32::MAX)));

        for file in [
            fragmented_file_with_samples(),
            reaching_past_the_start,
            written(&MovieFragmentRandomAccessOffsetBox::new(0))
                .get(1..)
                .unwrap()
                .to_vec(),
        ] {
            let mut driver =
                DemuxDriver::new(io::Cursor::new(file), FragmentedDemuxFsm::new()).unwrap();

            assert_eq!(driver.locate_movie_fragment_random_access().unwrap(), None);
        }
    }

    #[test]
    fn an_mfro_stepping_back_to_no_mfra_fails_the_resume_as_out_of_order() {
        let mut file = fragmented_file_with_samples();
        file.extend_from_slice(&written(&MovieFragmentRandomAccessOffsetBox::new(24)));
        let mut driver =
            DemuxDriver::new(io::Cursor::new(file), FragmentedDemuxFsm::new()).unwrap();
        driver.next().unwrap().unwrap();

        let located = driver
            .locate_movie_fragment_random_access()
            .unwrap()
            .unwrap();
        driver.fsm_mut().resume_at(located).unwrap();

        assert_eq!(
            next_yielded(&mut driver),
            Some(Err(ErrorKind::Structure(
                isobmff_structure::ErrorKind::BoxOutOfOrder
            )))
        );
    }

    #[test]
    fn a_locate_leaves_the_samples_read_on_from_where_they_stood() {
        let file = indexed_fragmented_file();
        let mut driver = DemuxDriver::new(
            io::Cursor::new(file.bytes.clone()),
            FragmentedDemuxFsm::new(),
        )
        .unwrap();
        let first = driver.next().unwrap().unwrap();

        driver.locate_movie_fragment_random_access().unwrap();

        let read_on: Vec<Sample> = core::iter::once(first)
            .chain(driver.map(Result::unwrap))
            .collect();
        assert_eq!(read_on, file.fragment_samples.concat());
    }

    #[test]
    fn a_source_that_does_not_seek_from_its_end_fails_the_locate_as_the_source_and_the_samples_read_on()
     {
        let file = indexed_fragmented_file();
        let mut driver = DemuxDriver::new(
            Unmeasured(io::Cursor::new(file.bytes)),
            FragmentedDemuxFsm::new(),
        )
        .unwrap();

        assert_eq!(
            driver
                .locate_movie_fragment_random_access()
                .map_err(|failure| failure.kind()),
            Err(ErrorKind::Io(io::ErrorKind::Unsupported))
        );
        assert_eq!(
            driver.collect::<Result<Vec<_>, _>>().unwrap(),
            file.fragment_samples.concat()
        );
    }

    #[test]
    fn the_bytes_a_step_made_before_failing_are_written_and_the_steps_failure_reported() {
        let mut muxer = Muxer::new(Recording::default(), Queued::default());

        let driven = muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Err(isobmff_structure::Error::already_finished())
        });

        assert_eq!(
            driven.map_err(|failure| failure.structure_error()),
            Err(Some(isobmff_structure::Error::already_finished()))
        );
        assert_eq!(
            muxer.sink,
            Recording {
                written: b"\0\0\0\x0cfreeMADE".to_vec(),
                flushed: false,
            }
        );
    }

    #[test]
    fn a_sink_refusing_the_bytes_is_reported_as_the_sink_failing() {
        let mut muxer = Muxer::new(&mut [][..], Queued::default());

        let driven = muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Ok(())
        });

        assert_eq!(
            driven.map_err(|failure| failure.kind()),
            Err(ErrorKind::Io(io::ErrorKind::WriteZero))
        );
    }

    #[test]
    fn the_writers_own_failure_is_reported_ahead_of_the_sinks() {
        let mut muxer = Muxer::new(&mut [][..], Queued::default());

        let driven = muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Err(isobmff_structure::Error::already_finished())
        });

        assert_eq!(
            driven.map_err(|failure| failure.kind()),
            Err(ErrorKind::Structure(
                isobmff_structure::ErrorKind::AlreadyFinished
            ))
        );
    }

    #[test]
    fn finishing_writes_the_last_step_and_flushes_the_sink() {
        let mut muxer = Muxer::new(Recording::default(), Queued::default());

        let finished: Result<(), Error> = muxer.finish(|writer| {
            writer.output.extend(framed(b"LAST"));

            Ok(())
        });

        assert_eq!(finished.map_err(|failure| failure.kind()), Ok(()));
        assert_eq!(
            muxer.sink,
            Recording {
                written: b"\0\0\0\x0cfreeLAST".to_vec(),
                flushed: true,
            }
        );
    }
}
