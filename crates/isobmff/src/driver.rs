//! [`ReadSamples`], [`PollOutput`], [`Demuxing`] and [`drive`], what every demuxer and muxer does the same way whatever the structure

use alloc::vec::Vec;
use core::ops::Range;
use std::io::{self, Read, Seek, SeekFrom, Write};

use isobmff_sample::Sample;
use isobmff_sequence::EventBytes;

use crate::{DriverError, StructureError};

/// Bytes handed over to the reader at a time
const CUT_LENGTH: u64 = 1024 * 1024;

/// Reads samples out of a file handed over as it arrives, as a demuxer drives a reader
///
/// The verbs every structure's reader answers to, each the reader's own of
/// the same name, with its contract.
pub(crate) trait ReadSamples {
    /// Takes the next cut of the file, the continuation of what was handed over before it
    fn handle_input(&mut self, input: &[u8]) -> Result<(), StructureError>;

    /// Takes bytes of the file fetched for what [`wanted_extent`](Self::wanted_extent) named
    fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), StructureError>;

    /// Takes the next sample the file handed over so far completed
    fn poll_sample(&mut self) -> Option<Sample>;

    /// Returns the bytes the earliest sample still held lacks, if any is held
    fn wanted_extent(&self) -> Option<Range<u64>>;

    /// Declares the file over
    fn finish(&mut self) -> Result<(), StructureError>;
}

/// Hands over the bytes a file has been laid down as, as a muxer drives a writer
///
/// The one verb every structure's writer shares: what it takes differs per
/// structure, what it makes of it is taken the same way.
pub(crate) trait PollOutput {
    /// Hands over the bytes the file has been laid down as so far
    fn poll_output(&mut self) -> Option<EventBytes>;
}

/// A reading stack driven over a source that seeks, a cut at a time
///
/// What every demuxer is beneath its own name: the source is read a cut at
/// a time and each cut handed to the reader, the bytes the reader names as
/// lacking are fetched by seeking to them wherever the file passed them by,
/// and the samples come out as `Iterator` items. The file begins where the
/// source stands when the demuxer is created. A failure ends the iteration:
/// the samples the reader had completed before it come first, then the
/// failure once, then `None` for good.
#[derive(Debug)]
pub(crate) struct Demuxing<S, R> {
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

impl<S: Read + Seek, R: ReadSamples> Demuxing<S, R> {
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

impl<S: Read + Seek, R: ReadSamples> Iterator for Demuxing<S, R> {
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

/// Makes `step` of `writer`, and writes what the writer made of it to `sink` whether it failed or not
///
/// What every verb of a muxer is: the bytes made before a refusal reach the
/// sink, the writer's failure is reported ahead of the sink's.
pub(crate) fn drive<W: PollOutput>(
    writer: &mut W,
    sink: &mut impl Write,
    step: impl FnOnce(&mut W) -> Result<(), StructureError>,
) -> Result<(), DriverError> {
    let stepped = step(writer);
    let mut written = Ok(());
    while let Some(bytes) = writer.poll_output() {
        written = written.and_then(|()| sink.write_all(&bytes));
    }
    stepped?;
    written?;

    Ok(())
}
