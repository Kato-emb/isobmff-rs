//! [`DemuxInput`], the input half every demuxer shares — the framing of the file, where the input stands, the samples gathered, and whether the demuxer still reads — with [`InputPosition`], where the input a demuxer takes in order stands, and [`WantedInput`], the one read the demuxer wants next

use core::ops::Range;

use crate::sequence::{BoxEvent, BoxReader};
use crate::{DemuxError, SampleGathering};

/// Takes the input of a file at the offsets it was read at, and keeps whether the demuxer above still reads
///
/// A demuxer hands its input to [`handle_input`](Self::handle_input), reads
/// the boxes the framing made of it through [`poll_event`](Self::poll_event),
/// and hands the result of reading them to [`record`](Self::record), so a
/// failure is kept here, once a call. What is done with each box stays with
/// the demuxer.
///
/// # Contract
///
/// * [`handle_input`](Self::handle_input) routes input as
///   [`InputPosition`] has it: the continuation of the input taken in order
///   goes to the framing, the bytes the samples lack to the samples alone, and
///   input at any other offset is refused.
/// * What the framing makes of the input is held until the events it made
///   before failing are read: [`record`](Self::record) reports a failure of
///   reading them first, and only then the failure of the framing.
/// * [`finish`](Self::finish) checks the samples last, after the framing, the
///   events it made and what the demuxer checked of them.
/// * A failure recorded is reported by every later call that takes input or
///   declares the file over. The samples completed before it are still taken
///   from [`poll_output`](Self::poll_output), and no read is wanted.
/// * Once the file is declared over, input or a second declaration is
///   [`AlreadyFinished`](crate::DemuxError::AlreadyFinished), until
///   [`restart`](Self::restart) takes the reading up again.
#[derive(Debug)]
pub(crate) struct DemuxInput<G> {
    boxes: BoxReader,
    position: InputPosition,
    samples: G,
    framing: Result<(), DemuxError>,
    state: State,
}

/// Where the input stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking the file as it arrives
    Reading,
    /// Told the file is over, and taking no more input
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(DemuxError),
}

impl<G: SampleGathering> DemuxInput<G> {
    /// Creates an input waiting at the start of a file, the bytes its samples lack gathered by `samples`
    #[must_use]
    pub(crate) const fn new(samples: G) -> Self {
        Self {
            boxes: BoxReader::new(),
            position: InputPosition::new(),
            samples,
            framing: Ok(()),
            state: State::Reading,
        }
    }

    /// Returns `Ok` while the demuxer still takes input
    ///
    /// # Errors
    ///
    /// * [`AlreadyFinished`](crate::DemuxError::AlreadyFinished): the file was
    ///   declared over.
    /// * The failure [`record`](Self::record) kept.
    const fn reading(&self) -> Result<(), DemuxError> {
        match self.state {
            State::Reading => Ok(()),
            State::Finished => Err(DemuxError::AlreadyFinished),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Returns `Ok` unless the demuxer has failed, whether or not the file was declared over
    ///
    /// # Errors
    ///
    /// The failure [`record`](Self::record) kept.
    pub(crate) const fn resumable(&self) -> Result<(), DemuxError> {
        match self.state {
            State::Reading | State::Finished => Ok(()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Takes bytes of the file read at `input_offset`, and routes them where they go
    ///
    /// Empty input is taken as nothing. Bytes the samples lack go to them
    /// alone, and a failure they report is recorded. The continuation of the
    /// input taken in order goes to the framing, whose events are then read
    /// through [`poll_event`](Self::poll_event) and whose failure is held for
    /// [`record`](Self::record).
    ///
    /// # Errors
    ///
    /// * [`Sample`](crate::DemuxError::Sample): what the samples make of the
    ///   bytes they lack.
    /// * [`UnwantedInput`](crate::DemuxError::UnwantedInput): `input_offset` is
    ///   neither where the input taken in order stands nor where the bytes the
    ///   samples lack start. Nothing is recorded for it.
    /// * What [`reading`](Self::reading) reports.
    pub(crate) fn handle_input(
        &mut self,
        input_offset: u64,
        input: &[u8],
    ) -> Result<(), DemuxError> {
        self.reading()?;
        if input.is_empty() {
            return Ok(());
        }

        match self
            .position
            .route(input_offset, self.samples.wanted_extent())
        {
            InputRoute::InOrder => {}
            InputRoute::Lacking => {
                let gathered = self.samples.handle_input(input_offset, input);

                return self.record(gathered);
            }
            InputRoute::Unwanted => {
                return Err(DemuxError::UnwantedInput { input_offset });
            }
        }

        self.position.advance(input.len());
        self.framing = self.boxes.handle_input(input).map_err(DemuxError::from);

        Ok(())
    }

    /// Takes the next event the framing made, with the file offset it starts at
    pub(crate) fn poll_event(&mut self) -> Option<(u64, BoxEvent)> {
        let (extent, event) = self.boxes.poll_event()?;

        Some((self.position.file_offset(extent.start), event))
    }

    /// Keeps the failure of reading the events, or else the failure of the framing held, and reports it
    ///
    /// # Errors
    ///
    /// The failure `read` carries, or else the failure of the framing held
    /// since the input or the declaration that made the events.
    pub(crate) fn record(&mut self, read: Result<(), DemuxError>) -> Result<(), DemuxError> {
        let result = read.and(core::mem::replace(&mut self.framing, Ok(())));

        if let Err(failure) = result {
            self.state = State::Failed(failure);
        }

        result
    }

    /// Declares the file over to the framing, whose last events are then read through [`poll_event`](Self::poll_event)
    ///
    /// What the framing makes of the end of the file is held for
    /// [`record`](Self::record).
    ///
    /// # Errors
    ///
    /// What [`reading`](Self::reading) reports.
    pub(crate) fn finish_framing(&mut self) -> Result<(), DemuxError> {
        self.reading()?;
        self.framing = self.boxes.finish().map_err(DemuxError::from);

        Ok(())
    }

    /// Declares the file over once the demuxer `checked` what it read, and checks the samples
    ///
    /// # Errors
    ///
    /// * The failure of the structure `checked` carries, as the [`DemuxError`]
    ///   variant of the same name.
    /// * [`Sample`](crate::DemuxError::Sample): a sample is short of the data
    ///   it claimed.
    /// * What [`reading`](Self::reading) reports.
    pub(crate) fn finish(
        &mut self,
        checked: Result<(), crate::structure::Error>,
    ) -> Result<(), DemuxError> {
        self.reading()?;
        let finished = checked
            .map_err(DemuxError::from)
            .and_then(|()| self.samples.finish());

        self.record(finished)?;
        self.state = State::Finished;

        Ok(())
    }

    /// Declares the file over with nothing more read or checked
    pub(crate) const fn declare_over(&mut self) {
        self.state = State::Finished;
    }

    /// Restarts the reading at `offset`, dropping the framing and the samples held
    pub(crate) fn restart(&mut self, offset: u64) {
        self.boxes = BoxReader::new();
        self.position.resume(offset);
        self.samples.clear();
        self.framing = Ok(());
        self.state = State::Reading;
    }

    /// Returns the one read wanted next, or `None` once the file is declared over or the demuxer has failed
    #[must_use]
    pub(crate) fn wanted_input(&self) -> Option<WantedInput> {
        matches!(self.state, State::Reading)
            .then(|| self.position.wanted_input(self.samples.wanted_extent()))
    }

    /// Returns the gathering, for the demuxer to hand the extents it resolved and the media data it read
    pub(crate) const fn samples_mut(&mut self) -> &mut G {
        &mut self.samples
    }

    /// Takes the next output the input completed
    pub(crate) fn poll_output(&mut self) -> Option<G::Output> {
        self.samples.poll_output()
    }
}

/// The one read a demuxer wants next: where in the file, and how much if it knows
///
/// [`offset`](Self::offset) is a file offset, counted from the first byte of
/// the file as a chunk offset or a base data offset is (ISO/IEC 14496-12
/// §8.7.5, §8.8.7). [`length`](Self::length) is `Some` for bytes the samples
/// lack that the input taken in order has already passed by — the whole of
/// what they lack from there — and `None` for the continuation of that
/// input, which runs on as far as the caller reads.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WantedInput {
    offset: u64,
    length: Option<u64>,
}

impl WantedInput {
    /// Creates the read of `length` bytes from `offset`, or of the continuation of the input from `offset` where `length` is `None`
    #[must_use]
    pub const fn new(offset: u64, length: Option<u64>) -> Self {
        Self { offset, length }
    }

    /// Returns the file offset the read starts at
    #[must_use]
    pub const fn offset(self) -> u64 {
        self.offset
    }

    /// Returns how many bytes the read wants, or `None` where it runs on as far as the caller reads
    #[must_use]
    pub const fn length(self) -> Option<u64> {
        self.length
    }
}

/// Where input handed over at a file offset goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputRoute {
    /// To the framing, as the continuation of the input taken in order
    InOrder,
    /// To the samples, as the bytes they lack that the input taken in order has passed
    Lacking,
    /// Nowhere: the offset is neither
    Unwanted,
}

/// Where the input a demuxer takes in order stands: the offset the reading started at, and what was handed over since
// Why not checked_add anywhere here: the offset the reading resumes at is one
// the caller vouches for, and a change of coordinates carries no failure kind,
// so an offset past any real file saturates.
#[derive(Clone, Copy, Debug)]
struct InputPosition {
    base: u64,
    handed: u64,
}

impl InputPosition {
    /// Creates a position at the first byte of the file
    const fn new() -> Self {
        Self { base: 0, handed: 0 }
    }

    /// Restarts the input at `offset`, nothing handed over from there yet
    const fn resume(&mut self, offset: u64) {
        self.base = offset;
        self.handed = 0;
    }

    /// Counts `length` more bytes handed over in order
    const fn advance(&mut self, length: usize) {
        self.handed = self.handed.saturating_add(length as u64);
    }

    /// Returns the file offset the next byte handed over in order lies at
    const fn offset(self) -> u64 {
        self.base.saturating_add(self.handed)
    }

    /// Returns the file offset of `framed`, an offset the framing counts from where the input started
    const fn file_offset(self, framed: u64) -> u64 {
        framed.saturating_add(self.base)
    }

    /// Returns the read wanted next: `lacking` where the input has passed its start, the continuation of the input otherwise
    fn wanted_input(self, lacking: Option<Range<u64>>) -> WantedInput {
        match lacking {
            Some(lacking) if lacking.start < self.offset() => WantedInput::new(
                lacking.start,
                Some(lacking.end.saturating_sub(lacking.start)),
            ),
            _ => WantedInput::new(self.offset(), None),
        }
    }

    /// Returns where input handed over at `offset` goes, given the bytes `lacking` the samples still lack
    fn route(self, offset: u64, lacking: Option<Range<u64>>) -> InputRoute {
        if offset == self.offset() {
            InputRoute::InOrder
        } else if lacking.is_some_and(|lacking| lacking.start == offset && offset < self.offset()) {
            InputRoute::Lacking
        } else {
            InputRoute::Unwanted
        }
    }
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::SampleFlags;

    use super::{DemuxError, DemuxInput, InputPosition, InputRoute, WantedInput};
    use crate::sample::{Sample, SampleExtent, SampleProperties, SampleReader};
    use crate::sequence::BoxEvent;

    /// A `free` box carrying `SAMP`, twelve bytes long
    const FREE_BOX: &[u8] = b"\0\0\0\x0cfreeSAMP";

    /// A box declaring a total shorter than its own header
    const BROKEN_BOX: &[u8] = b"\0\0\0\x04free";

    /// The one sample whose extent [`extent_at`] names, its bytes arrived
    fn sample() -> Sample {
        Sample::new(
            SampleProperties {
                track_id: 1,
                decode_time: 0,
                sample_duration: 1_024,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::ZERO,
                sample_description_index: 1,
            },
            b"SAMP".to_vec(),
        )
    }

    /// The extent of one sample whose four bytes lie at `start`
    fn extent_at(start: u64) -> SampleExtent {
        SampleExtent::new(
            SampleProperties {
                track_id: 1,
                decode_time: 0,
                sample_duration: 1_024,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::ZERO,
                sample_description_index: 1,
            },
            1,
            start..start.checked_add(4).unwrap(),
        )
    }

    /// Reads every event the framing made as a demuxer would, offering each payload to the samples
    fn read_framed(input: &mut DemuxInput<SampleReader>) -> Result<(), DemuxError> {
        while let Some((start, event)) = input.poll_event() {
            if let BoxEvent::Payload(payload) = event {
                input.samples_mut().handle_input(start, &payload)?;
            }
        }

        Ok(())
    }

    /// An input handed `file` whole, its events read, and one sample declared at `start`
    fn read_with_a_sample_at(file: &[u8], start: u64) -> DemuxInput<SampleReader> {
        let mut input = DemuxInput::new(SampleReader::new());

        input.handle_input(0, file).unwrap();
        let read = read_framed(&mut input);
        input.record(read).unwrap();
        input
            .samples_mut()
            .handle_sample_extent(extent_at(start))
            .unwrap();

        input
    }

    #[test]
    fn the_samples_completed_before_a_framing_failure_are_still_taken() {
        let mut input = DemuxInput::new(SampleReader::new());
        input
            .samples_mut()
            .handle_sample_extent(extent_at(8))
            .unwrap();

        input
            .handle_input(0, &[FREE_BOX, BROKEN_BOX].concat())
            .unwrap();
        let read = read_framed(&mut input);

        assert_eq!(
            input.record(read),
            Err(DemuxError::from(crate::sequence::Error::from(
                isobmff_core::Error::size_below_header(8, 4)
            )))
        );
        assert_eq!(input.poll_output(), Some(sample()));
        assert_eq!(input.wanted_input(), None);
    }

    #[test]
    fn a_failure_of_reading_the_events_is_reported_before_the_failure_of_the_framing() {
        let mut input = DemuxInput::new(SampleReader::new());
        let failure = DemuxError::UnwantedInput { input_offset: 0 };

        input.handle_input(0, BROKEN_BOX).unwrap();

        assert_eq!(input.record(Err(failure)), Err(failure));
    }

    #[test]
    fn a_failed_input_reports_the_same_failure_for_every_call_after_it() {
        let mut input = DemuxInput::new(SampleReader::new());
        input.handle_input(0, BROKEN_BOX).unwrap();
        let failure = input.record(Ok(())).unwrap_err();

        assert_eq!(input.handle_input(0, FREE_BOX), Err(failure));
        assert_eq!(input.finish_framing(), Err(failure));
        assert_eq!(input.resumable(), Err(failure));
    }

    #[test]
    fn bytes_the_samples_lack_are_wanted_with_their_length_once_the_input_has_passed_them() {
        let input = read_with_a_sample_at(FREE_BOX, 8);

        assert_eq!(input.wanted_input(), Some(WantedInput::new(8, Some(4))));
    }

    #[test]
    fn input_at_the_offset_wanted_completes_the_sample_and_the_continuation_is_wanted_after_it() {
        let mut input = read_with_a_sample_at(FREE_BOX, 8);

        input.handle_input(8, b"SAMP").unwrap();

        assert_eq!(input.poll_output(), Some(sample()));
        assert_eq!(input.wanted_input(), Some(WantedInput::new(12, None)));
    }

    #[test]
    fn input_in_order_is_taken_while_bytes_are_wanted() {
        let mut input = read_with_a_sample_at(FREE_BOX, 8);

        input.handle_input(12, FREE_BOX).unwrap();

        assert_eq!(input.wanted_input(), Some(WantedInput::new(8, Some(4))));
    }

    #[test]
    fn input_at_an_offset_neither_in_order_nor_wanted_is_refused_and_the_file_reads_on() {
        let mut input = read_with_a_sample_at(FREE_BOX, 8);

        assert_eq!(
            input.handle_input(9, b"AMP"),
            Err(DemuxError::UnwantedInput { input_offset: 9 })
        );
        assert_eq!(input.handle_input(8, b"SAMP"), Ok(()));
        assert_eq!(input.finish_framing(), Ok(()));
        assert_eq!(input.finish(Ok(())), Ok(()));
    }

    #[test]
    fn empty_input_is_taken_as_nothing_wherever_it_is_handed_over() {
        let mut input = DemuxInput::new(SampleReader::new());

        assert_eq!(input.handle_input(9, &[]), Ok(()));
        assert_eq!(input.wanted_input(), Some(WantedInput::new(0, None)));
    }

    #[test]
    fn a_file_declared_over_wants_nothing_and_takes_nothing_more() {
        let mut input = DemuxInput::new(SampleReader::new());

        input.finish_framing().unwrap();
        input.finish(Ok(())).unwrap();

        assert_eq!(input.wanted_input(), None);
        assert_eq!(
            input.handle_input(0, FREE_BOX),
            Err(DemuxError::AlreadyFinished)
        );
        assert_eq!(input.finish_framing(), Err(DemuxError::AlreadyFinished));
    }

    #[test]
    fn a_file_declared_over_with_a_sample_short_of_its_bytes_is_rejected() {
        let mut input = read_with_a_sample_at(FREE_BOX, 8);

        input.finish_framing().unwrap();

        assert!(matches!(
            input.finish(Ok(())),
            Err(DemuxError::Sample {
                error: crate::sample::Error::UnfinishedSample { .. }
            })
        ));
    }

    #[test]
    fn bytes_lacking_are_wanted_only_where_they_start_before_the_in_order_offset() {
        let mut position = InputPosition::new();
        position.advance(10);

        assert_eq!(
            [
                position.wanted_input(Some(9..20)),
                position.wanted_input(Some(10..20)),
                position.wanted_input(None)
            ],
            [
                WantedInput::new(9, Some(11)),
                WantedInput::new(10, None),
                WantedInput::new(10, None)
            ]
        );
    }

    #[test]
    fn input_goes_to_the_framing_in_order_to_the_samples_at_bytes_passed_and_nowhere_else() {
        let mut position = InputPosition::new();
        position.advance(10);

        assert_eq!(
            [
                position.route(10, Some(9..20)),
                position.route(9, Some(9..20)),
                position.route(12, Some(12..20)),
                position.route(8, Some(9..20)),
                position.route(9, None)
            ],
            [
                InputRoute::InOrder,
                InputRoute::Lacking,
                InputRoute::Unwanted,
                InputRoute::Unwanted,
                InputRoute::Unwanted
            ]
        );
    }
}
