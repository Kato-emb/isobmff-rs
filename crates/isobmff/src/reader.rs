//! [`SampleReader`], the samples of a fragmented presentation read as it arrives

mod fragment;
mod gathering;

use core::ops::Range;

use isobmff_boxes::{MovieBox, MovieFragmentBox};

use crate::error::SampleError;
use crate::reader::fragment::MovieTracks;
use crate::reader::gathering::SampleGathering;
use crate::sample::Sample;

/// Where the reader stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking fragments and media data as they arrive
    Reading,
    /// Told the samples are over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(SampleError),
}

/// Reads the samples a fragmented presentation carries, taking it as it arrives
///
/// The reader is set up from the `moov` of the presentation and then handed the
/// movie fragments and the media data that follow, each with the bytes of the
/// presentation it covers. It resolves where every sample lies and what is true
/// of it, and reports the ones whose data has arrived as owned [`Sample`]s. It
/// reaches for no source of its own: when to read and from where stay with the
/// caller.
///
/// The two halves of a fragmented presentation reach it separately —
/// [`handle_movie_fragment`](Self::handle_movie_fragment) takes a `moof` read
/// into a value, [`handle_media_data`](Self::handle_media_data) takes bytes.
/// The reader is scoped to a fragment and carries no notion of a file, so what
/// a file is laid out as stays with [`FragmentedReader`](crate::FragmentedReader),
/// which drives one of these from the boxes it frames.
///
/// # Contract
///
/// * Extents count from the start of the presentation, wherever the caller read
///   it from. A layer framing the file reports where a box lay in what it was
///   handed, so the caller adds the origin it read from before passing it on.
/// * The extent handed to
///   [`handle_media_data`](Self::handle_media_data) is the extent of the bytes
///   handed with it, and covers as many bytes as they hold. Which box those
///   bytes came from is not asked: media data no sample claims is passed over.
/// * Samples are reported in the order the fragments declare them — by `traf`,
///   then by `trun`, then by row. A sample whose data has arrived while one
///   declared before it is still short waits for it.
/// * The claims of a fragment lie ahead of what the reader has read: a fragment
///   claiming data behind that is
///   [`BackwardDataOffset`](crate::SampleErrorKind::BackwardDataOffset) and
///   fails the reader for good. Reading a presentation whose media data lies
///   before the fragment declaring it is the work of a random-access reader,
///   which this is not.
/// * The data of one sample is gathered from its start. Media data arriving for
///   a stretch of it the reader has not reached yet is passed over, so a caller
///   handing the presentation over in the order it lies loses nothing.
/// * Media data for a stretch of a sample that has arrived already is passed
///   over: what arrived first is what the sample carries.
/// * The caller drains before handing over more. Samples are held until they
///   are taken, and so are the claims of a fragment whose data has not arrived,
///   so reading on without polling has the reader hold the whole presentation.
/// * A fragment may be handed over while the samples of the one before it are
///   still short: what the two claim is held side by side.
/// * An `Err` leaves the reader failed for good,
///   [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished) aside: every
///   later call reports that same failure again. The samples completed before
///   it are still there to take, and no further one is ever completed.
/// * [`finish`](Self::finish) declares the samples over, and a claim still
///   short of its data is
///   [`UnfinishedSample`](crate::SampleErrorKind::UnfinishedSample). Samples are
///   still taken after it, but anything handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished).
///
/// # Decode times
///
/// A sample is placed on the media timeline of its track by the `tfdt` of its
/// fragment (ISO/IEC 14496-12 §8.8.12), or, where the fragment carries none, by
/// the durations of the samples read before it. The first fragment of a track
/// carrying no `tfdt` starts at zero.
///
/// A `tfdt` is taken as the absolute decode time it is defined to be, whatever
/// the durations read so far sum to. Where §8.8.12 has a gap close by extending
/// the duration of the sample before it, this reader does not. The durations
/// reported therefore need not sum to the decode time the next fragment states.
///
/// # Examples
///
/// ```
/// use isobmff::{
///     MovieFragmentBox, MovieFragmentHeaderBox, SampleReader, TrackExtendsBox,
///     TrackFragmentBox, TrackFragmentHeaderBox, TrackRunBox, TrackRunSample,
/// };
/// # use isobmff_test_support::fragmented_movie;
/// // A movie whose one track is fragmented, its samples lasting 1024 units each
/// let movie = fragmented_movie(TrackExtendsBox::new(1, 1, 1_024, 0, 0));
/// let mut reader = SampleReader::new(&movie).unwrap();
///
/// // One fragment of two samples, anchored at the fragment itself and starting
/// // 96 bytes into it — past the fragment and the header of the `mdat` beside it
/// let track_fragment = TrackFragmentBox::new(
///     TrackFragmentHeaderBox::new(
///         TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
///         1,
///         None,
///         None,
///         None,
///         None,
///         None,
///     )
///     .unwrap(),
///     None,
///     vec![
///         TrackRunBox::new(
///             Some(96),
///             None,
///             vec![
///                 TrackRunSample::new(None, Some(4), None, None).unwrap(),
///                 TrackRunSample::new(None, Some(4), None, None).unwrap(),
///             ],
///         )
///         .unwrap(),
///     ],
/// )
/// .unwrap();
/// let movie_fragment =
///     MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![track_fragment]);
///
/// // The fragment is claimed first, and nothing is ready until its data arrives
/// reader.handle_movie_fragment(movie_fragment, 0..88).unwrap();
/// assert_eq!(reader.poll_sample(), None);
///
/// // The payload of the `mdat` completes both samples
/// reader.handle_media_data(b"SAMPDATA", 96..104).unwrap();
///
/// // Nothing is left claiming data, so the samples are declared over
/// reader.finish().unwrap();
///
/// // Each is placed on the media timeline by the duration the `trex` sets
/// let first = reader.poll_sample().unwrap();
/// assert_eq!((first.data(), first.decode_time()), (b"SAMP".as_slice(), 0));
///
/// let second = reader.poll_sample().unwrap();
/// assert_eq!(
///     (second.data(), second.decode_time()),
///     (b"DATA".as_slice(), 1_024)
/// );
/// assert_eq!(reader.poll_sample(), None);
/// ```
#[derive(Debug)]
pub struct SampleReader {
    tracks: MovieTracks,
    gathering: SampleGathering,
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

    /// Creates a reader for the samples the fragments of `movie` carry
    ///
    /// What one sample may declare is bounded by
    /// [`DEFAULT_SAMPLE_SIZE_LIMIT`](Self::DEFAULT_SAMPLE_SIZE_LIMIT).
    ///
    /// # Errors
    ///
    /// * The failures of
    ///   [`with_sample_size_limit`](Self::with_sample_size_limit).
    pub fn new(movie: &MovieBox) -> Result<Self, SampleError> {
        Self::with_sample_size_limit(movie, Self::DEFAULT_SAMPLE_SIZE_LIMIT)
    }

    /// Creates a reader gathering no more than `sample_size_limit` bytes for one sample
    ///
    /// A sample is gathered whole before it is reported, so the length it
    /// declares is memory the reader is about to take. One declaring more than
    /// `sample_size_limit` bytes is
    /// [`SampleSizeLimitExceeded`](crate::SampleErrorKind::SampleSizeLimitExceeded)
    /// instead, reported before a byte of it is gathered.
    ///
    /// The limit bounds one sample rather than the presentation: it is checked
    /// against the length a fragment declares for a sample, not against what the
    /// samples before it took.
    ///
    /// A `trex` for a track the movie does not declare sets no defaults anyone
    /// falls back on, and is passed over. Two tracks declaring the same
    /// `track_id`, which §8.3.2.3 has identify a track uniquely, leave the first
    /// of them standing — a movie
    /// [`MovieBox::new`](isobmff_boxes::MovieBox::new) refuses to build, but a
    /// decoded one may carry.
    ///
    /// # Errors
    ///
    /// * [`MissingMovieExtends`](crate::SampleErrorKind::MissingMovieExtends):
    ///   the movie carries no `mvex`, and so continues in no fragments.
    pub fn with_sample_size_limit(
        movie: &MovieBox,
        sample_size_limit: u64,
    ) -> Result<Self, SampleError> {
        Ok(Self {
            tracks: MovieTracks::of(movie)?,
            gathering: SampleGathering::new(sample_size_limit),
            state: State::Reading,
        })
    }

    /// Takes a movie fragment, and claims the data of the samples it declares
    ///
    /// `extent` is the bytes of the presentation the `moof` occupies, which the
    /// data offsets of its fragments are resolved against (§8.8.7.1). The
    /// samples it declares are reported once their data has arrived, from
    /// [`poll_sample`](Self::poll_sample). A sample declaring no bytes holds
    /// every byte it claims as soon as it is claimed, and is reported without
    /// any media data arriving for it.
    ///
    /// The fragment is resolved whole before any of its claims is held, so a
    /// fragment failing on more than one count is reported by whichever of them
    /// resolving it reaches first.
    ///
    /// # Errors
    ///
    /// * [`UnknownTrackId`](crate::SampleErrorKind::UnknownTrackId): a `traf`
    ///   carries samples of a track the movie never declared.
    /// * [`BackwardDataOffset`](crate::SampleErrorKind::BackwardDataOffset): a
    ///   sample resolves to data lying behind what the reader has read.
    /// * [`SampleSizeLimitExceeded`](crate::SampleErrorKind::SampleSizeLimitExceeded):
    ///   a sample declares more bytes than the limit the reader was given.
    /// * [`DataOffsetOverflow`](crate::SampleErrorKind::DataOffsetOverflow): the
    ///   offsets a fragment states run past what 64 bits carry.
    /// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow): the
    ///   decode times of a track run past what 64 bits carry.
    /// * [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_movie_fragment(
        &mut self,
        movie_fragment: MovieFragmentBox,
        extent: Range<u64>,
    ) -> Result<(), SampleError> {
        self.reading()?;

        let settled = self
            .tracks
            .settle(&movie_fragment, extent.start)
            .map_err(|failure| self.fail(failure))?;

        self.gathering
            .claim(settled, extent.end)
            .map_err(|failure| self.fail(failure))
    }

    /// Takes media data that arrived, and fills the samples claiming it
    ///
    /// `extent` is the bytes of the presentation `data` holds, and covers as
    /// many bytes as it holds. Which box the bytes came from is not asked: a
    /// stretch no sample claims is passed over, and so is one that arrived
    /// already. The samples it completes are then taken from
    /// [`poll_sample`](Self::poll_sample).
    ///
    /// # Errors
    ///
    /// * [`ExtentLengthMismatch`](crate::SampleErrorKind::ExtentLengthMismatch):
    ///   `extent` covers a different number of bytes than `data` holds.
    /// * [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn handle_media_data(
        &mut self,
        data: &[u8],
        extent: Range<u64>,
    ) -> Result<(), SampleError> {
        self.reading()?;

        self.gathering
            .fill(data, &extent)
            .map_err(|failure| self.fail(failure))
    }

    /// Takes the next sample the fragments and media data handed over completed
    ///
    /// Reports `None` once they are used up: more of the presentation is
    /// needed. Failure is reported by the calls that take it, so this one never
    /// fails — a failed reader hands over the samples it had already completed,
    /// then `None` from there on.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.gathering.poll()
    }

    /// Declares the samples over
    ///
    /// # Errors
    ///
    /// * [`UnfinishedSample`](crate::SampleErrorKind::UnfinishedSample): a
    ///   sample a fragment declared is short of the data it claimed.
    /// * [`AlreadyFinished`](crate::SampleErrorKind::AlreadyFinished): the
    ///   samples were already declared over.
    /// * The failure of a previous call, which the reader keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), SampleError> {
        self.reading()?;

        self.gathering
            .finish()
            .map_err(|failure| self.fail(failure))?;

        self.state = State::Finished;

        Ok(())
    }

    /// Returns `Ok` while the reader still takes what arrives
    fn reading(&self) -> Result<(), SampleError> {
        match self.state {
            State::Reading => Ok(()),
            State::Finished => Err(SampleError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the reader for good, and hands the failure back to report
    fn fail(&mut self, failure: SampleError) -> SampleError {
        self.state = State::Failed(failure);

        failure
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        MovieExtendsBox, MovieFragmentHeaderBox, MovieHeaderBox, TrackExtendsBox, TrackFragmentBox,
        TrackFragmentHeaderBox, TrackRunBox, TrackRunSample,
    };
    use isobmff_core::{BoxDecode as _, BoxEncode as _, FullBoxFlags, Mp4EpochSeconds};
    use isobmff_test_support::{track, written};

    use super::{MovieBox, MovieFragmentBox, Sample, SampleError, SampleReader};

    /// Bytes the movie fragment of most of these tests is given
    pub(super) const MOVIE_FRAGMENT_LEN: u64 = 100;

    /// Movie declaring the given tracks, fragmented by the given extends boxes
    pub(super) fn movie(track_ids: &[u32], trex: Vec<TrackExtendsBox>) -> MovieBox {
        MovieBox::new(
            MovieHeaderBox::new(
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                1_000,
                0,
                2,
            ),
            track_ids.iter().copied().map(track).collect(),
            MovieExtendsBox::new(trex),
        )
        .unwrap()
    }

    /// Movie of one track whose samples last 1024 units and occupy 4 bytes each
    pub(super) fn one_track_movie() -> MovieBox {
        movie(&[1], vec![TrackExtendsBox::new(1, 1, 1_024, 4, 0)])
    }

    /// Movie of two tracks colliding on one `track_id`
    pub(super) fn movie_of_one_track_id_twice() -> MovieBox {
        // Why not MovieBox::new: it refuses a movie declaring one track_id twice,
        // so a reader only ever meets the collision through a decode.
        let declared = one_track_movie();
        let mut payload = vec![0; usize::try_from(declared.payload_len()).unwrap()];
        declared.encode_payload(&mut payload).unwrap();

        MovieBox::decode_payload(&[payload, written(&track(1))].concat()).unwrap()
    }

    /// Fragment header of one track, carrying the flags and defaults given
    pub(super) fn track_fragment_header(
        flags: FullBoxFlags,
        track_id: u32,
        base_data_offset: Option<u64>,
        default_sample_duration: Option<u32>,
        default_sample_size: Option<u32>,
    ) -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox::new(
            flags,
            track_id,
            base_data_offset,
            None,
            default_sample_duration,
            default_sample_size,
            None,
        )
        .unwrap()
    }

    /// Run of samples that take the size and duration of their defaults
    pub(super) fn run(data_offset: Option<i32>, sample_count: u32) -> TrackRunBox {
        let rows = (0..sample_count)
            .map(|_| TrackRunSample::new(None, None, None, None).unwrap())
            .collect();

        TrackRunBox::new(data_offset, None, rows).unwrap()
    }

    /// Fragment of track 1, anchored at the movie fragment, of runs of four-byte samples
    pub(super) fn track_fragment(trun: Vec<TrackRunBox>) -> TrackFragmentBox {
        TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                None,
            ),
            None,
            trun,
        )
        .unwrap()
    }

    /// Movie fragment carrying the given track fragments
    pub(super) fn movie_fragment(traf: Vec<TrackFragmentBox>) -> MovieFragmentBox {
        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), traf)
    }

    /// Movie fragment claiming one four-byte sample of track 1, just past itself
    pub(super) fn one_sample_movie_fragment() -> MovieFragmentBox {
        movie_fragment(vec![track_fragment(vec![run(
            Some(i32::try_from(MOVIE_FRAGMENT_LEN).unwrap()),
            1,
        )])])
    }

    /// Movie of two fragmented tracks whose samples last 1024 units and occupy 4 bytes each
    pub(super) fn two_track_movie() -> MovieBox {
        movie(
            &[1, 2],
            vec![
                TrackExtendsBox::new(1, 1, 1_024, 4, 0),
                TrackExtendsBox::new(2, 1, 1_024, 4, 0),
            ],
        )
    }

    /// Sample of track 1 as the defaults of `one_track_movie` settle it
    pub(super) fn sample(decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(1, decode_time, 1_024, None, 0, 1, data.to_vec())
    }

    /// Takes every sample the reader has completed
    pub(super) fn drained(reader: &mut SampleReader) -> Vec<Sample> {
        let mut samples = Vec::new();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }

        samples
    }

    /// Reads one movie fragment of `sample_count` four-byte samples, and the data of it
    pub(super) fn read_one_fragment(sample_count: u32, data: &[u8]) -> Vec<Sample> {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();

        reader
            .handle_movie_fragment(
                movie_fragment(vec![track_fragment(vec![run(
                    Some(i32::try_from(MOVIE_FRAGMENT_LEN).unwrap()),
                    sample_count,
                )])]),
                0..MOVIE_FRAGMENT_LEN,
            )
            .unwrap();
        reader
            .handle_media_data(
                data,
                MOVIE_FRAGMENT_LEN..MOVIE_FRAGMENT_LEN.saturating_add(data.len() as u64),
            )
            .unwrap();
        reader.finish().unwrap();

        drained(&mut reader)
    }

    #[test]
    fn a_failure_is_reported_again_for_every_call_after_it() {
        let mut reader = SampleReader::with_sample_size_limit(&one_track_movie(), 3).unwrap();
        let failure = reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap_err();

        assert_eq!(reader.handle_media_data(b"ABCD", 100..104), Err(failure));
        assert_eq!(reader.finish(), Err(failure));
    }

    #[test]
    fn samples_completed_before_a_failure_are_still_taken() {
        let mut reader = SampleReader::with_sample_size_limit(&one_track_movie(), 4).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        let oversized = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                Some(8),
            ),
            None,
            vec![run(Some(200), 1)],
        )
        .unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![oversized]), 104..204)
            .unwrap_err();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn anything_handed_over_after_the_samples_were_declared_over_is_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader.finish().unwrap();

        assert_eq!(
            reader.handle_media_data(b"ABCD", 100..104),
            Err(SampleError::already_finished())
        );
        assert_eq!(reader.finish(), Err(SampleError::already_finished()));
    }
}
