//! [`Demuxer`] and [`Muxer`], what every demuxer and muxer over `std::io` does the same way whatever the structure

use alloc::vec::Vec;
use std::io::{self, Read, Seek, SeekFrom, Write};

use isobmff_sample::Sample;

use crate::Error;
use crate::movie_fragment_random_access::{PROBE_LEN, Probe, Probed};
use crate::stack::{CUT_LENGTH, PollOutput, ReadSamples, ResumeSamples};

/// A reading stack driven over a source that seeks, a cut at a time
///
/// What every demuxer is beneath its own name: the source is read a cut at
/// a time and each cut handed to the reader, the bytes the reader names as
/// lacking are fetched by seeking to them wherever the file passed them by,
/// and the samples come out as `Iterator` items. The contract is each
/// demuxer's own.
#[derive(Debug)]
pub(crate) struct Demuxer<S, R> {
    source: S,
    reader: R,
    cut: Vec<u8>,
    origin: u64,
    handed: u64,
    state: State,
}

/// Where the demuxer stands between samples
#[derive(Debug)]
enum State {
    /// Reading the file off the source
    Reading,
    /// Over, holding the failure still to report if it ended in one
    Over(Option<Error>),
}

impl<S: Read + Seek, R: ReadSamples> Demuxer<S, R> {
    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not report
    ///   where it stands.
    pub(crate) fn new(mut source: S, reader: R) -> Result<Self, Error> {
        let origin = source.stream_position()?;

        Ok(Self {
            source,
            reader,
            cut: Vec::new(),
            origin,
            handed: 0,
            state: State::Reading,
        })
    }

    /// Returns the reader being driven, for what it read into values
    pub(crate) const fn reader(&self) -> &R {
        &self.reader
    }

    /// Reads on: fetches what the reader lacks if the file passed it by, else hands over the next cut
    fn read_on(&mut self) -> Result<(), Error> {
        let passed_by = self
            .reader
            .wanted_extent()
            .filter(|wanted| wanted.start < self.handed);
        if let Some(wanted) = passed_by {
            // Why not checked_add: the want lies before `handed`, a position
            // the source already stood at, so neither sum can run past what
            // 64 bits carry.
            self.source
                .seek(SeekFrom::Start(self.origin.saturating_add(wanted.start)))?;
            // Why not the want alone: a fragment, or a movie lying after its
            // media data, holds every extent it addresses at once, and a cut
            // read from the first fills the ones behind it too, where fetching
            // them one at a time costs a seek and a sweep of the extents held
            // per sample.
            let length = wanted
                .end
                .saturating_sub(wanted.start)
                .max(CUT_LENGTH as u64);
            if self.read_cut(length)? == 0 {
                // Why not carrying on: the want lies before what was handed
                // over in order, so a source holding nothing there has shrunk
                // since, and reading on would ask for the same bytes without end.
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            self.reader.handle_data(wanted.start, &self.cut)?;
            self.source
                .seek(SeekFrom::Start(self.origin.saturating_add(self.handed)))?;

            return Ok(());
        }

        let read = self.read_cut(CUT_LENGTH as u64)?;
        if read == 0 {
            self.reader.finish()?;
            self.state = State::Over(None);
        } else {
            self.reader.handle_input(&self.cut)?;
            self.handed = self.handed.saturating_add(read);
        }

        Ok(())
    }

    /// Finds where the `mfra` closing the file begins, from the `mfro` closing it, and leaves the source where it stood
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::ErrorKind::Io): the source does not seek from its
    ///   end, or does not read; a source not sought back to where it stood
    ///   ends the samples.
    pub(crate) fn locate_movie_fragment_random_access(&mut self) -> Result<Option<u64>, Error> {
        let located = self.probe_end();
        // Why not leaving the source where the probe failed: reading on
        // takes the bytes from where the source stands, so a source left
        // elsewhere would hand the reader bytes out of place.
        if let Err(failure) = self
            .source
            .seek(SeekFrom::Start(self.origin.saturating_add(self.handed)))
        {
            if let State::Reading = self.state {
                self.state = State::Over(None);
            }

            return Err(failure.into());
        }

        Ok(located?)
    }

    /// Reads the `mfro` off the end of the file, and the header of the box it names
    fn probe_end(&mut self) -> io::Result<Option<u64>> {
        let file_len = self
            .source
            .seek(SeekFrom::End(0))?
            .saturating_sub(self.origin);
        let Some(mut probe) = Probe::new(file_len) else {
            return Ok(None);
        };

        loop {
            self.source
                .seek(SeekFrom::Start(self.origin.saturating_add(probe.start())))?;
            self.read_cut(PROBE_LEN)?;
            match probe.handle(&self.cut) {
                Probed::Next(next) => probe = next,
                Probed::Settled(located) => return Ok(located),
            }
        }
    }

    /// Reads up to `length` bytes off the source into the cut, and returns how many came
    fn read_cut(&mut self, length: u64) -> io::Result<u64> {
        self.cut.clear();
        let read = self
            .source
            .by_ref()
            .take(length)
            .read_to_end(&mut self.cut)?;

        Ok(read as u64)
    }
}

impl<S: Read + Seek, R: ResumeSamples> Demuxer<S, R> {
    /// Restarts the reader at `offset` of the file, and moves the source there
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
    pub(crate) fn resume_at(&mut self, offset: u64) -> Result<(), Error> {
        let position = self
            .origin
            .checked_add(offset)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;

        self.state = State::Over(None);
        self.reader.resume_at(offset)?;
        self.handed = offset;
        self.source.seek(SeekFrom::Start(position))?;
        self.state = State::Reading;

        Ok(())
    }
}

impl<S: Read + Seek, R: ReadSamples> Iterator for Demuxer<S, R> {
    type Item = Result<Sample, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.reader.poll_sample() {
                return Some(Ok(sample));
            }
            if let State::Over(failure) = &mut self.state {
                return failure.take().map(Err);
            }
            if let Err(failure) = self.read_on() {
                self.state = State::Over(Some(failure));
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

    use isobmff_sample::Sample;

    use super::{CUT_LENGTH, Demuxer, Muxer};

    use crate::stack::tests::{Queued, Scripted, framed, sample};
    use crate::{Error, ErrorKind};

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

    /// What `demuxer` yields, kind for kind, until it is over
    fn yielded(
        demuxer: &mut Demuxer<impl Read + Seek, Scripted>,
    ) -> Vec<Result<Sample, ErrorKind>> {
        demuxer
            .map(|sample| sample.map_err(|failure| failure.kind()))
            .collect()
    }

    #[test]
    fn a_want_before_what_was_handed_over_is_fetched_by_seeking_and_reading_goes_on_from_where_it_stood()
     {
        let first_cut = vec![0x11; CUT_LENGTH];
        let past_the_cut = b"PASTCUT!".to_vec();
        let mut demuxer = Demuxer::new(
            io::Cursor::new([first_cut.clone(), past_the_cut.clone()].concat()),
            Scripted {
                wanted: Some(2..6),
                ..Scripted::default()
            },
        )
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
        let mut source = io::Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut demuxer = Demuxer::new(
            source,
            Scripted {
                wanted: Some(1..3),
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec()]);
        assert_eq!(demuxer.reader().data, [(1, b"ILE".to_vec())]);
    }

    #[test]
    fn a_source_shrunk_below_what_was_read_is_reported_as_ending() {
        let mut demuxer = Demuxer::new(
            Shrinking(io::Cursor::new(b"FILE".to_vec())),
            Scripted {
                wanted: Some(0..4),
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [Err(ErrorKind::Io(io::ErrorKind::UnexpectedEof))]
        );
    }

    #[test]
    fn the_end_of_the_source_declares_the_file_over() {
        let mut demuxer =
            Demuxer::new(io::Cursor::new(b"FILE".to_vec()), Scripted::default()).unwrap();

        assert_eq!(yielded(&mut demuxer), []);
        assert!(demuxer.reader().finished);
        assert!(demuxer.next().is_none());
    }

    #[test]
    fn the_samples_completed_before_a_failure_come_first_and_the_failure_once() {
        let mut demuxer = Demuxer::new(
            io::Cursor::new(b"FILE".to_vec()),
            Scripted {
                completed_by_input: vec![sample(b"S1"), sample(b"S2")],
                finish: Some(isobmff_structure::Error::already_finished()),
                ..Scripted::default()
            },
        )
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
        assert!(demuxer.next().is_none());
    }

    #[test]
    fn a_demuxer_resumed_past_the_end_hands_the_reader_the_file_from_the_offset_on() {
        let mut source = io::Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut demuxer = Demuxer::new(source, Scripted::default()).unwrap();
        assert_eq!(yielded(&mut demuxer), []);

        demuxer.resume_at(2).unwrap();

        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(demuxer.reader().resumed_at, [2]);
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec(), b"LE".to_vec()]);
    }

    #[test]
    fn a_locate_leaves_the_file_read_on_from_where_it_was_handed_over_to() {
        let first_cut = vec![0x11; CUT_LENGTH];
        let past_the_cut = b"PASTCUT!".to_vec();
        let mut demuxer = Demuxer::new(
            io::Cursor::new([first_cut.clone(), past_the_cut.clone()].concat()),
            Scripted {
                completed_by_input: vec![sample(b"S1")],
                ..Scripted::default()
            },
        )
        .unwrap();
        assert_eq!(
            demuxer.next().map(|sample| sample.unwrap()),
            Some(sample(b"S1"))
        );

        assert_eq!(
            demuxer
                .locate_movie_fragment_random_access()
                .map_err(|failure| failure.kind()),
            Ok(None)
        );

        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(demuxer.reader().inputs, [first_cut, past_the_cut]);
    }

    #[test]
    fn an_offset_past_what_a_seek_names_is_refused_and_leaves_the_demuxer_reading() {
        let mut source = io::Cursor::new(b"junkFILE".to_vec());
        source.set_position(4);
        let mut demuxer = Demuxer::new(source, Scripted::default()).unwrap();

        assert_eq!(
            demuxer
                .resume_at(u64::MAX)
                .map_err(|failure| failure.kind()),
            Err(ErrorKind::Io(io::ErrorKind::InvalidInput))
        );
        assert_eq!(yielded(&mut demuxer), []);
        assert_eq!(demuxer.reader().resumed_at, []);
        assert_eq!(demuxer.reader().inputs, [b"FILE".to_vec()]);
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
