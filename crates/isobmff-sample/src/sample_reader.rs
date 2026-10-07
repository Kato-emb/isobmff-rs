//! [`SampleReader`], the samples gathered out of the bytes their extents name

mod index;
mod limits;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::mem;
use core::ops::Range;

use crate::error::Error;
use crate::sample::{Sample, SampleExtent};

use index::Index;
pub use limits::SampleReaderLimits;

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
/// * What the reader holds is bounded by the [`SampleReaderLimits`] it was
///   created with. An extent naming more bytes than one may, or coming once
///   the reader holds as many extents as it may, is refused as it is held;
///   an extent counts until its sample is handed over. Input bringing the
///   first byte of a sample that would take the reader past the bytes it
///   holds is refused as it arrives; a sample counts the bytes its extent
///   names from then until [`poll_sample`](Self::poll_sample) takes it, so
///   input handed over while the samples are not taken runs into that limit.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::Error::AlreadyFinished) aside: every
///   later call that can fail reports that same failure again. The samples
///   made before it are still there to take, and no further one is ever made.
/// * [`finish`](Self::finish) declares the samples over, and fails if an
///   extent held is short of its bytes. Samples are still taken after it, but
///   an extent or input handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::Error::AlreadyFinished).
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
    /// The extents held once a short extent lacks bytes before one held ahead of it, until none is short
    ///
    /// `pending` is empty between calls while this is `Some`.
    // Why not indexing the extents by the bytes they lack all the time: keeping
    // an index costs every extent a few tree operations, which a reader fed in
    // order pays for nothing, where the order the extents are held in is an
    // index already wherever it is the order of their bytes — which extents
    // held together are put in, and extents held one at a time keep only where
    // they come in it.
    index: Option<Index>,
    limits: SampleReaderLimits,
    held_extents: u64,
    held_bytes: u64,
    state: State,
}

impl SampleReader {
    /// Creates a reader holding no sample yet, bounded by the limits [`SampleReaderLimits::new`] states
    #[must_use]
    pub const fn new() -> Self {
        Self::with_limits(SampleReaderLimits::new())
    }

    /// Creates a reader holding no sample yet, bounded by `limits`
    #[must_use]
    pub const fn with_limits(limits: SampleReaderLimits) -> Self {
        Self {
            pending: VecDeque::new(),
            ready: VecDeque::new(),
            whole_held: 0,
            index: None,
            limits,
            held_extents: 0,
            held_bytes: 0,
            state: State::Reading,
        }
    }

    /// Holds `extent`, and reports the sample it names once its bytes have arrived
    ///
    /// # Errors
    ///
    /// * [`SampleSizeLimitExceeded`](crate::Error::SampleSizeLimitExceeded):
    ///   the extent names more bytes than one extent may.
    /// * [`HeldExtentLimitExceeded`](crate::Error::HeldExtentLimitExceeded):
    ///   the reader already holds as many extents as it may.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_sample_extent(&mut self, extent: SampleExtent) -> Result<(), Error> {
        self.reading()?;
        let ready = self.ready.len();
        let first = self.pending.len();
        let held = self.admit(extent);
        self.order_from(first);
        self.report_front();
        self.handed_over_since(ready);

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
    /// * [`SampleSizeLimitExceeded`](crate::Error::SampleSizeLimitExceeded):
    ///   an extent names more bytes than one extent may.
    /// * [`HeldExtentLimitExceeded`](crate::Error::HeldExtentLimitExceeded):
    ///   an extent comes once the reader holds as many as it may.
    /// * The failure among `extents`, where one is.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_sample_extents(
        &mut self,
        extents: impl IntoIterator<Item = Result<SampleExtent, Error>>,
    ) -> Result<(), Error> {
        self.reading()?;
        let ready = self.ready.len();
        let mut extents = extents.into_iter();
        let room = self.limits.held_extents().saturating_sub(self.held_extents);
        self.pending.reserve(
            extents
                .size_hint()
                .0
                .min(usize::try_from(room).unwrap_or(usize::MAX)),
        );
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
        self.order_from(first);
        self.report_front();
        self.handed_over_since(ready);

        held
    }

    /// Fills the samples whose extents reach into `data`, the bytes of the file from `offset` on
    ///
    /// # Errors
    ///
    /// * [`HeldBytesLimitExceeded`](crate::Error::HeldBytesLimitExceeded):
    ///   a sample `data` starts would take the reader past the bytes it
    ///   holds. The samples `data` made whole before it are there to take.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_data(&mut self, offset: u64, data: &[u8]) -> Result<(), Error> {
        self.reading()?;

        // Why not checked_add: the caller read `data` out of a finite resource,
        // so its end cannot run past what 64 bits carry.
        let arriving = offset..offset.saturating_add(data.len() as u64);
        let ready = self.ready.len();
        let limit = self.limits.held_bytes();
        if let Some(index) = &mut self.index {
            let refused = index.fill(
                data,
                &arriving,
                &mut self.ready,
                &mut self.held_bytes,
                limit,
            );
            if !index.holds_short() {
                self.index = None;
            }
            self.handed_over_since(ready);

            return match refused {
                Ok(()) => Ok(()),
                Err(needed) => Err(self.fail(Error::HeldBytesLimitExceeded {
                    held_bytes: needed,
                    limit_bytes: limit,
                })),
            };
        }

        let mut made_whole: usize = 0;
        let mut refused = None;
        for pending in self.pending.iter_mut() {
            if pending.is_whole() {
                continue;
            }
            if pending.lacking().start >= arriving.end {
                break;
            }
            if let Err(needed) = pending.take_from(data, &arriving, &mut self.held_bytes, limit) {
                refused = Some(needed);
                break;
            }
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
        self.handed_over_since(ready);

        match refused {
            None => Ok(()),
            Some(needed) => Err(self.fail(Error::HeldBytesLimitExceeded {
                held_bytes: needed,
                limit_bytes: limit,
            })),
        }
    }

    /// Takes the next sample whose bytes have all arrived
    ///
    /// This never fails: a failed reader hands over the samples it made before
    /// failing, and then reports `None`.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        // Why not `pop_front()?` and `Some(sample)`: unwrapping the sample and
        // wrapping it again slowed a reader handed 64-byte samples by a tenth.
        let sample = self.ready.pop_front();
        if let Some(taken) = &sample {
            self.held_bytes = self.held_bytes.saturating_sub(taken.data().len() as u64);
        }

        sample
    }

    /// Returns the bytes the extent at the front of those held still lacks, if any extent is held
    #[must_use]
    pub fn wanted_extent(&self) -> Option<Range<u64>> {
        self.front().map(PendingSample::lacking)
    }

    /// Declares the samples over, which every extent held must have been met by
    ///
    /// # Errors
    ///
    /// * [`UnfinishedSample`](crate::Error::UnfinishedSample): an
    ///   extent held is short of the bytes it names.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.reading()?;

        match self.front() {
            Some(short) => Err(self.fail(Error::UnfinishedSample {
                track_id: short.extent.track_id(),
                needed_bytes: short.declared_len(),
                available_bytes: short.gathered_len(),
            })),
            None => {
                self.state = State::Finished;

                Ok(())
            }
        }
    }

    /// Drops every extent held and every sample not yet taken, to be handed the samples of another stretch of the file
    ///
    /// The limits stay as the reader was given them, and a reader whose samples
    /// were declared over by [`finish`](Self::finish) takes extents and bytes
    /// again. A reader that failed stays failed.
    pub fn clear(&mut self) {
        let failed = match self.state {
            State::Failed(failure) => Some(failure),
            State::Reading | State::Finished => None,
        };

        *self = Self::with_limits(self.limits);
        if let Some(failure) = failed {
            self.state = State::Failed(failure);
        }
    }

    /// Holds `extent` behind the extents held before it, refusing one past the limits
    fn admit(&mut self, extent: SampleExtent) -> Result<(), Error> {
        let pending = PendingSample {
            extent,
            data: Vec::new(),
        };
        let declared = pending.declared_len();
        if declared > self.limits.sample_size() {
            return Err(self.fail(Error::SampleSizeLimitExceeded {
                track_id: pending.extent.track_id(),
                declared_bytes: declared,
                limit_bytes: self.limits.sample_size(),
            }));
        }
        if self.held_extents >= self.limits.held_extents() {
            return Err(self.fail(Error::HeldExtentLimitExceeded {
                held_extents: self.held_extents.saturating_add(1),
                limit_extents: self.limits.held_extents(),
            }));
        }
        self.held_extents = self.held_extents.saturating_add(1);
        self.pending.push_back(pending);

        Ok(())
    }

    /// Lets go of the extents handed over as samples since `ready` were ready to take
    fn handed_over_since(&mut self, ready: usize) {
        let handed_over = self.ready.len().saturating_sub(ready);
        self.held_extents = self.held_extents.saturating_sub(handed_over as u64);
    }

    /// Moves the extents held into the index once a short one from `first` on lacks bytes before the last short one ahead of it, or the index is already kept
    fn order_from(&mut self, first: usize) {
        if self.index.is_none() {
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
            if before.is_some_and(|before| {
                batch.is_some_and(|batch| before.lacking().start > batch.lacking().start)
            }) {
                self.index = Some(Index::default());
            }
        }
        if let Some(index) = &mut self.index {
            index.hold(self.pending.drain(..));
        }
    }

    /// Returns the extent at the front of those held
    fn front(&self) -> Option<&PendingSample> {
        match &self.index {
            Some(index) => index.front(),
            None => self.pending.front(),
        }
    }

    /// Hands over the whole samples at the front of the queue, in the order they were held
    fn report_front(&mut self) {
        if let Some(index) = &mut self.index {
            index.report_front(&mut self.ready);

            return;
        }

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
            State::Finished => Err(Error::AlreadyFinished),
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
    fn allocated_bytes(&self) -> usize {
        let indexed = self.index.iter().flat_map(Index::held);
        let held = self
            .pending
            .iter()
            .chain(indexed)
            .map(|pending| pending.data.capacity());
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
    ///
    /// The sample adds the bytes its extent names to `held_bytes` as it begins
    /// gathering, and is refused where they would pass `held_bytes_limit`,
    /// with the bytes it would have the reader hold.
    fn take_from(
        &mut self,
        data: &[u8],
        arriving: &Range<u64>,
        held_bytes: &mut u64,
        held_bytes_limit: u64,
    ) -> Result<(), u64> {
        // Why not returning the `Error`: passing it back out of every call on
        // the path a sample is filled through slows a reader handed one sample
        // a call; the caller builds it once, where the fill stops.
        let lacking = self.lacking();
        if !arriving.contains(&lacking.start) {
            return Ok(());
        }

        let taken_from = lacking.start.saturating_sub(arriving.start);
        let taken_to = arriving.end.min(lacking.end).saturating_sub(arriving.start);
        let (Ok(taken_from), Ok(taken_to)) =
            (usize::try_from(taken_from), usize::try_from(taken_to))
        else {
            return Ok(());
        };

        if let Some(taken) = data.get(taken_from..taken_to) {
            // Why not reserving when the extent is held: a reader fed in order
            // fills, reports and drops one sample at a time, and reserving for
            // every extent of a fragment up front hands it cold memory instead.
            if self.data.is_empty() {
                let declared = self.declared_len();
                let held = held_bytes.saturating_add(declared);
                if held > held_bytes_limit {
                    return Err(held);
                }
                *held_bytes = held;
                self.data
                    .reserve_exact(usize::try_from(declared).unwrap_or(0));
            }
            self.data.extend_from_slice(taken);
        }

        Ok(())
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
mod tests;
