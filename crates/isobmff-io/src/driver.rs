//! [`DemuxDriver`], a demux FSM driven over an asynchronous source that seeks, and [`Muxer`], what every muxer over `futures::io` does the same way whatever the structure
//!
//! The drivers over `std::io` are written apart, in
//! [`blocking`](crate::blocking); both drive their FSM through the verbs of
//! `crate::stack`.

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::future::poll_fn;
use core::pin::Pin;
use std::io::{self, SeekFrom};

use futures_io::{AsyncRead, AsyncSeek, AsyncWrite};
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;

use crate::Error;
use crate::stack::{CUT_LENGTH, ClosingMovieFragmentRandomAccessOffset, Demux, Mux, Request};

/// Reads off `source` into `into`, and returns how many bytes came
async fn read<S: AsyncRead + Unpin>(source: &mut S, into: &mut [u8]) -> io::Result<usize> {
    poll_fn(|context| Pin::new(&mut *source).poll_read(context, into)).await
}

/// Moves `source` to `position`, and returns where it stands
async fn seek<S: AsyncSeek + Unpin>(source: &mut S, position: SeekFrom) -> io::Result<u64> {
    poll_fn(|context| Pin::new(&mut *source).poll_seek(context, position)).await
}

/// Drives a demux FSM over an asynchronous source that seeks, and hands over the samples it completes
///
/// The driver carries out what the FSM states, and holds nothing the FSM
/// holds: each [`next`](Self::next) takes a sample the FSM completed, or
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
/// * The samples come out of [`next`](Self::next), in the order the FSM
///   completes them. What the FSM read into values is there to read through
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
/// * Every `async fn` here is cancellation safe. What one read gave reaches
///   the FSM before the next await, and the driver trusts where the source
///   stands only once a seek or a read there has completed, so a
///   [`next`](Self::next) dropped where the source stood still is carried on
///   by the call that follows, with no byte read twice and none lost. A
///   [`locate_movie_fragment_random_access`](Self::locate_movie_fragment_random_access)
///   dropped part way is made again from its start by the call that repeats
///   it.
///
/// # Examples
///
/// ```
/// use futures_executor::block_on;
/// use futures_util::io::Cursor;
///
/// use isobmff_boxes::{SampleFlags, TrackExtendsBox};
/// use isobmff_io::{DemuxDriver, FragmentedMuxer};
/// use isobmff_sample::Sample;
/// use isobmff_structure::FragmentedDemuxFsm;
/// # use isobmff_test_support::{file_type, fragmented_movie};
/// block_on(async {
///     // A file of one fragment carrying two samples of track 1
///     let mut file = Vec::new();
///     let mut muxer = FragmentedMuxer::new(&mut file);
///     muxer.handle_file_type(file_type()).await?;
///     muxer.handle_movie(fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, SampleFlags::ZERO))).await?;
///     muxer.begin_fragment(1).await?;
///     muxer.handle_sample(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"SAMP".to_vec())).await?;
///     muxer.handle_sample(Sample::new(1, 1_024, 1_024, 0, SampleFlags::ZERO, 1, b"DATA".to_vec())).await?;
///     muxer.finish_fragment().await?;
///     muxer.finish().await?;
///
///     // The samples are read off the file as they were laid out
///     let mut driver = DemuxDriver::new(Cursor::new(file), FragmentedDemuxFsm::new()).await?;
///     let mut read_back = Vec::new();
///     while let Some(sample) = driver.next().await {
///         read_back.push(sample?.into_data());
///     }
///     assert_eq!(read_back, [b"SAMP".to_vec(), b"DATA".to_vec()]);
///
///     // The brands and the movie the file declared are there to read
///     assert_eq!(driver.fsm().file_type().map(|ftyp| ftyp.major_brand()), Some(file_type().major_brand()));
///     assert_eq!(driver.fsm().movie().map(|moov| moov.trak().len()), Some(1));
/// #   Ok::<(), isobmff_io::Error>(())
/// })
/// # .unwrap();
/// ```
#[derive(Debug)]
pub struct DemuxDriver<S, D> {
    source: S,
    fsm: D,
    origin: u64,
    cursor: Option<u64>,
    buffer: Vec<u8>,
}

impl<S: AsyncRead + AsyncSeek + Unpin, D: Demux> DemuxDriver<S, D> {
    /// Creates a driver of `fsm` over `source`
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub async fn new(mut source: S, fsm: D) -> Result<Self, Error> {
        let origin = seek(&mut source, SeekFrom::Current(0)).await?;

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

    /// Takes the next sample the file carries, reading on until one comes
    pub async fn next(&mut self) -> Option<Result<Sample, Error>> {
        loop {
            if let Some(sample) = self.fsm.poll_sample() {
                return Some(Ok(sample));
            }
            let request = Request::of(&self.fsm);
            let handed = self
                .read_at(request.offset, request.length)
                .await
                .map_err(Error::from)
                .and_then(|read| {
                    request.hand_over(&mut self.fsm, self.buffer.get(..read).unwrap_or_default())
                });
            match handed {
                Ok(true) => {}
                Ok(false) => return None,
                Err(failure) => return Some(self.fsm.poll_sample().ok_or(failure)),
            }
        }
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
    /// use futures_executor::block_on;
    /// use futures_util::io::Cursor;
    ///
    /// use isobmff_io::DemuxDriver;
    /// use isobmff_sample::movie_fragment_random_access::sync_sample_at;
    /// use isobmff_structure::FragmentedDemuxFsm;
    /// # use isobmff_test_support::indexed_fragmented_file;
    /// # let file = indexed_fragmented_file();
    /// # let (bytes, time) = (file.bytes, file.fragment_samples[1][0].decode_time());
    /// block_on(async {
    ///     // A driver over a file closing with an `mfra`
    ///     let mut driver = DemuxDriver::new(Cursor::new(bytes), FragmentedDemuxFsm::new()).await?;
    ///
    ///     // The movie is read as the first sample comes
    ///     driver.next().await.expect("the file carries samples")?;
    ///
    ///     // The `mfra` closing the file is read, with no sample coming out of it
    ///     let mfra = driver.locate_movie_fragment_random_access().await?.expect("the file closes with an mfro");
    ///     driver.fsm_mut().resume_at(mfra)?;
    ///     assert!(driver.next().await.is_none());
    ///
    ///     // The fragment holding the last sync sample at or before `time` is read from its `moof` on
    ///     let tfra = &driver.fsm().movie_fragment_random_access().expect("the mfra has been read").tfra()[0];
    ///     let moof_offset = sync_sample_at(tfra, time).expect("a sync sample lies at or before").moof_offset();
    ///     driver.fsm_mut().resume_at(moof_offset)?;
    ///     let resumed = driver.next().await.expect("the fragment carries samples")?;
    ///     assert_eq!(resumed.decode_time(), time);
    /// #   Ok::<(), isobmff_io::Error>(())
    /// })
    /// # .unwrap();
    /// ```
    pub async fn locate_movie_fragment_random_access(&mut self) -> Result<Option<u64>, Error> {
        self.cursor = None;
        let file_len = seek(&mut self.source, SeekFrom::End(0))
            .await?
            .saturating_sub(self.origin);
        let Some(closing) = ClosingMovieFragmentRandomAccessOffset::of(file_len) else {
            return Ok(None);
        };
        let position = SeekFrom::Start(closing.position(self.origin));
        seek(&mut self.source, position).await?;
        let mut mfro = [0; ClosingMovieFragmentRandomAccessOffset::LEN];
        let mut filled = 0;
        while let Some(into) = mfro.get_mut(filled..).filter(|into| !into.is_empty()) {
            match read(&mut self.source, into).await {
                Err(failure) if failure.kind() == io::ErrorKind::Interrupted => {}
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
                Ok(read) => filled = filled.saturating_add(read),
                Err(failure) => return Err(failure.into()),
            }
        }

        Ok(closing.movie_fragment_random_access_start(&mfro))
    }

    /// Reads up to `length` bytes of the file at `offset` into the buffer, seeking the source there unless it stands there, and returns how many came
    async fn read_at(&mut self, offset: u64, length: usize) -> io::Result<usize> {
        if self.cursor != Some(offset) {
            self.cursor = None;
            let position = self
                .origin
                .checked_add(offset)
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            seek(&mut self.source, SeekFrom::Start(position)).await?;
            self.cursor = Some(offset);
        }
        let into = self.buffer.get_mut(..length).unwrap_or_default();
        let read = loop {
            match read(&mut self.source, into).await {
                Err(failure) if failure.kind() == io::ErrorKind::Interrupted => {}
                read => break read,
            }
        };
        self.cursor = read
            .as_ref()
            .ok()
            .and_then(|read| u64::try_from(*read).ok())
            .and_then(|read| offset.checked_add(read));

        read
    }
}

/// A writing stack driven onto an asynchronous sink, a step at a time
///
/// What every asynchronous muxer is beneath its own name: each verb is a step
/// of the writer, and what the writer made of it is written to the sink
/// before the step reports. The contract is each muxer's own, with this much
/// of it held here: a verb makes its step before it awaits anything and keeps
/// what the writer made and what it refused, so a verb whose future is
/// dropped has made its step and no more — the verb that follows writes what
/// was left over ahead of its own bytes, and reports the refusal the dropped
/// one was carrying instead of making a step of its own.
#[derive(Debug)]
pub(crate) struct Muxer<S, W> {
    sink: S,
    writer: W,
    pending: VecDeque<EventBytes>,
    taken: usize,
    refusal: Option<isobmff_structure::Error>,
    finished: bool,
}

impl<S: AsyncWrite + Unpin, W: Mux> Muxer<S, W> {
    /// Creates a muxer writing to `sink` what `writer` makes of each step
    pub(crate) const fn new(sink: S, writer: W) -> Self {
        Self {
            sink,
            writer,
            pending: VecDeque::new(),
            taken: 0,
            refusal: None,
            finished: false,
        }
    }

    /// Makes `step` of the writer, and writes what the writer made of it whether it failed or not
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what the writer
    ///   makes of the step, reported ahead of the sink's failure; the bytes
    ///   made before the refusal reach the sink all the same.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes.
    pub(crate) async fn drive(
        &mut self,
        step: impl FnOnce(&mut W) -> Result<(), isobmff_structure::Error>,
    ) -> Result<(), Error> {
        if self.refusal.is_none() {
            self.make(step);
        }

        self.write_pending().await
    }

    /// Makes `step` of the writer as the last if no call made it yet, and flushes the sink
    ///
    /// # Errors
    ///
    /// * [`Structure`](crate::ErrorKind::Structure): what the writer
    ///   makes of the step.
    /// * [`Io`](crate::ErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub(crate) async fn finish(
        &mut self,
        step: impl FnOnce(&mut W) -> Result<(), isobmff_structure::Error>,
    ) -> Result<(), Error> {
        if self.refusal.is_none() && !self.finished {
            self.finished = true;
            self.make(step);
        }
        self.write_pending().await?;
        poll_fn(|context| Pin::new(&mut self.sink).poll_flush(context)).await?;

        Ok(())
    }

    /// Makes `step` of the writer, keeping the bytes it made and the refusal it reported
    fn make(&mut self, step: impl FnOnce(&mut W) -> Result<(), isobmff_structure::Error>) {
        self.refusal = step(&mut self.writer).err();
        while let Some(chunk) = self.writer.poll_output() {
            self.pending.push_back(chunk);
        }
    }

    /// Writes what is pending, and reports the writer's refusal ahead of the sink's failure
    async fn write_pending(&mut self) -> Result<(), Error> {
        let written = self.drain().await;
        if let Some(refusal) = self.refusal.take() {
            return Err(refusal.into());
        }
        written?;

        Ok(())
    }

    /// Hands the sink what the writer made and it has not taken yet, a piece of a chunk at a time
    async fn drain(&mut self) -> io::Result<()> {
        while let Some(chunk) = self.pending.front() {
            let Some(rest) = chunk.get(self.taken..).filter(|rest| !rest.is_empty()) else {
                self.pending.pop_front();
                self.taken = 0;

                continue;
            };

            // Why not one `write` for the whole chunk: a future dropped in it
            // would leave the sink holding a part no count of the driver's
            // names, where `poll_write` takes bytes only as it reports them,
            // and the count moves before the next await.
            let written =
                poll_fn(|context| Pin::new(&mut self.sink).poll_write(context, rest)).await?;
            if written == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero));
            }
            self.taken = self.taken.saturating_add(written);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::future::Future;
    use core::pin::{Pin, pin};
    use core::task::{Context, Poll, Waker};
    use std::io::{self, SeekFrom};

    use futures_executor::block_on;
    use futures_io::{AsyncRead, AsyncSeek, AsyncWrite};
    use futures_util::io::Cursor;
    use isobmff_boxes::{MovieFragmentRandomAccessBox, SampleFlags};
    use isobmff_core::BoxType;
    use isobmff_sample::Sample;
    use isobmff_structure::NonFragmentedDemuxFsm;
    use isobmff_test_support::{SAMPLE_DURATION, non_fragmented_file, written};

    use super::{DemuxDriver, Muxer};

    use crate::stack::tests::{Queued, Scripted, framed, sample};
    use crate::stack::{CUT_LENGTH, Demux};
    use crate::{Error, ErrorKind};

    /// Source holding nothing past any position it is sought back to
    struct Shrinking(Cursor<Vec<u8>>);

    impl AsyncRead for Shrinking {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            into: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.0).poll_read(context, into)
        }
    }

    impl AsyncSeek for Shrinking {
        fn poll_seek(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            from: SeekFrom,
        ) -> Poll<io::Result<u64>> {
            if let SeekFrom::Start(position) = from {
                if position < self.0.position() {
                    self.0
                        .get_mut()
                        .truncate(usize::try_from(position).unwrap());
                }
            }

            Pin::new(&mut self.0).poll_seek(context, from)
        }
    }

    /// Source standing still once before every read and every seek
    struct Hesitant<S> {
        source: S,
        standing_still: bool,
    }

    impl<S> Hesitant<S> {
        /// Creates a source standing still before whatever it is asked for next
        const fn new(source: S) -> Self {
            Self {
                source,
                standing_still: false,
            }
        }

        /// Reports whether it stands still here, and stands ready for what comes next
        fn stands_still(&mut self, context: &Context<'_>) -> bool {
            self.standing_still = !self.standing_still;
            if self.standing_still {
                context.waker().wake_by_ref();
            }

            self.standing_still
        }
    }

    impl<S: AsyncRead + Unpin> AsyncRead for Hesitant<S> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            into: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            if self.stands_still(context) {
                return Poll::Pending;
            }

            Pin::new(&mut self.source).poll_read(context, into)
        }
    }

    impl<S: AsyncSeek + Unpin> AsyncSeek for Hesitant<S> {
        fn poll_seek(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            from: SeekFrom,
        ) -> Poll<io::Result<u64>> {
            if self.stands_still(context) {
                return Poll::Pending;
            }

            Pin::new(&mut self.source).poll_seek(context, from)
        }
    }

    /// Source that moves as soon as it is sought, and reports the seek done only when polled again
    struct Unsettled {
        source: Cursor<Vec<u8>>,
        settling: Option<SeekFrom>,
    }

    impl AsyncRead for Unsettled {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            into: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.source).poll_read(context, into)
        }
    }

    impl AsyncSeek for Unsettled {
        fn poll_seek(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            from: SeekFrom,
        ) -> Poll<io::Result<u64>> {
            if self.settling == Some(from) {
                self.settling = None;

                return Poll::Ready(Ok(self.source.position()));
            }
            if let Poll::Ready(Err(failure)) = Pin::new(&mut self.source).poll_seek(context, from) {
                return Poll::Ready(Err(failure));
            }
            self.settling = Some(from);
            context.waker().wake_by_ref();

            Poll::Pending
        }
    }

    /// Sink recording what was written to it, and whether it was flushed
    #[derive(Default, PartialEq, Debug)]
    struct Recording {
        written: Vec<u8>,
        flushed: bool,
    }

    impl AsyncWrite for Recording {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.written.extend_from_slice(bytes);

            Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            self.flushed = true;

            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// Sink taking one byte at a time, and standing still once it holds `hesitate_at` of them
    #[derive(Default, Debug)]
    struct Trickle {
        recording: Recording,
        hesitate_at: Option<usize>,
    }

    impl AsyncWrite for Trickle {
        fn poll_write(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.hesitate_at == Some(self.recording.written.len()) {
                self.hesitate_at = None;
                context.waker().wake_by_ref();

                return Poll::Pending;
            }
            self.recording.written.extend(bytes.iter().take(1));

            Poll::Ready(Ok(bytes.len().min(1)))
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<io::Result<()>> {
            self.recording.flushed = true;

            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// Polls `future` once and hands over what it gave, dropping it where it stood
    fn poll_once<F: Future>(future: F) -> Option<F::Output> {
        let mut future = pin!(future);

        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        }
    }

    /// What `driver` hands over, kind for kind, until it returns `None`
    fn yielded<D: Demux>(
        driver: &mut DemuxDriver<impl AsyncRead + AsyncSeek + Unpin, D>,
    ) -> Vec<Result<Sample, ErrorKind>> {
        block_on(async {
            let mut yielded = Vec::new();
            while let Some(sample) = driver.next().await {
                yielded.push(sample.map_err(|failure| failure.kind()));
            }

            yielded
        })
    }

    /// What the next call to `driver` hands over, kind for kind
    fn next_yielded<D: Demux>(
        driver: &mut DemuxDriver<impl AsyncRead + AsyncSeek + Unpin, D>,
    ) -> Option<Result<Sample, ErrorKind>> {
        block_on(driver.next()).map(|sample| sample.map_err(|failure| failure.kind()))
    }

    /// What the next call to `driver` hands over, kind for kind, polled until it gives and dropped where it stood each time before
    fn next_carried_on<D: Demux>(
        driver: &mut DemuxDriver<impl AsyncRead + AsyncSeek + Unpin, D>,
    ) -> Option<Result<Sample, ErrorKind>> {
        loop {
            if let Some(given) = poll_once(driver.next()) {
                return given.map(|sample| sample.map_err(|failure| failure.kind()));
            }
        }
    }

    /// What `driver` hands over, kind for kind, until it returns `None`, dropping each future where it stood
    fn yielded_carried_on<D: Demux>(
        driver: &mut DemuxDriver<impl AsyncRead + AsyncSeek + Unpin, D>,
    ) -> Vec<Result<Sample, ErrorKind>> {
        core::iter::from_fn(|| next_carried_on(driver)).collect()
    }

    /// The two cuts of a file a cut long and then some, closing with an `mfra`
    fn file_closing_with_an_mfra() -> [Vec<u8>; 2] {
        [
            vec![0x11; CUT_LENGTH],
            written(&MovieFragmentRandomAccessBox::new(vec![])),
        ]
    }

    /// A driver over the file closing with an `mfra` on a source standing still at every await, completing one sample with the first cut
    fn hesitant_over_a_file_closing_with_an_mfra()
    -> DemuxDriver<Hesitant<Cursor<Vec<u8>>>, Scripted> {
        block_on(DemuxDriver::new(
            Hesitant::new(Cursor::new(file_closing_with_an_mfra().concat())),
            Scripted {
                completed_by_input: vec![sample(b"S1")],
                ..Scripted::default()
            },
        ))
        .unwrap()
    }

    /// A muxer onto a sink taking one byte at a time, which stands still part way through the first chunk
    fn trickling() -> Muxer<Trickle, Queued> {
        Muxer::new(
            Trickle {
                hesitate_at: Some(3),
                ..Trickle::default()
            },
            Queued::default(),
        )
    }

    #[test]
    fn a_want_is_read_where_it_lies_and_the_file_read_on_from_the_input_offset() {
        let mut driver = block_on(DemuxDriver::new(
            Cursor::new(b"FILEBYTES".to_vec()),
            Scripted {
                wanted: Some(2..6),
                input_offset: 4,
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(driver.fsm().data, [(2, b"LEBY".to_vec())]);
        assert_eq!(driver.fsm().inputs, [b"BYTES".to_vec()]);
    }

    #[test]
    fn the_file_begins_where_the_source_stands() {
        let mut source = Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut driver = block_on(DemuxDriver::new(
            source,
            Scripted {
                wanted: Some(1..3),
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec()]);
        assert_eq!(driver.fsm().data, [(1, b"IL".to_vec())]);
    }

    #[test]
    fn a_source_shrunk_below_a_want_is_reported_as_ending() {
        let mut driver = block_on(DemuxDriver::new(
            Shrinking(Cursor::new(b"FILE".to_vec())),
            Scripted::default(),
        ))
        .unwrap();
        block_on(driver.next());
        driver.fsm_mut().wanted = Some(0..4);

        assert_eq!(
            next_yielded(&mut driver),
            Some(Err(ErrorKind::Io(io::ErrorKind::UnexpectedEof)))
        );
    }

    #[test]
    fn the_end_of_the_source_declares_the_file_over_and_the_samples_end_until_a_resume() {
        let mut driver = block_on(DemuxDriver::new(
            Cursor::new(b"FILE".to_vec()),
            Scripted::default(),
        ))
        .unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert!(driver.fsm().finished);
        assert!(block_on(driver.next()).is_none());

        driver.fsm_mut().resume_at(2).unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec(), b"LE".to_vec()]);
    }

    #[test]
    fn the_samples_completed_before_the_fsm_fails_come_first() {
        let failure = isobmff_structure::Error::missing_mandatory_box(BoxType::compact(*b"moov"));
        let mut driver = block_on(DemuxDriver::new(
            Cursor::new(b"FILE".to_vec()),
            Scripted {
                completed_by_input: vec![sample(b"S1"), sample(b"S2")],
                finish: Some(failure),
                ..Scripted::default()
            },
        ))
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
    fn the_samples_a_read_completes_before_the_fsm_fails_on_it_come_before_the_failure() {
        let mut file = non_fragmented_file(&[&[b"SAMP"]], true);
        file.extend_from_slice(b"\0\0\0\x04free");
        let mut driver = block_on(DemuxDriver::new(
            Cursor::new(file),
            NonFragmentedDemuxFsm::new(),
        ))
        .unwrap();

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
    fn an_input_offset_past_what_a_seek_names_is_refused() {
        let mut source = Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut driver = block_on(DemuxDriver::new(source, Scripted::default())).unwrap();
        driver.fsm_mut().resume_at(u64::MAX).unwrap();

        assert_eq!(
            next_yielded(&mut driver),
            Some(Err(ErrorKind::Io(io::ErrorKind::InvalidInput)))
        );
    }

    #[test]
    fn a_movie_lying_after_its_media_data_has_the_bytes_read() {
        let file = non_fragmented_file(&[&[b"SAMP"]], false);
        let mut driver = block_on(DemuxDriver::new(
            Cursor::new(file),
            NonFragmentedDemuxFsm::new(),
        ))
        .unwrap();

        assert_eq!(
            yielded(&mut driver),
            [Ok(Sample::new(
                1,
                0,
                SAMPLE_DURATION,
                0,
                SampleFlags::ZERO,
                1,
                b"SAMP".to_vec()
            ))]
        );
    }

    #[test]
    fn a_driver_dropped_at_every_await_reads_a_want_and_the_file_once_each() {
        let mut source = Hesitant::new(Cursor::new(b"junkFILE".to_vec()));
        source.source.set_position(4);
        let mut driver = block_on(DemuxDriver::new(
            source,
            Scripted {
                wanted: Some(1..3),
                completed_by_input: vec![sample(b"S1"), sample(b"S2")],
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(
            yielded_carried_on(&mut driver),
            [Ok(sample(b"S1")), Ok(sample(b"S2"))]
        );
        assert_eq!(driver.fsm().data, [(1, b"IL".to_vec())]);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec()]);
        assert!(driver.fsm().finished);
    }

    #[test]
    fn a_driver_resumed_and_dropped_at_every_await_reads_the_file_from_the_offset_on() {
        let mut source = Hesitant::new(Cursor::new(b"junkFILE".to_vec()));
        source.source.set_position(4);
        let mut driver = block_on(DemuxDriver::new(source, Scripted::default())).unwrap();
        assert_eq!(yielded_carried_on(&mut driver), []);

        driver.fsm_mut().resume_at(2).unwrap();

        assert_eq!(yielded_carried_on(&mut driver), []);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec(), b"LE".to_vec()]);
    }

    #[test]
    fn a_seek_dropped_after_the_source_moved_has_the_next_call_seek_afresh() {
        let mut driver = block_on(DemuxDriver::new(
            Unsettled {
                source: Cursor::new(b"FILE".to_vec()),
                settling: None,
            },
            Scripted::default(),
        ))
        .unwrap();
        assert_eq!(yielded(&mut driver), []);
        driver.fsm_mut().wanted = Some(1..3);
        assert!(poll_once(driver.next()).is_none());

        driver.fsm_mut().resume_at(4).unwrap();

        assert_eq!(yielded(&mut driver), []);
        assert_eq!(driver.fsm().inputs, [b"FILE".to_vec()]);
    }

    #[test]
    fn a_locate_dropped_part_way_finds_the_mfra_when_made_again() {
        let mut driver = hesitant_over_a_file_closing_with_an_mfra();
        assert!(poll_once(driver.locate_movie_fragment_random_access()).is_none());
        assert!(poll_once(driver.locate_movie_fragment_random_access()).is_none());

        let located = block_on(driver.locate_movie_fragment_random_access());

        assert_eq!(
            located.map_err(|failure| failure.kind()),
            Ok(Some(CUT_LENGTH as u64))
        );
    }

    #[test]
    fn a_locate_dropped_part_way_leaves_the_samples_read_on_from_where_they_stood() {
        let mut driver = hesitant_over_a_file_closing_with_an_mfra();
        assert_eq!(next_carried_on(&mut driver), Some(Ok(sample(b"S1"))));

        let awaits_dropped = 4;
        for _ in 0..awaits_dropped {
            assert!(poll_once(driver.locate_movie_fragment_random_access()).is_none());
        }

        assert_eq!(yielded_carried_on(&mut driver), []);
        assert_eq!(driver.fsm().inputs, file_closing_with_an_mfra());
    }

    #[test]
    fn the_bytes_a_step_made_before_failing_are_written_and_the_steps_failure_reported() {
        let mut muxer = Muxer::new(Recording::default(), Queued::default());

        let driven = block_on(muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Err(isobmff_structure::Error::already_finished())
        }));

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
        let mut muxer = Muxer::new(Cursor::new(&mut [][..]), Queued::default());

        let driven = block_on(muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Ok(())
        }));

        assert_eq!(
            driven.map_err(|failure| failure.kind()),
            Err(ErrorKind::Io(io::ErrorKind::WriteZero))
        );
    }

    #[test]
    fn the_writers_own_failure_is_reported_ahead_of_the_sinks() {
        let mut muxer = Muxer::new(Cursor::new(&mut [][..]), Queued::default());

        let driven = block_on(muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Err(isobmff_structure::Error::already_finished())
        }));

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

        let finished: Result<(), Error> = block_on(muxer.finish(|writer| {
            writer.output.extend(framed(b"LAST"));

            Ok(())
        }));

        assert_eq!(finished.map_err(|failure| failure.kind()), Ok(()));
        assert_eq!(
            muxer.sink,
            Recording {
                written: b"\0\0\0\x0cfreeLAST".to_vec(),
                flushed: true,
            }
        );
    }

    #[test]
    fn a_step_dropped_with_a_chunk_part_written_writes_every_byte_once() {
        let mut muxer = trickling();

        assert!(
            poll_once(muxer.drive(|writer| {
                writer.output.extend(framed(b"MADE"));

                Ok(())
            }))
            .is_none()
        );
        assert_eq!(muxer.sink.recording.written, b"\0\0\0".to_vec());

        let driven = block_on(muxer.drive(|writer| {
            writer.output.extend(framed(b"MORE"));

            Ok(())
        }));

        assert_eq!(driven.map_err(|failure| failure.kind()), Ok(()));
        assert_eq!(
            muxer.sink.recording.written,
            b"\0\0\0\x0cfreeMADE\0\0\0\x0cfreeMORE".to_vec()
        );
    }

    #[test]
    fn a_refused_step_dropped_with_a_chunk_part_written_is_reported_by_the_call_that_follows() {
        let mut muxer = trickling();
        let mut steps = 0;

        assert!(
            poll_once(muxer.drive(|writer| {
                writer.output.extend(framed(b"MADE"));

                Err(isobmff_structure::Error::already_finished())
            }))
            .is_none()
        );
        let driven = block_on(muxer.drive(|_writer| {
            steps += 1;

            Ok(())
        }));

        assert_eq!(steps, 0);
        assert_eq!(
            driven.map_err(|failure| failure.kind()),
            Err(ErrorKind::Structure(
                isobmff_structure::ErrorKind::AlreadyFinished
            ))
        );
        assert_eq!(muxer.sink.recording.written, b"\0\0\0\x0cfreeMADE".to_vec());
    }

    #[test]
    fn a_finish_dropped_with_a_chunk_part_written_makes_its_step_once() {
        let mut muxer = trickling();
        let mut steps = 0;

        assert!(
            poll_once(muxer.finish(|writer| {
                steps += 1;
                writer.output.extend(framed(b"LAST"));

                Ok(())
            }))
            .is_none()
        );
        let finished = block_on(muxer.finish(|writer| {
            steps += 1;
            writer.output.extend(framed(b"LAST"));

            Ok(())
        }));

        assert_eq!(finished.map_err(|failure| failure.kind()), Ok(()));
        assert_eq!(steps, 1);
        assert_eq!(
            muxer.sink.recording,
            Recording {
                written: b"\0\0\0\x0cfreeLAST".to_vec(),
                flushed: true,
            }
        );
    }
}
