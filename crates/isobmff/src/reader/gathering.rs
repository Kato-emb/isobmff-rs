//! [`SampleGathering`], the samples claimed by the fragments read so far

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::ops::Range;

use crate::error::SampleError;
use crate::reader::fragment::SettledSample;
use crate::sample::Sample;

/// Sample a fragment claimed the data of, gathered as far as it has arrived
///
/// The length gathered is how far into its extent the sample is whole, so a
/// claim is met when that length reaches the one it declared.
#[derive(Clone, Debug)]
struct PendingSample {
    settled: SettledSample,
    data: Vec<u8>,
}

impl PendingSample {
    /// Returns the bytes of the sample that have arrived
    fn gathered_len(&self) -> u64 {
        self.data.len() as u64
    }

    /// Returns whether every byte the sample was declared to occupy has arrived
    fn is_whole(&self) -> bool {
        self.gathered_len() >= self.settled.declared_len()
    }

    /// Takes off `arriving` the bytes of this sample that come next
    ///
    /// The sample fills from its start, so what is taken is the run beginning
    /// where the bytes gathered so far end and reaching no further than the end
    /// of the sample. A sample already whole, media data ending before what it
    /// holds already, and media data starting past the point it fills from each
    /// leave it as it stands.
    fn take_from(&mut self, arriving: &[u8], extent: &Range<u64>) {
        let filled_end = self
            .settled
            .extent
            .start
            .saturating_add(self.gathered_len());
        if self.is_whole() || !extent.contains(&filled_end) {
            return;
        }

        let taken_from = filled_end.saturating_sub(extent.start);
        let taken_to = extent
            .end
            .min(self.settled.extent.end)
            .saturating_sub(extent.start);
        let (Ok(taken_from), Ok(taken_to)) =
            (usize::try_from(taken_from), usize::try_from(taken_to))
        else {
            return;
        };

        if let Some(taken) = arriving.get(taken_from..taken_to) {
            self.data.extend_from_slice(taken);
        }
    }

    /// Returns the sample, now that every byte of it has arrived
    fn into_sample(mut self) -> Sample {
        // Why not leave the buffer as it stands: it grew as the data arrived, so
        // it holds up to twice the sample where a caller cut the media data
        // small, and the caller it is handed to has no way to take that back.
        self.data.shrink_to_fit();

        Sample::new(
            self.settled.track_id,
            self.settled.decode_time,
            self.settled.sample_duration,
            self.settled.sample_composition_time_offset,
            self.settled.sample_flags,
            self.settled.sample_description_index,
            self.data,
        )
    }
}

/// Claims of the fragments read so far, gathered from the media data that arrives
///
/// A sample is held from the moment its fragment claimed it until every byte it
/// claimed has arrived, and is then reported in the order the fragments
/// declared it. A claim reaching behind what has been read is refused rather
/// than held.
#[derive(Clone, Debug)]
pub(super) struct SampleGathering {
    pending: VecDeque<PendingSample>,
    ready: VecDeque<Sample>,
    read_so_far: u64,
    sample_size_limit: u64,
}

impl SampleGathering {
    /// Creates a gathering holding no more than `sample_size_limit` bytes for one sample
    pub(super) const fn new(sample_size_limit: u64) -> Self {
        Self {
            pending: VecDeque::new(),
            ready: VecDeque::new(),
            read_so_far: 0,
            sample_size_limit,
        }
    }

    /// Holds the claims a fragment settled to, none of which may lie behind `read_to`
    ///
    /// `read_to` is where the fragment that declared them ends, and so how far
    /// the presentation has been read by the time they are claimed.
    ///
    /// # Errors
    ///
    /// * [`SampleSizeLimitExceeded`](crate::SampleErrorKind::SampleSizeLimitExceeded):
    ///   a sample declares more bytes than the limit the gathering was given.
    /// * [`BackwardDataOffset`](crate::SampleErrorKind::BackwardDataOffset): a
    ///   sample was resolved to data lying behind what has been read.
    pub(super) fn claim(
        &mut self,
        settled: Vec<SettledSample>,
        read_to: u64,
    ) -> Result<(), SampleError> {
        self.read_so_far = self.read_so_far.max(read_to);

        for sample in settled {
            let declared = sample.declared_len();
            if declared > self.sample_size_limit {
                return Err(SampleError::sample_size_limit_exceeded(
                    sample.track_id,
                    declared,
                    self.sample_size_limit,
                ));
            }
            if sample.extent.start < self.read_so_far {
                return Err(SampleError::backward_data_offset(
                    sample.extent.start,
                    self.read_so_far,
                ));
            }

            self.pending.push_back(PendingSample {
                settled: sample,
                data: Vec::new(),
            });
        }

        // Why not reporting only where media data arrives: a fragment whose
        // samples all declare no bytes is met by no media data at all.
        self.report_whole();

        Ok(())
    }

    /// Fills the samples claiming the bytes `data` holds, which `extent` covers
    ///
    /// # Errors
    ///
    /// * [`ExtentLengthMismatch`](crate::SampleErrorKind::ExtentLengthMismatch):
    ///   `extent` covers a different number of bytes than `data` holds.
    pub(super) fn fill(&mut self, data: &[u8], extent: &Range<u64>) -> Result<(), SampleError> {
        let covered = extent.end.saturating_sub(extent.start);
        let offered = data.len() as u64;
        if covered != offered {
            return Err(SampleError::extent_length_mismatch(covered, offered));
        }

        self.read_so_far = self.read_so_far.max(extent.end);
        for pending in &mut self.pending {
            pending.take_from(data, extent);
        }
        self.report_whole();

        Ok(())
    }

    /// Takes the next sample whose data has all arrived
    pub(super) fn poll(&mut self) -> Option<Sample> {
        self.ready.pop_front()
    }

    /// Returns `Ok` where no claim is left short of the data it named
    ///
    /// # Errors
    ///
    /// * [`UnfinishedSample`](crate::SampleErrorKind::UnfinishedSample): a
    ///   sample a fragment declared is short of the data it claimed.
    pub(super) fn finish(&self) -> Result<(), SampleError> {
        match self.pending.front() {
            Some(pending) => Err(SampleError::unfinished_sample(
                pending.settled.track_id,
                pending.settled.declared_len(),
                pending.gathered_len(),
            )),
            None => Ok(()),
        }
    }

    /// Hands over the samples at the front of the queue that hold every byte they claimed
    ///
    /// The samples are reported in the order the fragments declared them, so one
    /// whole is held back while a sample declared before it is still short.
    fn report_whole(&mut self) {
        while self.pending.front().is_some_and(PendingSample::is_whole) {
            // Why not unwrap: the front was just found to be there, and this
            // `else` stands for a `None` the loop does not reach.
            let Some(whole) = self.pending.pop_front() else {
                break;
            };

            self.ready.push_back(whole.into_sample());
        }
    }
}
