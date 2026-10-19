//! [`SampleGathering`], what a movie demuxer does with the extents it resolves: [`SampleReader`] gathers the samples, [`SampleExtents`] keeps the extents

use alloc::collections::VecDeque;
use core::ops::Range;

use crate::DemuxError;
use crate::sample::{self, Sample, SampleExtent, SampleReader};

mod sealed {
    use core::ops::Range;

    use crate::DemuxError;
    use crate::sample::{self, SampleExtent};

    /// The verbs a demuxer speaks to its gathering, closing [`SampleGathering`](super::SampleGathering) to the types beside it
    #[allow(
        unnameable_types,
        reason = "sealing SampleGathering takes a supertrait a caller cannot name"
    )]
    pub trait Gathering {
        /// Takes the extents a `moov` or a `moof` resolved, as their iterator hands them over
        ///
        /// # Errors
        ///
        /// * [`Sample`](crate::DemuxError::Sample): the failure among
        ///   `extents`, where one is, or what the gathering makes of them.
        fn handle_sample_extents(
            &mut self,
            extents: impl IntoIterator<Item = Result<SampleExtent, sample::Error>>,
        ) -> Result<(), DemuxError>;

        /// Returns the bytes of the file the samples lack next, if any
        fn wanted_extent(&self) -> Option<Range<u64>>;

        /// Takes bytes of the file read at `offset` that the samples lack
        ///
        /// # Errors
        ///
        /// * [`Sample`](crate::DemuxError::Sample): what the samples make
        ///   of the bytes.
        fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), DemuxError>;

        /// Checks, once the file is over, that no sample is short of its bytes
        ///
        /// # Errors
        ///
        /// * [`Sample`](crate::DemuxError::Sample): a sample is short of
        ///   the data it claimed.
        fn finish(&mut self) -> Result<(), DemuxError>;

        /// Drops what is held: the extents, the samples and the bytes they lack
        fn clear(&mut self);
    }
}

/// What a movie demuxer does with the extents it resolves, and what it yields for each sample
///
/// The sample tables of a movie, and each fragment against the movie it
/// continues, resolve into the extents of their samples (ISO/IEC 14496-12
/// §8.7, §8.8): where each lies in the file and what is true of it. What
/// becomes of those extents is the gathering's. [`SampleReader`] holds them
/// and fills them out of the media data, yielding each [`Sample`] once its
/// bytes have arrived; [`SampleExtents`] holds them as they are and yields
/// each [`SampleExtent`], the media data passed over unread.
/// [`MovieDemuxer`](crate::MovieDemuxer) takes one as its type parameter, and
/// what the caller takes from it is the gathering's [`Output`](Self::Output).
///
/// The trait is sealed: the two types here implement it, and no other.
pub trait SampleGathering: sealed::Gathering {
    /// What is yielded for each sample
    type Output;

    /// Takes the next output the input so far completed, or `None` once they are used up
    fn poll_output(&mut self) -> Option<Self::Output>;
}

impl sealed::Gathering for SampleReader {
    fn handle_sample_extents(
        &mut self,
        extents: impl IntoIterator<Item = Result<SampleExtent, sample::Error>>,
    ) -> Result<(), DemuxError> {
        Self::handle_sample_extents(self, extents).map_err(DemuxError::from)
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        Self::wanted_extent(self)
    }

    fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), DemuxError> {
        Self::handle_input(self, offset, input).map_err(DemuxError::from)
    }

    fn finish(&mut self) -> Result<(), DemuxError> {
        Self::finish(self).map_err(DemuxError::from)
    }

    fn clear(&mut self) {
        Self::clear(self);
    }
}

impl SampleGathering for SampleReader {
    type Output = Sample;

    fn poll_output(&mut self) -> Option<Sample> {
        self.poll_sample()
    }
}

/// The extents a movie demuxer resolves, held as they are and yielded in place of samples
///
/// [`MovieDemuxer`](crate::MovieDemuxer) gathering with this type reads a movie file through the same
/// layers it otherwise does, short of gathering the samples: the media data,
/// like every box passed over, goes unread, so the file is read once, in
/// order, whatever order its media data lies in, and no bytes are wanted but
/// the continuation of the input. The extents of a `moov` come out in the
/// order their bytes lie in the file, then those of each `moof` in the order
/// it declares them, up to the first the resolution refuses. Of the
/// [`DemuxLimits`](crate::DemuxLimits) the demuxer is created
/// with, those of
/// [`DemuxLimits::sample_reader`](crate::DemuxLimits::sample_reader)
/// do not apply: nothing is gathered.
#[derive(Debug, Default)]
pub struct SampleExtents {
    extents: VecDeque<SampleExtent>,
}

impl SampleExtents {
    /// Creates a gathering holding no extent
    #[must_use]
    pub const fn new() -> Self {
        Self {
            extents: VecDeque::new(),
        }
    }
}

impl sealed::Gathering for SampleExtents {
    fn handle_sample_extents(
        &mut self,
        extents: impl IntoIterator<Item = Result<SampleExtent, sample::Error>>,
    ) -> Result<(), DemuxError> {
        for extent in extents {
            self.extents.push_back(extent?);
        }

        Ok(())
    }

    fn wanted_extent(&self) -> Option<Range<u64>> {
        None
    }

    fn handle_input(&mut self, _offset: u64, _input: &[u8]) -> Result<(), DemuxError> {
        Ok(())
    }

    fn finish(&mut self) -> Result<(), DemuxError> {
        Ok(())
    }

    fn clear(&mut self) {
        self.extents.clear();
    }
}

impl SampleGathering for SampleExtents {
    type Output = SampleExtent;

    fn poll_output(&mut self) -> Option<SampleExtent> {
        self.extents.pop_front()
    }
}
