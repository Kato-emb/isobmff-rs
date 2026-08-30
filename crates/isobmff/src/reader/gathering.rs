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

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::{TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox, TrackRunSample};
    use isobmff_core::FullBoxFlags;

    use crate::error::SampleError;
    use crate::reader::SampleReader;
    use crate::reader::tests::{
        MOVIE_FRAGMENT_LEN, drained, movie_fragment, one_sample_movie_fragment, one_track_movie,
        run, sample, track_fragment, track_fragment_header, two_track_movie,
    };
    use crate::sample::Sample;

    #[test]
    fn a_fragment_claiming_data_behind_what_was_read_is_refused() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1, Some(0), None, None),
            None,
            vec![run(Some(8), 1)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();

        assert_eq!(
            reader.handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..100),
            Err(SampleError::backward_data_offset(8, 100))
        );
    }

    #[test]
    fn a_sample_declaring_more_bytes_than_the_limit_is_refused() {
        let mut reader = SampleReader::with_sample_size_limit(&one_track_movie(), 3).unwrap();

        assert_eq!(
            reader.handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN),
            Err(SampleError::sample_size_limit_exceeded(1, 4, 3))
        );
    }

    #[test]
    fn media_data_no_sample_claims_is_passed_over() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();

        reader.handle_media_data(b"XXXX", 200..204).unwrap();
        assert_eq!(reader.poll_sample(), None);
    }

    #[test]
    fn media_data_that_arrived_already_leaves_the_sample_as_it_stands() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();

        reader.handle_media_data(b"AB", 100..102).unwrap();
        reader.handle_media_data(b"XXCD", 100..104).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn a_sample_fills_from_its_start() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();

        reader.handle_media_data(b"CD", 102..104).unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader.handle_media_data(b"ABCD", 100..104).unwrap();
        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn a_fragment_is_taken_while_the_samples_of_the_one_before_it_are_still_short() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 104..204)
            .unwrap();

        reader.handle_media_data(b"EFGH", 204..208).unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader.handle_media_data(b"ABCD", 100..104).unwrap();
        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
    }

    #[test]
    fn samples_declaring_no_bytes_are_reported_before_any_media_data_arrives() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                Some(0),
            ),
            None,
            vec![run(Some(i32::try_from(MOVIE_FRAGMENT_LEN).unwrap()), 2)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.finish().unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b""), sample(1_024, b"")]);
    }

    #[test]
    fn a_sample_declaring_no_bytes_waits_on_the_ones_declared_before_it() {
        let rows = vec![
            TrackRunSample::new(None, Some(4), None, None).unwrap(),
            TrackRunSample::new(None, Some(0), None, None).unwrap(),
        ];
        let trun =
            TrackRunBox::new(Some(i32::try_from(MOVIE_FRAGMENT_LEN).unwrap()), None, rows).unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(
                movie_fragment(vec![track_fragment(vec![trun])]),
                0..MOVIE_FRAGMENT_LEN,
            )
            .unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader.handle_media_data(b"ABCD", 100..104).unwrap();
        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"")]
        );
    }

    #[test]
    fn samples_are_reported_in_the_order_the_fragments_declare_them() {
        let of_track = |track_id, data_offset| {
            TrackFragmentBox::new(
                track_fragment_header(
                    TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                    track_id,
                    None,
                    None,
                    None,
                ),
                None,
                vec![run(Some(data_offset), 1)],
            )
            .unwrap()
        };
        let two_tracks = two_track_movie();

        let mut reader = SampleReader::new(&two_tracks).unwrap();
        reader
            .handle_movie_fragment(
                movie_fragment(vec![of_track(2, 104), of_track(1, 100)]),
                0..MOVIE_FRAGMENT_LEN,
            )
            .unwrap();
        reader.handle_media_data(b"ABCDEFGH", 100..108).unwrap();

        assert_eq!(
            drained(&mut reader),
            [
                Sample::new(2, 0, 1_024, None, 0, 1, b"EFGH".to_vec()),
                sample(0, b"ABCD"),
            ]
        );
    }

    #[test]
    fn a_sample_short_of_its_data_is_refused_when_the_samples_are_declared_over() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"AB", 100..102).unwrap();

        assert_eq!(
            reader.finish(),
            Err(SampleError::unfinished_sample(1, 4, 2))
        );
    }

    #[test]
    fn media_data_covering_a_different_number_of_bytes_than_it_holds_is_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();

        assert_eq!(
            reader.handle_media_data(b"ABCD", 100..106),
            Err(SampleError::extent_length_mismatch(6, 4))
        );
    }
}
