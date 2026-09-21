//! [`ReadSamples`], [`PollOutput`], [`Demuxer`] and [`Muxer`], what every demuxer and muxer does the same way whatever the structure

use alloc::vec::Vec;
use core::ops::Range;
use std::io::{self, Read, Seek, SeekFrom, Write};

use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;

use crate::{DriverError, StructureError};

/// Bytes handed over to the reader at a time
const CUT_LENGTH: u64 = 1024 * 1024;

/// The five verbs of a reader a demuxer drives
///
/// Each is the reader's own of the same name, with its contract.
pub(crate) trait ReadSamples {
    /// Takes the next cut of the file and reads the samples it completes
    fn handle_input(&mut self, input: &[u8]) -> Result<(), StructureError>;

    /// Takes bytes of the file fetched for what [`wanted_extent`](Self::wanted_extent) named, and reads the samples they complete
    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), StructureError>;

    /// Takes the next sample the file handed over so far completed
    fn poll_sample(&mut self) -> Option<Sample>;

    /// Returns the bytes the earliest sample still held lacks, if any is held
    fn wanted_extent(&self) -> Option<Range<u64>>;

    /// Declares the file over
    fn finish(&mut self) -> Result<(), StructureError>;
}

/// The one verb of a writer a muxer takes its bytes by
///
/// It is the writer's own of the same name, with its contract.
pub(crate) trait PollOutput {
    /// Hands over the bytes the file has been laid down as so far
    fn poll_output(&mut self) -> Option<EventBytes>;
}

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
    Over(Option<DriverError>),
}

impl<S: Read + Seek, R: ReadSamples> Demuxer<S, R> {
    /// Creates a demuxer over `source` driving `reader`, the file beginning where the source stands
    ///
    /// # Errors
    ///
    /// * [`Io`](crate::DriverErrorKind::Io): the source does not report
    ///   where it stands.
    pub(crate) fn new(mut source: S, reader: R) -> Result<Self, DriverError> {
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
    fn read_on(&mut self) -> Result<(), DriverError> {
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
            let length = wanted.end.saturating_sub(wanted.start).max(CUT_LENGTH);
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

        let read = self.read_cut(CUT_LENGTH)?;
        if read == 0 {
            self.reader.finish()?;
            self.state = State::Over(None);
        } else {
            self.reader.handle_input(&self.cut)?;
            self.handed = self.handed.saturating_add(read);
        }

        Ok(())
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

impl<S: Read + Seek, R: ReadSamples> Iterator for Demuxer<S, R> {
    type Item = Result<Sample, DriverError>;

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
    /// * [`Structure`](crate::DriverErrorKind::Structure): what the writer
    ///   makes of the step, reported ahead of the sink's failure; the bytes
    ///   made before the refusal reach the sink all the same.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes.
    pub(crate) fn drive(
        &mut self,
        step: impl FnOnce(&mut W) -> Result<(), StructureError>,
    ) -> Result<(), DriverError> {
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
    /// * [`Structure`](crate::DriverErrorKind::Structure): what the writer
    ///   makes of the step.
    /// * [`Io`](crate::DriverErrorKind::Io): the sink refuses the bytes, or
    ///   does not flush.
    pub(crate) fn finish(
        &mut self,
        step: impl FnOnce(&mut W) -> Result<(), StructureError>,
    ) -> Result<(), DriverError> {
        self.drive(step)?;
        self.sink.flush()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::ops::Range;
    use std::io::{self, Read, Seek, SeekFrom, Write};

    use isobmff_core::{BoxHeader, BoxType};
    use isobmff_sample::Sample;
    use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};

    use super::{CUT_LENGTH, Demuxer, Muxer, PollOutput, ReadSamples};
    use crate::{DriverError, DriverErrorKind, StructureError, StructureErrorKind};

    /// Reader answering as scripted, and recording what it was handed
    #[derive(Default)]
    struct Scripted {
        wanted: Option<Range<u64>>,
        completed_by_input: Vec<Sample>,
        finish: Option<StructureError>,
        inputs: Vec<Vec<u8>>,
        data: Vec<(u64, Vec<u8>)>,
        samples: VecDeque<Sample>,
        finished: bool,
    }

    impl ReadSamples for Scripted {
        fn handle_input(&mut self, input: &[u8]) -> Result<(), StructureError> {
            self.inputs.push(input.to_vec());
            self.samples.extend(self.completed_by_input.drain(..));

            Ok(())
        }

        fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), StructureError> {
            self.data.push((offset, data.to_vec()));
            self.wanted = None;

            Ok(())
        }

        fn poll_sample(&mut self) -> Option<Sample> {
            self.samples.pop_front()
        }

        fn wanted_extent(&self) -> Option<Range<u64>> {
            self.wanted.clone()
        }

        fn finish(&mut self) -> Result<(), StructureError> {
            self.finished = true;

            self.finish.take().map_or(Ok(()), Err)
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

    /// Writer handing over what it was scripted to, step by step
    #[derive(Default)]
    struct Queued {
        output: VecDeque<EventBytes>,
    }

    impl PollOutput for Queued {
        fn poll_output(&mut self) -> Option<EventBytes> {
            self.output.pop_front()
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

    /// A sample of track 1 carrying `data`
    fn sample(data: &[u8]) -> Sample {
        Sample::new(1, 0, 1, 0, 0, 1, data.to_vec())
    }

    /// The bytes a `free` box of `payload` is framed as, one `EventBytes` a step
    fn framed(payload: &[u8]) -> Vec<EventBytes> {
        let mut boxes = BoxWriter::new();
        let header =
            BoxHeader::with_payload_len(BoxType::compact(*b"free"), payload.len() as u64).unwrap();

        boxes.handle_event(BoxEvent::Header(header)).unwrap();
        boxes
            .handle_event(BoxEvent::Payload(payload.to_vec()))
            .unwrap();
        boxes.handle_event(BoxEvent::End).unwrap();

        core::iter::from_fn(|| boxes.poll_output()).collect()
    }

    /// What `demuxer` yields, kind for kind, until it is over
    fn yielded(
        demuxer: &mut Demuxer<impl Read + Seek, Scripted>,
    ) -> Vec<Result<Sample, DriverErrorKind>> {
        demuxer
            .map(|sample| sample.map_err(|failure| failure.kind()))
            .collect()
    }

    #[test]
    fn a_want_before_what_was_handed_over_is_fetched_by_seeking_and_reading_goes_on_from_where_it_stood()
     {
        let first_cut = vec![0x11; usize::try_from(CUT_LENGTH).unwrap()];
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
            [Err(DriverErrorKind::Io(io::ErrorKind::UnexpectedEof))]
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
                finish: Some(StructureError::already_finished()),
                ..Scripted::default()
            },
        )
        .unwrap();

        assert_eq!(
            yielded(&mut demuxer),
            [
                Ok(sample(b"S1")),
                Ok(sample(b"S2")),
                Err(DriverErrorKind::Structure(
                    StructureErrorKind::AlreadyFinished
                )),
            ]
        );
        assert!(demuxer.next().is_none());
    }

    #[test]
    fn the_bytes_a_step_made_before_failing_are_written_and_the_steps_failure_reported() {
        let mut muxer = Muxer::new(Recording::default(), Queued::default());

        let driven = muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Err(StructureError::already_finished())
        });

        assert_eq!(
            driven.map_err(|failure| failure.structure_error()),
            Err(Some(StructureError::already_finished()))
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
            Err(DriverErrorKind::Io(io::ErrorKind::WriteZero))
        );
    }

    #[test]
    fn the_writers_own_failure_is_reported_ahead_of_the_sinks() {
        let mut muxer = Muxer::new(&mut [][..], Queued::default());

        let driven = muxer.drive(|writer| {
            writer.output.extend(framed(b"MADE"));

            Err(StructureError::already_finished())
        });

        assert_eq!(
            driven.map_err(|failure| failure.kind()),
            Err(DriverErrorKind::Structure(
                StructureErrorKind::AlreadyFinished
            ))
        );
    }

    #[test]
    fn finishing_writes_the_last_step_and_flushes_the_sink() {
        let mut muxer = Muxer::new(Recording::default(), Queued::default());

        let finished: Result<(), DriverError> = muxer.finish(|writer| {
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
