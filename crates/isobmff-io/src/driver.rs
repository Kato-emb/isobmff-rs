//! [`Demuxer`] and [`Muxer`], what every demuxer and muxer over `futures::io` does the same way whatever the structure
//!
//! The loop over `std::io` is written apart, in [`blocking`](crate::blocking);
//! both drive their stack through the verbs of `crate::stack`.

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::future::poll_fn;
use core::mem;
use core::ops::Range;
use core::pin::Pin;
use std::io::{self, SeekFrom};

use futures_io::{AsyncRead, AsyncSeek, AsyncWrite};
use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;

use crate::Error;
use crate::movie_fragment_random_access::{PROBE_LEN, Probe, Probed};
use crate::stack::{CUT_LENGTH, PollOutput, ReadSamples, ResumeSamples};

/// Reads off `source` into `into`, and returns how many bytes came
async fn read<S: AsyncRead + Unpin>(source: &mut S, into: &mut [u8]) -> io::Result<usize> {
    poll_fn(|context| Pin::new(&mut *source).poll_read(context, into)).await
}

/// Moves `source` to `position`, and returns where it stands
async fn seek<S: AsyncSeek + Unpin>(source: &mut S, position: SeekFrom) -> io::Result<u64> {
    poll_fn(|context| Pin::new(&mut *source).poll_seek(context, position)).await
}

/// A reading stack driven over an asynchronous source that seeks, a cut at a time
///
/// What every asynchronous demuxer is beneath its own name: the source is
/// read a cut at a time and each cut handed to the reader, the bytes the
/// reader names as lacking are fetched by seeking to them wherever the file
/// passed them by, and the samples come out of [`next`](Self::next). The
/// contract is each demuxer's own, with this much of it held here: where the
/// demuxer stands is a state it keeps, every seek names a position of the
/// file rather than a step from wherever the source happens to be, and what
/// one read gave reaches the reader before the next await — so a
/// [`next`](Self::next) whose future is dropped is carried on by the call
/// that follows, with no byte read twice and none lost.
#[derive(Debug)]
pub(crate) struct Demuxer<S, R> {
    source: S,
    reader: R,
    cut: Vec<u8>,
    origin: u64,
    handed: u64,
    state: State,
    locating: Option<Locating>,
}

/// How far a search for the `mfra` closing the file has come
///
/// The part it reads, whether the source stands at its next byte, and how
/// many of its first bytes the cut holds.
#[derive(Clone, Copy, Debug)]
struct Locating {
    probe: Probe,
    sought: bool,
    probed: usize,
}

/// Where the demuxer stands between samples
#[derive(Debug)]
enum State {
    /// Reading the file off the source where it stands
    Reading,
    /// Moving the source to the bytes the reader lacks
    Fetching(Range<u64>),
    /// Standing at the bytes the reader lacks, to read them
    Fetched(Range<u64>),
    /// Moving the source back to where the file was handed over to
    Restoring,
    /// Over, holding the failure still to report if it ended in one
    Over(Option<Error>),
}

impl<S: AsyncRead + AsyncSeek + Unpin, R: ReadSamples> Demuxer<S, R> {
    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub(crate) async fn new(mut source: S, reader: R) -> Result<Self, Error> {
        let origin = seek(&mut source, SeekFrom::Current(0)).await?;

        Ok(Self {
            source,
            reader,
            cut: vec![0; CUT_LENGTH],
            origin,
            handed: 0,
            state: State::Reading,
            locating: None,
        })
    }

    /// Returns the reader being driven, for what it read into values
    pub(crate) const fn reader(&self) -> &R {
        &self.reader
    }

    /// Takes the next sample the file carries, reading on until one comes
    pub(crate) async fn next(&mut self) -> Option<Result<Sample, Error>> {
        self.locating = None;
        loop {
            if let Some(sample) = self.reader.poll_sample() {
                return Some(Ok(sample));
            }

            let stepped = match &mut self.state {
                State::Over(failure) => return failure.take().map(Err),
                State::Reading => self.read_on().await,
                State::Fetching(wanted) => {
                    let wanted = wanted.clone();

                    self.seek_to(wanted.start, State::Fetched(wanted)).await
                }
                State::Fetched(wanted) => {
                    let start = wanted.start;

                    self.fetch(start).await
                }
                State::Restoring => self.seek_to(self.handed, State::Reading).await,
            };

            if let Err(failure) = stepped {
                self.state = State::Over(Some(failure));
            }
        }
    }

    /// Finds where the `mfra` closing the file begins, from the `mfro` closing it
    ///
    /// The source is left elsewhere, and the demuxer seeks it back to where
    /// the file was handed over to as it reads on.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not seek from its
    ///   end, or does not read.
    pub(crate) async fn locate_movie_fragment_random_access(
        &mut self,
    ) -> Result<Option<u64>, Error> {
        // Why not seeking back once the search is done: a future dropped
        // part way would leave the source moved with nothing to seek it back,
        // where a state seeking on its own carries the reading on whichever
        // call follows.
        self.state = match mem::replace(&mut self.state, State::Restoring) {
            State::Reading => State::Restoring,
            State::Fetched(wanted) => State::Fetching(wanted),
            standing @ (State::Fetching(_) | State::Restoring | State::Over(_)) => standing,
        };

        let located = self.locate().await;
        self.locating = None;

        located
    }

    /// Carries the search for the `mfra` on from where it stands, until it settles
    async fn locate(&mut self) -> Result<Option<u64>, Error> {
        let probe_len = usize::try_from(PROBE_LEN).unwrap_or_default();
        loop {
            let Some(Locating {
                probe,
                sought,
                probed,
            }) = self.locating
            else {
                let end = seek(&mut self.source, SeekFrom::End(0)).await?;
                let Some(probe) = Probe::new(end.saturating_sub(self.origin)) else {
                    return Ok(None);
                };
                self.locating = Some(Locating {
                    probe,
                    sought: false,
                    probed: 0,
                });

                continue;
            };

            if !sought {
                let position = self.origin.saturating_add(probe.start());
                seek(&mut self.source, SeekFrom::Start(position)).await?;
                self.locating = Some(Locating {
                    probe,
                    sought: true,
                    probed,
                });

                continue;
            }

            if probed < probe_len {
                let into = self.cut.get_mut(probed..probe_len).unwrap_or_default();
                let filled = read(&mut self.source, into).await?;
                if filled > 0 {
                    self.locating = Some(Locating {
                        probe,
                        sought,
                        probed: probed.saturating_add(filled),
                    });

                    continue;
                }
            }

            match probe.handle(self.cut.get(..probed).unwrap_or_default()) {
                Probed::Next(next) => {
                    self.locating = Some(Locating {
                        probe: next,
                        sought: false,
                        probed: 0,
                    });
                }
                Probed::Settled(located) => return Ok(located),
            }
        }
    }

    /// Reads on: names the bytes the reader lacks if the file passed them by, else hands over the next cut
    async fn read_on(&mut self) -> Result<(), Error> {
        let passed_by = self
            .reader
            .wanted_extent()
            .filter(|wanted| wanted.start < self.handed);
        if let Some(wanted) = passed_by {
            self.state = State::Fetching(wanted);

            return Ok(());
        }

        let filled = read(&mut self.source, &mut self.cut).await?;
        if filled == 0 {
            self.reader.finish()?;
            self.state = State::Over(None);
        } else {
            self.reader
                .handle_input(self.cut.get(..filled).unwrap_or_default())?;
            self.handed = self.handed.saturating_add(filled as u64);
        }

        Ok(())
    }

    /// Reads the bytes the source stands at as those wanted from `start`, and hands them over
    async fn fetch(&mut self, start: u64) -> Result<(), Error> {
        // Why not reading the want alone: a fragment, or a movie lying after
        // its media data, holds every extent it addresses at once, and a cut
        // read from the first fills the ones behind it too, where fetching
        // them one at a time costs a seek and a sweep of the extents held per
        // sample.
        let filled = read(&mut self.source, &mut self.cut).await?;
        if filled == 0 {
            // Why not carrying on: the want lies before what was handed over
            // in order, so a source holding nothing there has shrunk since,
            // and reading on would ask for the same bytes without end.
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }

        // Why not holding the part of a want the read did not cover: the
        // reader names what it still lacks again, and the next `Reading`
        // fetches it, so a part kept here would be fetched twice.
        let handled = self
            .reader
            .handle_data(start, self.cut.get(..filled).unwrap_or_default());
        self.state = State::Restoring;

        handled.map_err(Error::from)
    }

    /// Moves the source to `offset` of the file, and stands in `next` there
    async fn seek_to(&mut self, offset: u64, next: State) -> Result<(), Error> {
        // Why not checked_add: the offset is one the file was read to, a want
        // lying before it, or a resume point checked as it was taken, so no
        // sum can run past what 64 bits carry.
        let position = SeekFrom::Start(self.origin.saturating_add(offset));
        seek(&mut self.source, position).await?;
        self.state = next;

        Ok(())
    }
}

impl<S: AsyncRead + AsyncSeek + Unpin, R: ResumeSamples> Demuxer<S, R> {
    /// Restarts the reader at `offset` of the file, and moves the source there
    ///
    /// The reader is restarted before the first await, so a future dropped
    /// before the source moved is carried on by the call that follows,
    /// `next` included.
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): `offset` lies past what a seek
    ///   names, which leaves the demuxer as it was, or the source does not
    ///   seek there.
    /// * [`Structure`](crate::ErrorKind::Structure): what the reader's
    ///   `resume_at` makes of the call.
    ///
    /// A failure after the offset is checked ends the samples.
    pub(crate) async fn resume_at(&mut self, offset: u64) -> Result<(), Error> {
        if self.origin.checked_add(offset).is_none() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput).into());
        }

        self.locating = None;
        self.state = State::Over(None);
        self.reader.resume_at(offset)?;
        self.handed = offset;
        self.state = State::Restoring;

        let restored = self.seek_to(self.handed, State::Reading).await;
        if restored.is_err() {
            self.state = State::Over(None);
        }

        restored
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

impl<S: AsyncWrite + Unpin, W: PollOutput> Muxer<S, W> {
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
    use isobmff_boxes::MovieFragmentRandomAccessBox;
    use isobmff_sample::Sample;
    use isobmff_test_support::written;

    use super::{Demuxer, Muxer};

    use crate::stack::CUT_LENGTH;
    use crate::stack::tests::{Queued, Scripted, framed, sample};
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

    /// What `demuxer` hands over, kind for kind, until it is over
    fn yielded(
        demuxer: &mut Demuxer<impl AsyncRead + AsyncSeek + Unpin, Scripted>,
    ) -> Vec<Result<Sample, ErrorKind>> {
        block_on(async {
            let mut yielded = Vec::new();
            while let Some(sample) = demuxer.next().await {
                yielded.push(sample.map_err(|failure| failure.kind()));
            }

            yielded
        })
    }

    /// Polls what `call` makes of `demuxer` until it gives, dropping each future where it stood
    fn carried_on<D, T>(demuxer: &mut D, mut call: impl AsyncFnMut(&mut D) -> T) -> T {
        loop {
            if let Some(given) = poll_once(call(demuxer)) {
                return given;
            }
        }
    }

    /// What `demuxer` hands over, kind for kind, until it is over, dropping each future where it stood
    fn yielded_carried_on(
        demuxer: &mut Demuxer<impl AsyncRead + AsyncSeek + Unpin, Scripted>,
    ) -> Vec<Result<Sample, ErrorKind>> {
        let mut yielded = Vec::new();
        while let Some(sample) = carried_on(demuxer, async |demuxer| demuxer.next().await) {
            yielded.push(sample.map_err(|failure| failure.kind()));
        }

        yielded
    }

    /// The two cuts of a file a cut long and then some, closing with an `mfra`
    fn file_closing_with_an_mfra() -> [Vec<u8>; 2] {
        [
            vec![0x11; CUT_LENGTH],
            written(&MovieFragmentRandomAccessBox::new(vec![])),
        ]
    }

    /// A demuxer over `file` on a source standing still at every await, completing one sample with the first cut
    fn hesitant_over(file: Vec<u8>) -> Demuxer<Hesitant<Cursor<Vec<u8>>>, Scripted> {
        block_on(Demuxer::new(
            Hesitant::new(Cursor::new(file)),
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
    fn a_want_before_what_was_handed_over_is_fetched_by_seeking_and_reading_goes_on_from_where_it_stood()
     {
        let first_cut = vec![0x11; CUT_LENGTH];
        let past_the_cut = b"PASTCUT!".to_vec();
        let mut demuxer = block_on(Demuxer::new(
            Cursor::new([first_cut.clone(), past_the_cut.clone()].concat()),
            Scripted {
                wanted: Some(2..6),
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(
            demuxer.reader().inputs,
            [first_cut.clone(), past_the_cut.clone()]
        );
        let fetched: Vec<u8> = first_cut
            .iter()
            .skip(2)
            .chain(past_the_cut.iter().take(2))
            .copied()
            .collect();
        assert_eq!(demuxer.reader().data, [(2, fetched)]);
    }

    #[test]
    fn the_file_begins_where_the_source_stands() {
        let mut source = Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut demuxer = block_on(Demuxer::new(
            source,
            Scripted {
                wanted: Some(1..3),
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec()]);
        assert_eq!(demuxer.reader().data, [(1, b"ILE".to_vec())]);
    }

    #[test]
    fn a_source_shrunk_below_what_was_read_is_reported_as_ending() {
        let mut demuxer = block_on(Demuxer::new(
            Shrinking(Cursor::new(b"FILE".to_vec())),
            Scripted {
                wanted: Some(0..4),
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [Err(ErrorKind::Io(io::ErrorKind::UnexpectedEof))]
        );
    }

    #[test]
    fn the_end_of_the_source_declares_the_file_over() {
        let mut demuxer = block_on(Demuxer::new(
            Cursor::new(b"FILE".to_vec()),
            Scripted::default(),
        ))
        .unwrap();

        assert_eq!(yielded(&mut demuxer), []);
        assert!(demuxer.reader().finished);
        assert!(block_on(demuxer.next()).is_none());
    }

    #[test]
    fn the_samples_completed_before_a_failure_come_first_and_the_failure_once() {
        let mut demuxer = block_on(Demuxer::new(
            Cursor::new(b"FILE".to_vec()),
            Scripted {
                completed_by_input: vec![sample(b"S1"), sample(b"S2")],
                finish: Some(isobmff_structure::Error::already_finished()),
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(sample(b"S1")),
                Ok(sample(b"S2")),
                Err(ErrorKind::Structure(
                    isobmff_structure::ErrorKind::AlreadyFinished
                )),
            ]
        );
        assert!(block_on(demuxer.next()).is_none());
    }

    #[test]
    fn a_read_dropped_where_the_source_stood_still_loses_no_sample() {
        let mut demuxer = block_on(Demuxer::new(
            Hesitant::new(Cursor::new(b"FILE".to_vec())),
            Scripted {
                completed_by_input: vec![sample(b"S1"), sample(b"S2")],
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert!(poll_once(demuxer.next()).is_none());

        assert_eq!(
            yielded(&mut demuxer),
            [Ok(sample(b"S1")), Ok(sample(b"S2"))]
        );
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec()]);
    }

    #[test]
    fn a_demuxer_dropped_at_every_await_fetches_what_the_file_passed_by_and_reads_it_once() {
        let mut source = Hesitant::new(Cursor::new(b"junkFILE".to_vec()));
        source.source.set_position(4);
        let mut demuxer = block_on(Demuxer::new(
            source,
            Scripted {
                wanted: Some(1..3),
                completed_by_input: vec![sample(b"S1"), sample(b"S2")],
                ..Scripted::default()
            },
        ))
        .unwrap();

        assert_eq!(
            yielded_carried_on(&mut demuxer),
            [Ok(sample(b"S1")), Ok(sample(b"S2"))]
        );
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec()]);
        assert_eq!(demuxer.reader().data, [(1, b"ILE".to_vec())]);
        assert!(demuxer.reader().finished);
    }

    #[test]
    fn a_demuxer_resumed_past_the_end_hands_the_reader_the_file_from_the_offset_on() {
        let mut source = Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut demuxer = block_on(Demuxer::new(source, Scripted::default())).unwrap();
        assert_eq!(yielded(&mut demuxer), []);

        block_on(demuxer.resume_at(2)).unwrap();

        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(demuxer.reader().resumed_at, [2]);
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec(), b"LE".to_vec()]);
    }

    #[test]
    fn an_offset_past_what_a_seek_names_is_refused_and_leaves_the_demuxer_reading() {
        let mut source = Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut demuxer = block_on(Demuxer::new(source, Scripted::default())).unwrap();

        assert_eq!(
            block_on(demuxer.resume_at(u64::MAX)).map_err(|failure| failure.kind()),
            Err(ErrorKind::Io(io::ErrorKind::InvalidInput))
        );
        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(demuxer.reader().resumed_at, []);
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec()]);
    }

    #[test]
    fn a_resume_dropped_before_the_source_moved_is_carried_on_by_the_next_sample() {
        let mut source = Hesitant::new(Cursor::new(b"junkFILE".to_vec()));
        source.source.set_position(4);
        let mut demuxer = block_on(Demuxer::new(source, Scripted::default())).unwrap();
        assert_eq!(yielded_carried_on(&mut demuxer), []);

        assert!(poll_once(demuxer.resume_at(2)).is_none());

        assert_eq!(yielded_carried_on(&mut demuxer), []);
        assert_eq!(demuxer.reader().resumed_at, [2]);
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec(), b"LE".to_vec()]);
    }

    #[test]
    fn a_locate_dropped_at_every_await_finds_the_mfra_and_the_file_reads_on_once() {
        let [first_cut, mfra] = file_closing_with_an_mfra();
        let mut demuxer = hesitant_over([first_cut.clone(), mfra.clone()].concat());
        assert_eq!(
            carried_on(&mut demuxer, async |demuxer| demuxer.next().await)
                .map(|sample| sample.map_err(|failure| failure.kind())),
            Some(Ok(sample(b"S1")))
        );

        let located = carried_on(&mut demuxer, async |demuxer| {
            demuxer.locate_movie_fragment_random_access().await
        });

        assert_eq!(
            located.map_err(|failure| failure.kind()),
            Ok(Some(CUT_LENGTH as u64))
        );
        assert_eq!(yielded_carried_on(&mut demuxer), []);
        assert_eq!(demuxer.reader().inputs, [first_cut, mfra]);
    }

    #[test]
    fn a_locate_given_up_part_way_leaves_the_file_read_on_from_where_it_was_handed_over_to() {
        let [first_cut, mfra] = file_closing_with_an_mfra();
        let mut demuxer = hesitant_over([first_cut.clone(), mfra.clone()].concat());
        assert_eq!(
            carried_on(&mut demuxer, async |demuxer| demuxer.next().await)
                .map(|sample| sample.map_err(|failure| failure.kind())),
            Some(Ok(sample(b"S1")))
        );

        for _ in 0..4 {
            assert!(poll_once(demuxer.locate_movie_fragment_random_access()).is_none());
        }

        assert_eq!(yielded_carried_on(&mut demuxer), []);
        assert_eq!(demuxer.reader().inputs, [first_cut, mfra]);
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
