//! [`SampleReader`], the samples gathered out of the bytes their extents name

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::mem;
use core::ops::Range;

use crate::error::Error;
use crate::sample::{Sample, SampleExtent};

/// Reads samples out of the bytes of a file, given where each of them lies
///
/// The reader is handed [`SampleExtent`]s, each naming the bytes of a sample,
/// and the input as it arrives, each piece with the offset it starts at, in
/// any order and cut anywhere. It reports a [`Sample`] once every byte of its
/// extent has arrived, and names the extent it still lacks so a caller that
/// can seek fetches it. It reaches for no source of its own: when to read and
/// from where stay with the caller.
///
/// Where a sample's bytes lie is what the reader is told, and every extent is
/// taken to lie in the one resource the bytes come from; which resource that
/// is was settled where the extents were resolved.
///
/// # Contract
///
/// * [`handle_sample_extent`](Self::handle_sample_extent) holds the extent
///   from then on, behind those held before it;
///   [`handle_sample_extents`](Self::handle_sample_extents) holds several
///   behind those held before them, in the order of their bytes, those
///   starting at the same byte in the order they came; and
///   [`handle_data`](Self::handle_data) fills every held extent the input
///   reaches. Bytes no held extent names are dropped, so input arriving
///   before the extent that names it is not kept for it, and the extent is
///   reported as lacking those bytes once it arrives.
/// * The samples are taken one at a time from
///   [`poll_sample`](Self::poll_sample), in the order their bytes arrived
///   whole: input that makes a sample whole hands over every whole sample
///   held that carries bytes, in the order they were held, and input that
///   makes none whole hands over nothing. An extent naming no bytes is whole
///   as soon as it is held, and is handed over once nothing short is held
///   before it — at once, or with whatever input clears the way. Bytes
///   arriving in the order the extents are held thus yield the samples in
///   that order, and bytes arriving in the order of the file, wherever cut,
///   yield the samples of extents held together in that order.
/// * A sample fills from its start: input reaching it takes from the byte it
///   lacks next up to the end of the input or of the sample, and input
///   starting past that byte, or ending before it, leaves the sample as it
///   stands.
/// * Two extents naming the same bytes are held and filled apart, each
///   yielding its own sample.
/// * [`wanted_extent`](Self::wanted_extent) names the bytes the extent at
///   the front of those held still lacks, and `None` once no extent is held.
///   A caller reading the file in order never needs it; one that can seek
///   fetches what it names, and the next want appears once that one is met.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside: every
///   later call that can fail reports that same failure again. The samples
///   made before it are still there to take, and no further one is ever made.
/// * [`finish`](Self::finish) declares the samples over, and fails if an
///   extent held is short of its bytes. Samples are still taken after it, but
///   an extent or input handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::SampleFlags;
/// use isobmff_sample::{Sample, SampleExtent, SampleReader};
///
/// let mut reader = SampleReader::new();
///
/// // Bytes arriving before the extent that names them are dropped
/// reader.handle_data(100, b"ABCD")?;
/// reader.handle_sample_extent(SampleExtent::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, 1, 100..104))?;
/// assert_eq!(reader.poll_sample(), None);
///
/// // The reader names what it lacks, and the sample is whole once handed it
/// assert_eq!(reader.wanted_extent(), Some(100..104));
/// reader.handle_data(100, b"ABCD")?;
/// assert_eq!(
///     reader.poll_sample(),
///     Some(Sample::new(1, 0, 1_024, 0, SampleFlags::ZERO, 1, b"ABCD".to_vec()))
/// );
/// assert_eq!(reader.wanted_extent(), None);
/// reader.finish()?;
/// # Ok::<(), isobmff_sample::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct SampleReader {
    pending: VecDeque<PendingSample>,
    ready: VecDeque<Sample>,
    // Why not counting the whole extents held when they are asked for: that
    // is a walk over every held extent on every arrival, which a reader fed
    // in order pays for nothing.
    whole_held: usize,
    // Why not an index of the extents by the bytes they lack: keeping one costs
    // every extent a few tree operations, which a reader fed in order pays for
    // nothing, where the order the extents are held in is an index already
    // wherever it is the order of their bytes — which extents held together
    // are put in, and extents held one at a time keep only where they come
    // in it. The long way is left for what is held behind a short extent
    // lying past it, one at a time or together.
    held_in_order: bool,
    sample_size_limit: u64,
    state: State,
}

impl SampleReader {
    /// Bytes one sample may declare, where the caller names no limit
    ///
    /// Sixteen mebibytes. A caller reading a presentation whose samples reach
    /// past that — a mezzanine format holds whole uncompressed frames — names a
    /// limit of its own with
    /// [`with_sample_size_limit`](Self::with_sample_size_limit).
    pub const DEFAULT_SAMPLE_SIZE_LIMIT: u64 = 16 * 1024 * 1024;

    /// Creates a reader holding no sample yet
    ///
    /// What one sample may declare is bounded by
    /// [`DEFAULT_SAMPLE_SIZE_LIMIT`](Self::DEFAULT_SAMPLE_SIZE_LIMIT).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_sample_size_limit(Self::DEFAULT_SAMPLE_SIZE_LIMIT)
    }

    /// Creates a reader gathering no more than `sample_size_limit` bytes for one sample
    ///
    /// A sample is gathered whole before it is reported, so the length its
    /// extent names is memory the reader is about to take. An extent naming
    /// more than `sample_size_limit` bytes is
    /// [`SampleSizeLimitExceeded`](crate::ErrorKind::SampleSizeLimitExceeded)
    /// instead, refused before a byte of it is gathered.
    ///
    /// The limit bounds one sample rather than the presentation: it is checked
    /// against the length an extent names, not against what the extents held
    /// before it name between them.
    #[must_use]
    pub const fn with_sample_size_limit(sample_size_limit: u64) -> Self {
        Self {
            pending: VecDeque::new(),
            ready: VecDeque::new(),
            whole_held: 0,
            held_in_order: true,
            sample_size_limit,
            state: State::Reading,
        }
    }

    /// Holds `extent`, and reports the sample it names once its bytes have arrived
    ///
    /// # Errors
    ///
    /// * [`SampleSizeLimitExceeded`](crate::ErrorKind::SampleSizeLimitExceeded):
    ///   the extent names more bytes than the limit the reader was given.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_sample_extent(&mut self, extent: SampleExtent) -> Result<(), Error> {
        self.reading()?;
        let first = self.pending.len();
        let held = self.admit(extent);
        self.held_in_order_from(first);
        self.report_front();

        held
    }

    /// Holds the extents of `extents` together, in the order of their bytes
    ///
    /// The extents are held behind those held before them, ordered by the
    /// byte each starts at — those starting at the same byte in the order
    /// they came — up to the first the reader refuses or the first that is a
    /// failure, which fails the reader as one of its own would, the extents
    /// before it held. The iterators
    /// [`sample_table::sample_extents`](crate::sample_table::sample_extents)
    /// and [`movie_fragment::sample_extents`](crate::movie_fragment::sample_extents)
    /// return are handed over as they are.
    ///
    /// # Errors
    ///
    /// * [`SampleSizeLimitExceeded`](crate::ErrorKind::SampleSizeLimitExceeded):
    ///   an extent names more bytes than the limit the reader was given.
    /// * The failure among `extents`, where one is.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_sample_extents(
        &mut self,
        extents: impl IntoIterator<Item = Result<SampleExtent, Error>>,
    ) -> Result<(), Error> {
        self.reading()?;
        let mut extents = extents.into_iter();
        self.pending.reserve(extents.size_hint().0);
        let first = self.pending.len();
        let mut in_order = true;
        let mut last_start = None;
        let held = extents.try_for_each(|extent| {
            let extent = extent.map_err(|failure| self.fail(failure))?;
            let start = extent.extent().start;
            in_order &= last_start.is_none_or(|last| last <= start);
            last_start = Some(start);

            self.admit(extent)
        });
        // Why not sorting outright: making the queue contiguous first moves
        // every extent held wherever the ring has wrapped, which a batch
        // already in order — as a fragment of one track laid down run after
        // run declares — never needs.
        if !in_order {
            if let Some(batch) = self.pending.make_contiguous().get_mut(first..) {
                batch.sort_by_key(|held| held.extent.extent().start);
            }
        }
        self.held_in_order_from(first);
        self.report_front();

        held
    }

    /// Fills the samples whose extents reach into `data`, the bytes of the file from `offset` on
    ///
    /// # Errors
    ///
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), Error> {
        self.reading()?;

        // Why not checked_add: the caller read `data` out of a finite resource,
        // so its end cannot run past what 64 bits carry.
        let arriving = offset..offset.saturating_add(data.len() as u64);
        let mut made_whole: usize = 0;
        let held_in_order = self.held_in_order;
        for pending in self.pending.iter_mut() {
            if pending.is_whole() {
                continue;
            }
            if held_in_order && pending.lacking().start >= arriving.end {
                break;
            }
            pending.take_from(data, &arriving);
            made_whole = made_whole.saturating_add(usize::from(pending.is_whole()));
        }

        if made_whole > 0 {
            self.whole_held = self.whole_held.saturating_add(made_whole);
            // Why not sweeping every held extent each time: bytes read in order
            // make whole the extents at the front and no other, and popping
            // those costs nothing, where moving the rest up a slot costs the
            // whole queue.
            self.report_front();
            if self.whole_held > 0 {
                for pending in mem::take(&mut self.pending) {
                    if pending.is_whole() && pending.declared_len() > 0 {
                        self.ready.push_back(pending.into_sample());
                    } else {
                        self.pending.push_back(pending);
                    }
                }
                self.whole_held = 0;
            }
        }

        Ok(())
    }

    /// Takes the next sample whose bytes have all arrived
    ///
    /// This never fails: a failed reader hands over the samples it made before
    /// failing, and then reports `None`.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.ready.pop_front()
    }

    /// Returns the bytes the extent at the front of those held still lacks, if any extent is held
    #[must_use]
    pub fn wanted_extent(&self) -> Option<Range<u64>> {
        self.pending.front().map(PendingSample::lacking)
    }

    /// Declares the samples over, which every extent held must have been met by
    ///
    /// # Errors
    ///
    /// * [`UnfinishedSample`](crate::ErrorKind::UnfinishedSample): an
    ///   extent held is short of the bytes it names.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.reading()?;

        match self.pending.front() {
            Some(short) => Err(self.fail(Error::unfinished_sample(
                short.extent.track_id(),
                short.declared_len(),
                short.gathered_len(),
            ))),
            None => {
                self.state = State::Finished;

                Ok(())
            }
        }
    }

    /// Drops every extent held and every sample not yet taken, to be handed the samples of another stretch of the file
    ///
    /// The limit stays as the reader was given it, and a reader whose samples
    /// were declared over by [`finish`](Self::finish) takes extents and bytes
    /// again. A reader that failed stays failed.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.ready.clear();
        self.whole_held = 0;
        self.held_in_order = true;
        if matches!(self.state, State::Finished) {
            self.state = State::Reading;
        }
    }

    /// Holds `extent` behind the extents held before it, refusing one past the limit
    fn admit(&mut self, extent: SampleExtent) -> Result<(), Error> {
        let pending = PendingSample {
            extent,
            data: Vec::new(),
        };
        let declared = pending.declared_len();
        if declared > self.sample_size_limit {
            return Err(self.fail(Error::sample_size_limit_exceeded(
                pending.extent.track_id(),
                declared,
                self.sample_size_limit,
            )));
        }
        self.pending.push_back(pending);

        Ok(())
    }

    /// Keeps `held_in_order` where the first short extent held from `first` on lies at or past the last short one before it, the batch itself in order
    fn held_in_order_from(&mut self, first: usize) {
        // Why the start of what is lacked and not the start of the extent, and
        // why the extents already whole are passed over: input fills every
        // short extent it reaches up to its own end, so short extents held in
        // the order of their bytes stay in the order of the bytes they lack,
        // and the fill stops at the first short extent lacking bytes past the
        // input. An extent naming no bytes is whole where it is held and
        // waits behind the short ones, keeping a start the fills leave behind.
        let before = self
            .pending
            .range(..first)
            .rev()
            .find(|held| !held.is_whole());
        let batch = self.pending.range(first..).find(|held| !held.is_whole());
        self.held_in_order = before.is_none_or(|before| {
            self.held_in_order
                && batch.is_none_or(|batch| before.lacking().start <= batch.lacking().start)
        });
    }

    /// Hands over the whole samples at the front of the queue, in the order they were held
    fn report_front(&mut self) {
        while self.pending.front().is_some_and(PendingSample::is_whole) {
            let Some(front) = self.pending.pop_front() else {
                break;
            };
            // Why not counting an extent naming no bytes among the whole held:
            // it is whole where it is held and leaves only from the front, so
            // it never calls for a sweep of the queue, and counted behind a
            // short front it would set one off that hands over nothing.
            self.whole_held = self
                .whole_held
                .saturating_sub(usize::from(front.declared_len() > 0));
            self.ready.push_back(front.into_sample());
        }
    }

    /// Returns `Ok` while the reader still takes extents and bytes
    const fn reading(&self) -> Result<(), Error> {
        match self.state {
            State::Reading => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the reader for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }

    /// Returns the bytes the reader has taken memory for, held or ready to take
    #[cfg(test)]
    fn held_bytes(&self) -> usize {
        let held = self.pending.iter().map(|pending| pending.data.capacity());
        let ready = self.ready.iter().map(|sample| sample.data().len());

        held.chain(ready).sum()
    }
}

impl Default for SampleReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the reader stands between calls
#[derive(Clone, Debug)]
enum State {
    /// Taking extents and bytes
    Reading,
    /// Told the samples are over, and taking no more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

/// Sample an extent names, gathered as far as its bytes have arrived
#[derive(Clone, Debug)]
struct PendingSample {
    extent: SampleExtent,
    data: Vec<u8>,
}

impl PendingSample {
    /// Returns the bytes the extent names
    fn declared_len(&self) -> u64 {
        let named = self.extent.extent();

        named.end.saturating_sub(named.start)
    }

    /// Returns the bytes of the sample that have arrived
    fn gathered_len(&self) -> u64 {
        self.data.len() as u64
    }

    /// Returns whether every byte the extent names has arrived
    fn is_whole(&self) -> bool {
        self.gathered_len() >= self.declared_len()
    }

    /// Returns the bytes the extent names that have yet to arrive
    fn lacking(&self) -> Range<u64> {
        let named = self.extent.extent();

        named.start.saturating_add(self.gathered_len())..named.end
    }

    /// Takes off `data`, the bytes of the file `arriving` covers, the bytes of this sample that come next
    fn take_from(&mut self, data: &[u8], arriving: &Range<u64>) {
        let lacking = self.lacking();
        if !arriving.contains(&lacking.start) {
            return;
        }

        let taken_from = lacking.start.saturating_sub(arriving.start);
        let taken_to = arriving.end.min(lacking.end).saturating_sub(arriving.start);
        let (Ok(taken_from), Ok(taken_to)) =
            (usize::try_from(taken_from), usize::try_from(taken_to))
        else {
            return;
        };

        if let Some(taken) = data.get(taken_from..taken_to) {
            // Why not reserving when the extent is held: a reader fed in order
            // fills, reports and drops one sample at a time, and reserving for
            // every extent of a fragment up front hands it cold memory instead.
            if self.data.is_empty() {
                self.data
                    .reserve_exact(usize::try_from(self.declared_len()).unwrap_or(0));
            }
            self.data.extend_from_slice(taken);
        }
    }

    /// Returns the sample, now that every byte of it has arrived
    fn into_sample(self) -> Sample {
        Sample::new(
            self.extent.track_id(),
            self.extent.decode_time(),
            self.extent.sample_duration(),
            self.extent.sample_composition_time_offset(),
            self.extent.sample_flags(),
            self.extent.sample_description_index(),
            self.data,
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::SampleFlags;

    use super::SampleReader;
    use crate::error::Error;
    use crate::sample::{Sample, SampleExtent};

    /// Extent of a sample of track 1 as the tests here declare it
    fn extent(decode_time: u64, data: Range<u64>) -> SampleExtent {
        SampleExtent::new(1, decode_time, 1_024, 0, SampleFlags::ZERO, 1, 1, data)
    }

    /// Sample of track 1 as [`extent`] declares it
    fn sample(decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(
            1,
            decode_time,
            1_024,
            0,
            SampleFlags::ZERO,
            1,
            data.to_vec(),
        )
    }

    /// Reader holding the extents given, none of them met
    fn holding(extents: impl IntoIterator<Item = SampleExtent>) -> SampleReader {
        let mut reader = SampleReader::new();
        for extent in extents {
            reader.handle_sample_extent(extent).unwrap();
        }

        reader
    }

    /// Takes every sample the reader has made whole
    fn drained(reader: &mut SampleReader) -> Vec<Sample> {
        let mut samples = Vec::new();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }

        samples
    }

    #[test]
    fn a_cleared_reader_holds_nothing_and_reads_the_next_stretch_under_the_same_limit() {
        let mut reader = SampleReader::with_sample_size_limit(8);
        reader
            .handle_sample_extents([Ok(extent(0, 100..104)), Ok(extent(1_024, 104..108))])
            .unwrap();
        reader.handle_data(100, b"ABCD").unwrap();

        reader.clear();

        assert_eq!(reader.wanted_extent(), None);
        assert_eq!(reader.poll_sample(), None);

        reader
            .handle_sample_extents([Ok(extent(8_192, 500..504))])
            .unwrap();
        reader.handle_data(500, b"WXYZ").unwrap();

        assert_eq!(drained(&mut reader), [sample(8_192, b"WXYZ")]);
        assert_eq!(
            reader.handle_sample_extent(extent(9_216, 504..513)),
            Err(Error::sample_size_limit_exceeded(1, 9, 8))
        );
    }

    #[test]
    fn a_reader_declared_over_takes_extents_again_once_cleared_and_a_failed_one_stays_failed() {
        let mut finished = SampleReader::new();
        finished.finish().unwrap();

        finished.clear();

        assert_eq!(finished.handle_sample_extent(extent(0, 100..104)), Ok(()));

        let mut failed = SampleReader::new();
        failed.handle_sample_extent(extent(0, 100..104)).unwrap();
        let unfinished = failed.finish().unwrap_err();

        failed.clear();

        assert_eq!(failed.handle_data(100, b"ABCD"), Err(unfinished));
    }

    #[test]
    fn a_sample_is_reported_once_the_bytes_its_extent_names_have_arrived() {
        let mut reader = holding([extent(0, 100..104)]);

        reader.handle_data(100, b"AB").unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader.handle_data(102, b"CD").unwrap();
        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn input_read_in_order_meets_every_extent_without_a_want_being_asked() {
        let mut reader = holding([extent(0, 100..104), extent(1_024, 104..108)]);

        reader.handle_data(100, b"ABCDEF").unwrap();
        reader.handle_data(106, b"GH").unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
        assert_eq!(reader.wanted_extent(), None);
    }

    #[test]
    fn extents_handed_over_together_are_held_in_the_order_of_their_bytes() {
        let mut reader = SampleReader::new();

        reader
            .handle_sample_extents([Ok(extent(1_024, 104..108)), Ok(extent(0, 100..104))])
            .unwrap();
        assert_eq!(reader.wanted_extent(), Some(100..104));

        reader.handle_data(100, b"ABCDEFGH").unwrap();
        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
    }

    #[test]
    fn extents_handed_over_together_starting_at_the_same_byte_keep_the_order_they_came() {
        let mut reader = SampleReader::new();

        reader
            .handle_sample_extents([Ok(extent(1_024, 100..104)), Ok(extent(0, 100..104))])
            .unwrap();
        reader.handle_data(100, b"ABCD").unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(1_024, b"ABCD"), sample(0, b"ABCD")]
        );
    }

    #[test]
    fn extents_handed_over_together_behind_one_lying_past_them_are_filled_all_the_same() {
        let mut reader = holding([extent(2_048, 200..204)]);

        reader
            .handle_sample_extents([Ok(extent(1_024, 104..108)), Ok(extent(0, 100..104))])
            .unwrap();
        reader.handle_data(100, b"ABCDEFGH").unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
        assert_eq!(reader.wanted_extent(), Some(200..204));
    }

    #[test]
    fn the_failure_a_resolver_stopped_at_fails_the_reader_after_the_extents_before_it_held_in_order()
     {
        let mut reader = SampleReader::new();
        let stopped_at = Error::data_offset_overflow(1);

        assert_eq!(
            reader.handle_sample_extents([
                Ok(extent(1_024, 104..108)),
                Ok(extent(0, 100..100)),
                Err(stopped_at)
            ]),
            Err(stopped_at)
        );
        assert_eq!(drained(&mut reader), [sample(0, b"")]);
        assert_eq!(reader.wanted_extent(), Some(104..108));
        assert_eq!(reader.handle_data(104, b"ABCD"), Err(stopped_at));
    }

    #[test]
    fn an_extent_held_behind_one_lying_past_it_is_filled_all_the_same() {
        let mut reader = holding([extent(0, 200..204), extent(1_024, 100..104)]);

        reader.handle_data(100, b"ABCD").unwrap();

        assert_eq!(drained(&mut reader), [sample(1_024, b"ABCD")]);
        assert_eq!(reader.wanted_extent(), Some(200..204));
    }

    #[test]
    fn an_extent_naming_no_bytes_held_behind_a_short_one_does_not_hide_the_extents_held_after_it() {
        let mut reader = holding([extent(0, 0..100), extent(1_024, 10..10)]);
        reader.handle_data(0, &[0xab; 30]).unwrap();

        reader.handle_sample_extent(extent(2_048, 20..25)).unwrap();
        reader.handle_data(20, b"ABCDE").unwrap();

        assert_eq!(drained(&mut reader), [sample(2_048, b"ABCDE")]);
        assert_eq!(reader.wanted_extent(), Some(30..100));
    }

    #[test]
    fn extents_overlapping_each_take_what_they_lack_from_the_same_input() {
        let mut reader = holding([extent(0, 100..110), extent(1_024, 104..108)]);

        reader.handle_data(100, b"ABCDEF").unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader.handle_data(106, b"GHIJ").unwrap();
        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCDEFGHIJ"), sample(1_024, b"EFGH")]
        );
    }

    #[test]
    fn bytes_arriving_before_the_extent_that_names_them_are_dropped_and_wanted_again() {
        let mut reader = SampleReader::new();
        reader.handle_data(100, b"ABCD").unwrap();

        reader.handle_sample_extent(extent(0, 100..104)).unwrap();
        assert_eq!(reader.poll_sample(), None);
        assert_eq!(reader.wanted_extent(), Some(100..104));

        reader.handle_data(100, b"ABCD").unwrap();
        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn bytes_no_extent_names_are_passed_over() {
        let mut reader = holding([extent(0, 100..104)]);

        reader.handle_data(200, b"XXXX").unwrap();

        assert_eq!(reader.poll_sample(), None);
        assert_eq!(reader.wanted_extent(), Some(100..104));
    }

    #[test]
    fn the_reader_holds_no_more_than_the_bytes_its_extents_name() {
        let mut reader = holding([
            extent(0, 100..104),
            extent(1_024, 104..108),
            extent(2_048, 1_000..1_004),
        ]);
        assert_eq!(reader.held_bytes(), 0);

        reader.handle_data(2_000, &[0xab; 4_096]).unwrap();
        assert_eq!(reader.held_bytes(), 0);

        reader.handle_data(0, &[0xab; 106]).unwrap();
        assert_eq!(reader.held_bytes(), 4 + 4);

        reader.handle_data(100, &[0xab; 2_048]).unwrap();
        assert_eq!(reader.held_bytes(), 4 + 4 + 4);

        assert_eq!(drained(&mut reader).len(), 3);
        assert_eq!(reader.held_bytes(), 0);
    }

    #[test]
    fn the_want_is_what_the_earliest_extent_held_still_lacks() {
        let mut reader = holding([extent(0, 100..104), extent(1_024, 200..204)]);
        assert_eq!(reader.wanted_extent(), Some(100..104));

        reader.handle_data(100, b"AB").unwrap();
        assert_eq!(reader.wanted_extent(), Some(102..104));

        reader.handle_data(102, b"CD").unwrap();
        assert_eq!(reader.wanted_extent(), Some(200..204));
    }

    #[test]
    fn bytes_that_arrived_already_leave_the_sample_as_it_stands() {
        let mut reader = holding([extent(0, 100..104)]);

        reader.handle_data(100, b"AB").unwrap();
        reader.handle_data(100, b"XXCD").unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn a_sample_fills_from_its_start() {
        let mut reader = holding([extent(0, 100..104)]);

        reader.handle_data(102, b"CD").unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader.handle_data(100, b"ABCD").unwrap();
        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn samples_come_out_in_the_order_their_bytes_arrived_whole() {
        let mut reader = holding([extent(0, 100..104), extent(1_024, 104..108)]);

        reader.handle_data(104, b"EFGH").unwrap();
        reader.handle_data(100, b"ABCD").unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(1_024, b"EFGH"), sample(0, b"ABCD")]
        );
    }

    #[test]
    fn two_extents_naming_the_same_bytes_are_each_met_by_them() {
        let mut reader = holding([extent(0, 100..104), extent(1_024, 100..104)]);

        reader.handle_data(100, b"ABCD").unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"ABCD")]
        );
    }

    #[test]
    fn an_extent_naming_no_bytes_is_whole_as_soon_as_it_is_held() {
        let mut reader = holding([extent(0, 100..100)]);

        assert_eq!(drained(&mut reader), [sample(0, b"")]);
        assert_eq!(reader.wanted_extent(), None);
        reader.finish().unwrap();
    }

    #[test]
    fn an_extent_naming_no_bytes_waits_on_the_short_ones_held_before_it() {
        let mut reader = holding([extent(0, 100..104), extent(1_024, 104..104)]);
        assert_eq!(reader.poll_sample(), None);

        reader.handle_data(100, b"ABCD").unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"")]
        );
    }

    #[test]
    fn input_making_a_sample_whole_hands_over_every_whole_sample_held_that_carries_bytes() {
        let mut reader = holding([
            extent(0, 100..104),
            extent(1_024, 104..104),
            extent(2_048, 104..108),
        ]);

        reader.handle_data(104, b"EFGH").unwrap();

        assert_eq!(drained(&mut reader), [sample(2_048, b"EFGH")]);
        assert_eq!(reader.wanted_extent(), Some(100..104));
    }

    #[test]
    fn an_extent_naming_no_bytes_is_not_handed_over_ahead_of_a_short_one_held_before_it() {
        let mut reader = holding([
            extent(0, 100..104),
            extent(1_024, 104..108),
            extent(2_048, 108..108),
        ]);

        reader.handle_data(100, b"ABCD").unwrap();
        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);

        reader.handle_data(104, b"EFGH").unwrap();
        assert_eq!(
            drained(&mut reader),
            [sample(1_024, b"EFGH"), sample(2_048, b"")]
        );
    }

    #[test]
    fn an_extent_naming_more_bytes_than_the_limit_is_refused() {
        let mut reader = SampleReader::with_sample_size_limit(3);

        assert_eq!(
            reader.handle_sample_extent(extent(0, 100..104)),
            Err(Error::sample_size_limit_exceeded(1, 4, 3))
        );
    }

    #[test]
    fn a_sample_short_of_its_bytes_is_refused_when_the_samples_are_declared_over() {
        let mut reader = holding([extent(0, 100..104)]);
        reader.handle_data(100, b"AB").unwrap();

        assert_eq!(reader.finish(), Err(Error::unfinished_sample(1, 4, 2)));
    }

    #[test]
    fn a_failure_is_reported_again_for_every_call_after_it() {
        let mut reader = SampleReader::with_sample_size_limit(3);
        let refused = reader.handle_sample_extent(extent(0, 100..104));

        assert_eq!(reader.handle_data(100, b"ABCD"), refused);
        assert_eq!(reader.finish(), refused);
    }

    #[test]
    fn samples_made_whole_before_a_failure_are_still_taken() {
        let mut reader = holding([extent(0, 100..104), extent(1_024, 104..108)]);
        reader.handle_data(100, b"ABCD").unwrap();

        assert_eq!(reader.finish(), Err(Error::unfinished_sample(1, 4, 0)));
        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn anything_handed_over_after_the_samples_were_declared_over_is_refused() {
        let mut reader = SampleReader::new();
        reader.finish().unwrap();

        assert_eq!(
            reader.handle_sample_extent(extent(0, 100..104)),
            Err(Error::already_finished())
        );
        assert_eq!(
            reader.handle_data(100, b"ABCD"),
            Err(Error::already_finished())
        );
        assert_eq!(reader.finish(), Err(Error::already_finished()));
    }
}
