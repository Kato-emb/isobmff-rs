//! [`SampleReader`], the samples of a fragmented presentation read as it arrives

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::ops::Range;

use isobmff_boxes::{MovieBox, MovieFragmentBox};

use crate::error::SampleError;
use crate::sample::Sample;

/// Defaults one track sets for its fragments, and where its timeline stands
///
/// The defaults are the ones the `trex` of the track declares (§8.8.3), which a
/// `tfhd` overrides for one fragment and a `trun` row for one sample. The
/// decode time is the running one: where the samples read so far leave the
/// media timeline of this track, and so where a fragment carrying no `tfdt`
/// starts.
#[derive(Clone, Copy, Debug)]
struct Track {
    default_sample_description_index: u32,
    default_sample_duration: u32,
    default_sample_size: u32,
    default_sample_flags: u32,
    decode_time: u64,
}

/// Sample a fragment has claimed the data of, gathered as far as it has arrived
///
/// `extent` is the bytes of the presentation the sample was resolved to, and
/// `data` holds them from its start: the length gathered is how far into the
/// extent the sample is whole, so a claim is met when the two lengths meet.
#[derive(Clone, Debug)]
struct PendingSample {
    track_id: u32,
    decode_time: u64,
    sample_duration: u32,
    sample_composition_time_offset: Option<i64>,
    sample_flags: u32,
    sample_description_index: u32,
    extent: Range<u64>,
    data: Vec<u8>,
}

impl PendingSample {
    /// Returns the bytes the sample was declared to occupy
    fn declared_len(&self) -> u64 {
        self.extent.end.saturating_sub(self.extent.start)
    }

    /// Returns the bytes of the sample that have arrived
    fn gathered_len(&self) -> u64 {
        self.data.len() as u64
    }

    /// Returns whether every byte the sample was declared to occupy has arrived
    fn is_whole(&self) -> bool {
        self.gathered_len() >= self.declared_len()
    }

    /// Takes off `arriving` the bytes of this sample that come next
    ///
    /// The sample fills from its start, so what is taken is the run beginning
    /// where the bytes gathered so far end and reaching no further than the end
    /// of the sample. A sample already whole, media data ending before what it
    /// holds already, and media data starting past the point it fills from each
    /// leave it as it stands.
    fn take_from(&mut self, arriving: &[u8], extent: &Range<u64>) {
        let filled_end = self.extent.start.saturating_add(self.gathered_len());
        if self.is_whole() || !extent.contains(&filled_end) {
            return;
        }

        let taken_from = filled_end.saturating_sub(extent.start);
        let taken_to = extent.end.min(self.extent.end).saturating_sub(extent.start);
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
            self.track_id,
            self.decode_time,
            self.sample_duration,
            self.sample_composition_time_offset,
            self.sample_flags,
            self.sample_description_index,
            self.data,
        )
    }
}

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
/// into a value, [`handle_media_data`](Self::handle_media_data) takes bytes —
/// so wiring it to
/// [`BoxReader`](https://docs.rs/isobmff-sequence/latest/isobmff_sequence/struct.BoxReader.html)
/// is a match on two of its events.
///
/// # Contract
///
/// * Extents count from the start of the presentation, wherever the caller read
///   it from. A box layer reports where an event lay in what it was handed, so
///   the caller adds the origin it read from before passing it on.
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
///     MovieFragmentBox, MovieFragmentHeaderBox, SampleReader, TrackFragmentBox,
///     TrackFragmentHeaderBox, TrackRunBox, TrackRunSample,
/// };
/// # use isobmff::{
/// #     ChunkOffsetBox, DataEntry, DataEntryUrlBox, DataInformationBox, DataReferenceBox, FourCC,
/// #     FullBoxFlags, HandlerBox, LanguageCode, MediaBox, MediaHeaderBox, MediaInformationBox,
/// #     MediaInformationHeader, MovieBox, MovieExtendsBox, MovieHeaderBox, Mp4EpochSeconds,
/// #     NullTerminatedString, SampleDescriptionBox, SampleSizeBox, SampleSizes, SampleTableBox,
/// #     SampleToChunkBox, TimeToSampleBox, TrackBox, TrackExtendsBox, TrackHeaderBox,
/// #     VideoMediaHeaderBox,
/// # };
/// # fn movie() -> MovieBox {
/// #     let sample_table = SampleTableBox::new(
/// #         SampleDescriptionBox::new(vec![]),
/// #         TimeToSampleBox::new(vec![]),
/// #         SampleToChunkBox::new(vec![]),
/// #         SampleSizeBox::new(SampleSizes::PerSample(vec![])),
/// #         ChunkOffsetBox::new(vec![]),
/// #     );
/// #     let media_information = MediaInformationBox::new(
/// #         MediaInformationHeader::Video(VideoMediaHeaderBox::new(0, [0, 0, 0])),
/// #         DataInformationBox::new(DataReferenceBox::new(vec![DataEntry::Url(
/// #             DataEntryUrlBox::new(None),
/// #         )])),
/// #         sample_table,
/// #     );
/// #     let media = MediaBox::new(
/// #         MediaHeaderBox::new(
/// #             Mp4EpochSeconds::from_seconds(0),
/// #             Mp4EpochSeconds::from_seconds(0),
/// #             90_000,
/// #             0,
/// #             LanguageCode::from_letters(b"und").unwrap(),
/// #         ),
/// #         HandlerBox::new(
/// #             FourCC::new(*b"vide"),
/// #             NullTerminatedString::new(String::from("")).unwrap(),
/// #         ),
/// #         media_information,
/// #     );
/// #     let track = TrackBox::new(
/// #         TrackHeaderBox::new(
/// #             FullBoxFlags::new(3).unwrap(),
/// #             Mp4EpochSeconds::from_seconds(0),
/// #             Mp4EpochSeconds::from_seconds(0),
/// #             1,
/// #             0,
/// #         ),
/// #         media,
/// #     );
/// #     MovieBox::new(
/// #         MovieHeaderBox::new(
/// #             Mp4EpochSeconds::from_seconds(0),
/// #             Mp4EpochSeconds::from_seconds(0),
/// #             1_000,
/// #             0,
/// #             2,
/// #         ),
/// #         vec![track],
/// #         Some(MovieExtendsBox::new(vec![TrackExtendsBox::new(1, 1, 1_024, 0, 0)]).unwrap()),
/// #     )
/// #     .unwrap()
/// # }
/// // A reader set up from a movie whose one track is fragmented, its samples
/// // lasting 1024 units each
/// let mut reader = SampleReader::new(&movie()).unwrap();
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
    tracks: BTreeMap<u32, Track>,
    pending: VecDeque<PendingSample>,
    ready: VecDeque<Sample>,
    read_so_far: u64,
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
        let Some(mvex) = movie.mvex() else {
            return Err(SampleError::missing_movie_extends());
        };

        let mut tracks = BTreeMap::new();
        for trak in movie.trak() {
            let track_id = trak.tkhd().track_id();
            if let Some(trex) = mvex.trex().iter().find(|trex| trex.track_id() == track_id) {
                tracks.entry(track_id).or_insert(Track {
                    default_sample_description_index: trex.default_sample_description_index(),
                    default_sample_duration: trex.default_sample_duration(),
                    default_sample_size: trex.default_sample_size(),
                    default_sample_flags: trex.default_sample_flags(),
                    decode_time: 0,
                });
            }
        }

        Ok(Self {
            tracks,
            pending: VecDeque::new(),
            ready: VecDeque::new(),
            read_so_far: 0,
            sample_size_limit,
            state: State::Reading,
        })
    }

    /// Takes a movie fragment, and claims the data of the samples it declares
    ///
    /// `extent` is the bytes of the presentation the `moof` occupies, which the
    /// data offsets of its fragments are resolved against (§8.8.7.1). The
    /// samples it declares are reported once their data has arrived, from
    /// [`poll_sample`](Self::poll_sample).
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
        self.read_so_far = self.read_so_far.max(extent.end);

        self.claim(&movie_fragment, extent.start)
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

        let covered = extent.end.saturating_sub(extent.start);
        let offered = data.len() as u64;
        if covered != offered {
            return Err(self.fail(SampleError::extent_length_mismatch(covered, offered)));
        }

        self.read_so_far = self.read_so_far.max(extent.end);
        for pending in &mut self.pending {
            pending.take_from(data, &extent);
        }

        while self.pending.front().is_some_and(PendingSample::is_whole) {
            // Why not unwrap: the front was just found to be there, and this
            // `else` stands for a `None` the loop does not reach.
            let Some(whole) = self.pending.pop_front() else {
                break;
            };

            self.ready.push_back(whole.into_sample());
        }

        Ok(())
    }

    /// Takes the next sample the fragments and media data handed over completed
    ///
    /// Reports `None` once they are used up: more of the presentation is
    /// needed. Failure is reported by the calls that take it, so this one never
    /// fails — a failed reader hands over the samples it had already completed,
    /// then `None` from there on.
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.ready.pop_front()
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

        if let Some(pending) = self.pending.front() {
            let failure = SampleError::unfinished_sample(
                pending.track_id,
                pending.declared_len(),
                pending.gathered_len(),
            );
            return Err(self.fail(failure));
        }

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

    /// Resolves the samples `movie_fragment` declares, and claims the data of each
    ///
    /// `moof_start` is where the fragment begins. §8.8.7.1 anchors the offsets
    /// of a track fragment at the `base_data_offset` it states, at the fragment
    /// itself where it sets `default-base-is-moof`, and where it states
    /// neither, at the fragment for the first track fragment and at the end of
    /// the data of the one before it for those that follow. §8.8.8 has a run
    /// stating no offset of its own start where the run before it ended.
    ///
    /// A fragment declaring an empty duration carries no samples, and moves the
    /// timeline of its track on by the default duration alone (§8.8.7.1).
    fn claim(
        &mut self,
        movie_fragment: &MovieFragmentBox,
        moof_start: u64,
    ) -> Result<(), SampleError> {
        let mut fragment_data_end = None;

        for traf in movie_fragment.traf() {
            let tfhd = traf.tfhd();
            let track_id = tfhd.track_id();
            let Some(track) = self.tracks.get(&track_id).copied() else {
                return Err(SampleError::unknown_track_id(track_id));
            };

            let base_data_offset = if let Some(explicit) = tfhd.base_data_offset() {
                explicit
            } else if tfhd.default_base_is_moof() {
                moof_start
            } else {
                fragment_data_end.unwrap_or(moof_start)
            };

            let mut decode_time = traf
                .tfdt()
                .map_or(track.decode_time, |tfdt| tfdt.base_media_decode_time());
            let default_sample_duration = tfhd
                .default_sample_duration()
                .unwrap_or(track.default_sample_duration);
            let default_sample_size = tfhd
                .default_sample_size()
                .unwrap_or(track.default_sample_size);
            let default_sample_flags = tfhd
                .default_sample_flags()
                .unwrap_or(track.default_sample_flags);
            let sample_description_index = tfhd
                .sample_description_index()
                .unwrap_or(track.default_sample_description_index);

            let mut data_offset = base_data_offset;
            if tfhd.duration_is_empty() {
                decode_time = decode_time
                    .checked_add(u64::from(default_sample_duration))
                    .ok_or(SampleError::decode_time_overflow(track_id))?;
            }

            for trun in traf.trun() {
                if let Some(stated) = trun.data_offset() {
                    data_offset = base_data_offset
                        .checked_add_signed(i64::from(stated))
                        .ok_or(SampleError::data_offset_overflow(track_id))?;
                }

                let mut first_sample_flags = trun.first_sample_flags();
                for row in trun.samples() {
                    let declared = u64::from(row.sample_size().unwrap_or(default_sample_size));
                    if declared > self.sample_size_limit {
                        return Err(SampleError::sample_size_limit_exceeded(
                            track_id,
                            declared,
                            self.sample_size_limit,
                        ));
                    }
                    if data_offset < self.read_so_far {
                        return Err(SampleError::backward_data_offset(
                            data_offset,
                            self.read_so_far,
                        ));
                    }

                    let data_end = data_offset
                        .checked_add(declared)
                        .ok_or(SampleError::data_offset_overflow(track_id))?;
                    let sample_duration = row.sample_duration().unwrap_or(default_sample_duration);

                    self.pending.push_back(PendingSample {
                        track_id,
                        decode_time,
                        sample_duration,
                        sample_composition_time_offset: row.sample_composition_time_offset(),
                        sample_flags: first_sample_flags
                            .take()
                            .or(row.sample_flags())
                            .unwrap_or(default_sample_flags),
                        sample_description_index,
                        extent: data_offset..data_end,
                        data: Vec::new(),
                    });

                    decode_time = decode_time
                        .checked_add(u64::from(sample_duration))
                        .ok_or(SampleError::decode_time_overflow(track_id))?;
                    data_offset = data_end;
                }
            }

            self.tracks.insert(
                track_id,
                Track {
                    decode_time,
                    ..track
                },
            );
            fragment_data_end = Some(data_offset);
        }

        Ok(())
    }

    /// Fails the reader for good, and hands the failure back to report
    fn fail(&mut self, failure: SampleError) -> SampleError {
        self.state = State::Failed(failure);

        failure
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::{
        ChunkOffsetBox, DataEntry, DataEntryUrlBox, DataInformationBox, DataReferenceBox,
        HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, MediaInformationHeader,
        MovieExtendsBox, MovieFragmentHeaderBox, MovieHeaderBox, SampleDescriptionBox,
        SampleSizeBox, SampleSizes, SampleTableBox, SampleToChunkBox, TimeToSampleBox, TrackBox,
        TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
        TrackFragmentHeaderBox, TrackHeaderBox, TrackRunBox, TrackRunSample, VideoMediaHeaderBox,
    };
    use isobmff_core::{
        BoxDecode as _, BoxEncode as _, FourCC, FullBoxFlags, LanguageCode, Mp4EpochSeconds,
        NullTerminatedString,
    };

    use super::{MovieBox, MovieFragmentBox, Sample, SampleError, SampleReader};

    /// Bytes the movie fragment of most of these tests is given
    const MOVIE_FRAGMENT_LEN: u64 = 100;

    /// Track of a fragmented movie, which holds no samples of its own
    fn track(track_id: u32) -> TrackBox {
        let sample_table = SampleTableBox::new(
            SampleDescriptionBox::new(vec![]),
            TimeToSampleBox::new(vec![]),
            SampleToChunkBox::new(vec![]),
            SampleSizeBox::new(SampleSizes::PerSample(vec![])),
            ChunkOffsetBox::new(vec![]),
        );
        let media_information = MediaInformationBox::new(
            MediaInformationHeader::Video(VideoMediaHeaderBox::new(0, [0, 0, 0])),
            DataInformationBox::new(DataReferenceBox::new(vec![DataEntry::Url(
                DataEntryUrlBox::new(None),
            )])),
            sample_table,
        );
        let media = MediaBox::new(
            MediaHeaderBox::new(
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                90_000,
                0,
                LanguageCode::from_letters(b"und").unwrap(),
            ),
            HandlerBox::new(
                FourCC::new(*b"vide"),
                NullTerminatedString::new(String::from("")).unwrap(),
            ),
            media_information,
        );

        TrackBox::new(
            TrackHeaderBox::new(
                FullBoxFlags::new(3).unwrap(),
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                track_id,
                0,
            ),
            media,
        )
    }

    /// Movie declaring the given tracks, fragmented by the given extends boxes
    fn movie(track_ids: &[u32], trex: Vec<TrackExtendsBox>) -> MovieBox {
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
    fn one_track_movie() -> MovieBox {
        movie(&[1], vec![TrackExtendsBox::new(1, 1, 1_024, 4, 0)])
    }

    /// Movie of two tracks colliding on one `track_id`
    fn movie_of_one_track_id_twice() -> MovieBox {
        // Why not MovieBox::new: it refuses a movie declaring one track_id twice,
        // so a reader only ever meets the collision through a decode.
        let declared = one_track_movie();
        let mut payload = vec![0; usize::try_from(declared.payload_len()).unwrap()];
        declared.encode_payload(&mut payload).unwrap();

        let repeated = declared.trak().first().unwrap();
        let mut encoded_track = vec![0; usize::try_from(repeated.encoded_len()).unwrap()];
        repeated.encode(&mut encoded_track).unwrap();

        MovieBox::decode_payload(&[payload, encoded_track].concat()).unwrap()
    }

    /// Fragment header of one track, carrying the flags and defaults given
    fn track_fragment_header(
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
    fn run(data_offset: Option<i32>, sample_count: u32) -> TrackRunBox {
        let rows = (0..sample_count)
            .map(|_| TrackRunSample::new(None, None, None, None).unwrap())
            .collect();

        TrackRunBox::new(data_offset, None, rows).unwrap()
    }

    /// Fragment of track 1, anchored at the movie fragment, of runs of four-byte samples
    fn track_fragment(trun: Vec<TrackRunBox>) -> TrackFragmentBox {
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
    fn movie_fragment(traf: Vec<TrackFragmentBox>) -> MovieFragmentBox {
        MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), traf)
    }

    /// Movie fragment claiming one four-byte sample of track 1, just past itself
    fn one_sample_movie_fragment() -> MovieFragmentBox {
        movie_fragment(vec![track_fragment(vec![run(
            Some(i32::try_from(MOVIE_FRAGMENT_LEN).unwrap()),
            1,
        )])])
    }

    /// Movie of two fragmented tracks whose samples last 1024 units and occupy 4 bytes each
    fn two_track_movie() -> MovieBox {
        movie(
            &[1, 2],
            vec![
                TrackExtendsBox::new(1, 1, 1_024, 4, 0),
                TrackExtendsBox::new(2, 1, 1_024, 4, 0),
            ],
        )
    }

    /// Sample of track 1 as the defaults of `one_track_movie` settle it
    fn sample(decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(1, decode_time, 1_024, None, 0, 1, data.to_vec())
    }

    /// Takes every sample the reader has completed
    fn drained(reader: &mut SampleReader) -> Vec<Sample> {
        let mut samples = Vec::new();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }

        samples
    }

    /// Reads one movie fragment of `sample_count` four-byte samples, and the data of it
    fn read_one_fragment(sample_count: u32, data: &[u8]) -> Vec<Sample> {
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
    fn a_sample_takes_what_its_row_states_over_the_defaults_of_the_fragment_and_the_track() {
        let rows =
            vec![TrackRunSample::new(Some(512), Some(2), Some(0x0100_0000), Some(-8)).unwrap()];
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(256),
                Some(8),
            ),
            None,
            vec![TrackRunBox::new(Some(100), None, rows).unwrap()],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"AB", 100..102).unwrap();

        assert_eq!(
            drained(&mut reader),
            [Sample::new(
                1,
                0,
                512,
                Some(-8),
                0x0100_0000,
                1,
                b"AB".to_vec()
            )]
        );
    }

    #[test]
    fn a_sample_takes_what_its_fragment_states_over_the_defaults_of_its_track() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(256),
                Some(2),
            ),
            None,
            vec![run(Some(100), 2)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(
            drained(&mut reader),
            [
                Sample::new(1, 0, 256, None, 0, 1, b"AB".to_vec()),
                Sample::new(1, 256, 256, None, 0, 1, b"CD".to_vec()),
            ]
        );
    }

    #[test]
    fn the_flags_of_the_first_sample_of_a_run_stand_in_for_the_defaults() {
        let rows = vec![
            TrackRunSample::new(None, None, None, None).unwrap(),
            TrackRunSample::new(None, None, None, None).unwrap(),
        ];
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                None,
                None,
            ),
            None,
            vec![TrackRunBox::new(Some(100), Some(0x0200_0000), rows).unwrap()],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCDEFGH", 100..108).unwrap();

        assert_eq!(
            drained(&mut reader),
            [
                Sample::new(1, 0, 1_024, None, 0x0200_0000, 1, b"ABCD".to_vec()),
                sample(1_024, b"EFGH"),
            ]
        );
    }

    #[test]
    fn a_track_starts_at_zero_where_its_first_fragment_states_no_decode_time() {
        assert_eq!(read_one_fragment(1, b"ABCD"), [sample(0, b"ABCD")]);
    }

    #[test]
    fn samples_are_placed_by_the_durations_of_the_samples_before_them() {
        assert_eq!(
            read_one_fragment(2, b"ABCDEFGH"),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
    }

    #[test]
    fn a_fragment_stating_no_decode_time_carries_on_from_the_one_before_it() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();

        for (fragment_start, data_start) in [(0, 100), (200, 300)] {
            reader
                .handle_movie_fragment(
                    one_sample_movie_fragment(),
                    fragment_start..fragment_start + 100,
                )
                .unwrap();
            reader
                .handle_media_data(b"ABCD", data_start..data_start + 4)
                .unwrap();
        }

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"ABCD")]
        );
    }

    #[test]
    fn a_decode_time_is_taken_as_stated_however_the_durations_read_so_far_sum() {
        let stated = |base_media_decode_time| {
            TrackFragmentBox::new(
                track_fragment_header(
                    TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                    1,
                    None,
                    None,
                    None,
                ),
                Some(TrackFragmentBaseMediaDecodeTimeBox::new(
                    base_media_decode_time,
                )),
                vec![run(Some(100), 1)],
            )
            .unwrap()
        };

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        for (fragment_start, data_start, decode_time) in [(0, 100, 90_000), (200, 300, 4_096)] {
            reader
                .handle_movie_fragment(
                    movie_fragment(vec![stated(decode_time)]),
                    fragment_start..fragment_start + 100,
                )
                .unwrap();
            reader
                .handle_media_data(b"ABCD", data_start..data_start + 4)
                .unwrap();
        }

        assert_eq!(
            drained(&mut reader),
            [sample(90_000, b"ABCD"), sample(4_096, b"ABCD")]
        );
    }

    #[test]
    fn offsets_are_anchored_at_the_base_the_fragment_states() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1, Some(400), None, None),
            None,
            vec![run(Some(8), 1)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 408..412).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn offsets_of_a_fragment_stating_no_anchor_at_all_are_anchored_at_the_movie_fragment() {
        let track_fragment = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1, None, None, None),
            None,
            vec![run(Some(100), 1)],
        )
        .unwrap();

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn offsets_of_a_later_track_fragment_stating_no_anchor_follow_the_data_before_it() {
        let stating_no_anchor = |track_id, data_offset| {
            TrackFragmentBox::new(
                track_fragment_header(FullBoxFlags::ZERO, track_id, None, None, None),
                None,
                vec![run(data_offset, 1)],
            )
            .unwrap()
        };
        let two_tracks = two_track_movie();

        let mut reader = SampleReader::new(&two_tracks).unwrap();
        reader
            .handle_movie_fragment(
                movie_fragment(vec![
                    stating_no_anchor(1, Some(100)),
                    stating_no_anchor(2, None),
                ]),
                0..MOVIE_FRAGMENT_LEN,
            )
            .unwrap();
        reader.handle_media_data(b"ABCDEFGH", 100..108).unwrap();

        assert_eq!(
            drained(&mut reader),
            [
                sample(0, b"ABCD"),
                Sample::new(2, 0, 1_024, None, 0, 1, b"EFGH".to_vec()),
            ]
        );
    }

    #[test]
    fn a_run_stating_no_offset_starts_where_the_run_before_it_ended() {
        let track_fragment = track_fragment(vec![run(Some(100), 1), run(None, 1)]);

        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader
            .handle_movie_fragment(movie_fragment(vec![track_fragment]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCDEFGH", 100..108).unwrap();

        assert_eq!(
            drained(&mut reader),
            [sample(0, b"ABCD"), sample(1_024, b"EFGH")]
        );
    }

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
    fn a_movie_carrying_no_extends_box_is_not_fragmented_at_all() {
        assert_eq!(
            SampleReader::new(&movie(&[1], vec![])).unwrap_err(),
            SampleError::missing_movie_extends()
        );
    }

    #[test]
    fn an_extends_box_for_a_track_the_movie_never_declared_is_passed_over() {
        let with_a_spare = movie(
            &[1],
            vec![
                TrackExtendsBox::new(1, 1, 1_024, 4, 0),
                TrackExtendsBox::new(7, 1, 1_024, 4, 0),
            ],
        );

        let mut reader = SampleReader::new(&with_a_spare).unwrap();
        let of_the_spare = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                7,
                None,
                None,
                None,
            ),
            None,
            vec![run(Some(100), 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(movie_fragment(vec![of_the_spare]), 0..MOVIE_FRAGMENT_LEN),
            Err(SampleError::unknown_track_id(7))
        );
    }

    #[test]
    fn two_tracks_declaring_one_id_leave_the_first_of_them_standing() {
        let mut reader = SampleReader::new(&movie_of_one_track_id_twice()).unwrap();
        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn a_fragment_of_a_track_the_movie_never_declared_is_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let of_an_unknown_track = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                3,
                None,
                None,
                None,
            ),
            None,
            vec![run(Some(100), 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(
                movie_fragment(vec![of_an_unknown_track]),
                0..MOVIE_FRAGMENT_LEN
            ),
            Err(SampleError::unknown_track_id(3))
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
    fn an_empty_duration_moves_the_timeline_on_without_a_sample() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let empty = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DURATION_IS_EMPTY,
                1,
                None,
                Some(4_096),
                None,
            ),
            None,
            vec![],
        )
        .unwrap();

        reader
            .handle_movie_fragment(movie_fragment(vec![empty]), 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        assert_eq!(reader.poll_sample(), None);

        reader
            .handle_movie_fragment(one_sample_movie_fragment(), 100..200)
            .unwrap();
        reader.handle_media_data(b"ABCD", 200..204).unwrap();

        assert_eq!(drained(&mut reader), [sample(4_096, b"ABCD")]);
    }

    #[test]
    fn the_sequence_number_a_fragment_carries_is_neither_checked_nor_reported() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let out_of_order = MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(9),
            vec![track_fragment(vec![run(Some(100), 1)])],
        );

        reader
            .handle_movie_fragment(out_of_order, 0..MOVIE_FRAGMENT_LEN)
            .unwrap();
        reader.handle_media_data(b"ABCD", 100..104).unwrap();

        assert_eq!(drained(&mut reader), [sample(0, b"ABCD")]);
    }

    #[test]
    fn decode_times_running_past_what_64_bits_carry_are_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let at_the_end_of_time = TrackFragmentBox::new(
            track_fragment_header(
                TrackFragmentHeaderBox::DEFAULT_BASE_IS_MOOF,
                1,
                None,
                Some(u32::MAX),
                None,
            ),
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(u64::MAX)),
            vec![run(Some(100), 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(
                movie_fragment(vec![at_the_end_of_time]),
                0..MOVIE_FRAGMENT_LEN
            ),
            Err(SampleError::decode_time_overflow(1))
        );
    }

    #[test]
    fn data_offsets_running_past_what_64_bits_carry_are_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        let past_the_end_of_the_file = TrackFragmentBox::new(
            track_fragment_header(FullBoxFlags::ZERO, 1, Some(u64::MAX), None, None),
            None,
            vec![run(None, 1)],
        )
        .unwrap();

        assert_eq!(
            reader.handle_movie_fragment(
                movie_fragment(vec![past_the_end_of_the_file]),
                0..MOVIE_FRAGMENT_LEN
            ),
            Err(SampleError::data_offset_overflow(1))
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
    fn anything_handed_over_after_the_samples_were_declared_over_is_refused() {
        let mut reader = SampleReader::new(&one_track_movie()).unwrap();
        reader.finish().unwrap();

        assert_eq!(
            reader.handle_media_data(b"ABCD", 100..104),
            Err(SampleError::already_finished())
        );
        assert_eq!(reader.finish(), Err(SampleError::already_finished()));
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
