//! [`StructureDataReader`], which resolves the samples a box declares to where their bytes lie

use alloc::vec::Vec;

use isobmff_boxes::{MovieBox, MovieFragmentBox};

use super::identified_media_data::IdentifiedMediaData;
use super::movie_fragment::{self, FragmentTrack};
use super::{Error, sample_table};
use crate::SampleLocation;

/// How a reader reads tables that disagree with each other
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reading {
    /// As the specification reads them: the box is refused
    Specification,
    /// Losing what the specification gives no value to: the samples
    /// placed are those every table needed gives a value to
    Lossy,
}

/// Reader of the samples the structure-data of a file declares, from the boxes read into their values
///
/// The sample tables of the tracks of a movie, ISO/IEC 14496-12 §8.5.
///
/// [`handle_movie`](Self::handle_movie) takes a `moov` and returns the
/// [`SampleLocation`] of each sample of its tracks, in the order of their file
/// offsets. It refuses a sample table whose tables count different numbers of
/// samples, or a track whose samples lie in another file.
/// [`handle_movie_fragment`](Self::handle_movie_fragment) then takes each
/// `moof` that follows and returns the samples it adds, ISO/IEC 14496-12
/// §8.8: their decoding times go on from those of the samples before them in
/// their track, the `moov`'s and those of the fragments before. A sample whose
/// offset counts from the payload of an `imda`, ISO/IEC 14496-12 §8.7.2.1,
/// is returned at its file offset once
/// [`handle_identified_media_data`](Self::handle_identified_media_data) has
/// taken where that `imda` lies, from the call that resolves it or from that
/// one, whichever comes last; [`finish`](Self::finish) refuses the samples
/// whose `imda` never came. It does not read the bytes of the file: reading
/// the boxes out of them, and the samples where their locations say, is up
/// to the caller.
///
/// ```
/// use isobmff::framing::{BoxReader, Event, FramerConfig};
/// use isobmff::structure_data::StructureDataReader;
/// use isobmff_boxes::MovieBox;
/// use isobmff_core::{BoxDecode, BoxType};
///
/// let file: &[u8] = include_bytes!("../../tests/fixtures/movie_moov_first.mp4");
///
/// // Read the `moov` out of the file
/// let mut boxes = BoxReader::new(FramerConfig::new(1024 * 1024));
/// let mut movie = None;
/// loop {
///     while let Some(event) = boxes.poll_event() {
///         if let Event::Gathered { box_type, payload, .. } = event
///             && box_type == BoxType::compact(*b"moov")
///         {
///             movie = Some(MovieBox::decode_payload(payload)?);
///         }
///     }
///     let Some(required) = boxes.required_input() else { break };
///     match file.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => boxes.handle_input(required.position, bytes),
///         _ => boxes.finish(),
///     }
/// }
///
/// // Resolve its samples to where their bytes lie
/// let mut structure_data = StructureDataReader::new(1024, 16);
/// let locations: Vec<_> = structure_data.handle_movie(&movie.ok_or("no moov")?)?.collect();
/// assert_eq!(locations.len(), 12 + 58);
/// assert!(locations.windows(2).all(|pair| pair[0].offset <= pair[1].offset));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct StructureDataReader {
    sample_count_limit: u64,
    // Why keep the locations between boxes rather than collect them anew: the
    // buffer is reused, so resolving the next box allocates only past the
    // most samples a box held before.
    locations: Vec<SampleLocation>,
    /// What the tracks of the `moov` give the samples of their fragments,
    /// once a `moov` was taken
    tracks: Option<Vec<FragmentTrack>>,
    identified: IdentifiedMediaData,
    // Why hold that the input ended rather than drop what the reader holds:
    // a box taken after it would resolve against `imda` it can no longer
    // tell from those the file lacks.
    finished: bool,
}

impl StructureDataReader {
    /// Creates a reader that resolves at most `sample_count_limit` samples from one box, and remembers where at most `identified_media_data_limit` `imda` lie
    ///
    /// The number of samples is read from the input, and a box of a few bytes
    /// can declare millions, so the reader holds the declared number, with
    /// that of the samples that wait for an `imda`, against
    /// `sample_count_limit` before it allocates room for them. The number of
    /// `imda` grows with the input, each remembered until the input ends.
    #[must_use]
    pub const fn new(sample_count_limit: u64, identified_media_data_limit: u64) -> Self {
        Self {
            sample_count_limit,
            locations: Vec::new(),
            tracks: None,
            identified: IdentifiedMediaData::new(identified_media_data_limit),
            finished: false,
        }
    }

    /// Returns the location of each sample the tracks of `movie` declare, in the order of their file offsets
    ///
    /// Samples at the same offset keep the order of their tracks in `movie`,
    /// and within a track the decoding order.
    ///
    /// # Errors
    /// * [`Error::SampleCountOverLimit`]: the tracks declare more samples than
    ///   the limit, with those that wait for an `imda`, checked before
    ///   anything is allocated for them.
    /// * [`Error::MissingTable`], [`Error::SampleCountMismatch`],
    ///   [`Error::InvalidEntry`]: the sample table of a track lacks a table it
    ///   needs, or its tables disagree, such as a table that counts another
    ///   number of samples than the sample size table, §8.7.3, or a sync
    ///   sample table not in strictly increasing order of sample number,
    ///   §8.6.2.
    /// * [`Error::MediaDataElsewhere`]: the samples of a track lie in another
    ///   file.
    /// * [`Error::SequenceNumberOutsideMovieFragment`]: the samples of a track
    ///   lie in the `imda` the sequence number of a movie fragment names.
    /// * [`Error::Overflow`]: a sample lies past the last file offset or
    ///   decoding time.
    ///
    /// # Panics
    /// Panics if called after [`finish`](Self::finish): a bug of the caller.
    pub fn handle_movie(
        &mut self,
        movie: &MovieBox,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        self.resolve_movie(movie, Reading::Specification)
    }

    /// Returns the location of each sample the tracks of `movie` declare that every table it needs gives a value to, in the order of their file offsets
    ///
    /// Where the tables of a sample table disagree, which §8.7.3.2 recommends
    /// against, the specification gives no reading of the samples past the
    /// shorter table, and [`handle_movie`](Self::handle_movie) refuses it.
    /// This reads it losing what the specification gives no value to, and
    /// resolves the samples as `handle_movie` does where the tables agree:
    ///
    /// 1. An entry that names no sample is not read, as §4.2.2 has a reader
    ///    ignore what it does not recognize and go on: what a
    ///    time-to-sample, composition offset, sample dependency or
    ///    sample-to-chunk table declares past the samples of the sample size
    ///    table, and a sync sample number, §8.6.2.1, past the last sample or
    ///    not after the one before it.
    /// 2. A sample past the end of an optional table takes the value it has
    ///    where the table is absent: a composition offset of 0, §8.6.1.3, and
    ///    unknown dependencies, §8.6.4. This is the rule §8.9.2.3 sets for a
    ///    sample-to-group table shorter than the samples.
    /// 3. A sample the specification gives no size, §8.7.3, chunk, §8.7.4, or
    ///    duration, §8.6.1.2, is not placed: the samples are as many as the
    ///    fewest the sample size table, the chunks and the time-to-sample
    ///    table give a value to.
    ///
    /// # Errors
    /// As [`handle_movie`](Self::handle_movie), but for the disagreements
    /// above; [`Error::SampleCountMismatch`] where no sample can be placed:
    /// the chunks place none, or the time-to-sample table gives no duration,
    /// though the sample size table counts some.
    ///
    /// # Panics
    /// Panics if called after [`finish`](Self::finish): a bug of the caller.
    ///
    /// ```
    /// use isobmff::structure_data::{Error, StructureDataReader};
    /// use isobmff_boxes::MovieBox;
    ///
    /// // A movie whose tables agree reads the same either way
    /// let movie = MovieBox::default();
    /// let mut reader = StructureDataReader::new(1024, 16);
    /// assert_eq!(reader.handle_movie_lossy(&movie)?.len(), 0);
    /// # Ok::<(), Error>(())
    /// ```
    pub fn handle_movie_lossy(
        &mut self,
        movie: &MovieBox,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        self.resolve_movie(movie, Reading::Lossy)
    }

    #[expect(
        clippy::panic_in_result_fn,
        reason = "a box handed over after finish is a bug of the caller, not a box the reader refuses"
    )]
    fn resolve_movie(
        &mut self,
        movie: &MovieBox,
        reading: Reading,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        assert!(!self.finished, "a moov handed over after finish");
        let sample_count = movie
            .trak
            .iter()
            .map(|track| sample_table::sample_count(&track.mdia.minf.stbl, track.tkhd.track_id))
            .try_fold(self.identified.waiting_count(), |total, count| {
                count.map(|count| total.saturating_add(count))
            })?;
        if sample_count > self.sample_count_limit {
            return Err(Error::SampleCountOverLimit {
                sample_count,
                limit: self.sample_count_limit,
            });
        }
        self.locations.clear();
        self.locations
            .reserve(usize::try_from(sample_count).unwrap_or(usize::MAX));
        let mut tracks = Vec::with_capacity(movie.trak.len());
        let identified = &mut self.identified;
        let resolved = movie.trak.iter().try_for_each(|track| {
            let track_id = track.tkhd.track_id;
            let (stbl, dref) = (&track.mdia.minf.stbl, &track.mdia.minf.dinf.dref);
            let next_decoding_time = sample_table::resolve(
                stbl,
                track_id,
                dref,
                &mut self.locations,
                identified,
                reading,
            )?;
            let defaults = movie.mvex.as_ref().and_then(|mvex| {
                mvex.trex
                    .iter()
                    .find(|trex| trex.track_id == track_id)
                    .cloned()
            });
            tracks.push(FragmentTrack::new(
                track_id,
                defaults,
                &stbl.stsd,
                dref,
                next_decoding_time,
            ));
            Ok(())
        });
        self.identified.end_box(resolved)?;
        self.tracks = Some(tracks);
        self.locations.sort_by_key(|location| location.offset);
        Ok(self.locations.drain(..))
    }

    /// Returns the location of each sample the track fragments of `moof`, the `moof` at file offset `moof_offset`, declare, in the order of their file offsets
    ///
    /// A track fragment counts its data from the base the `tfhd` gives, or
    /// from the first byte of the `moof` or the end of the data of the track
    /// fragment before it, as §8.8.7.1 Table 6 sets out. Its decoding times
    /// start at the `tfdt` where there is one, and otherwise go on from the
    /// samples before it in its track, §8.8.12. A `tfdt` past the time those
    /// samples add up to extends the last of them, §8.8.12.1: its location,
    /// returned before, keeps the `sample_duration` it declares, and the
    /// extended duration is the decoding time of the next sample less its
    /// own. Each value a sample does not carry is taken from the `tfhd`, then
    /// from the `trex` of its track.
    ///
    /// # Errors
    /// * [`Error::MovieFragmentWithoutMovie`]: no `moov` was taken; nothing
    ///   says what the tracks of the fragment are.
    /// * [`Error::SampleCountOverLimit`]: the track runs declare more samples
    ///   than the limit, with those that wait for an `imda`, checked before
    ///   anything is allocated for them.
    /// * [`Error::UnknownTrack`]: a track fragment is of a track the `moov`
    ///   does not have.
    /// * [`Error::MissingDefault`]: a sample takes a value neither the `tfhd`
    ///   nor a `trex` gives.
    /// * [`Error::MediaDataElsewhere`]: a sample lies in another file.
    /// * [`Error::SampleDescriptionIndexOutOfRange`]: a sample names a sample
    ///   entry its track does not have.
    /// * [`Error::DecodingTimeMismatch`]: the `tfdt` of a track fragment is
    ///   less than the durations of the samples before it in its track add
    ///   up to, §8.8.12.
    /// * [`Error::Overflow`]: a sample lies past the last file offset or
    ///   decoding time.
    ///
    /// A box refused leaves the decoding times of the tracks as they were.
    ///
    /// A track fragment whose sample entry names an `imda` counts its data
    /// from the payload of that `imda` where Table 6 says so: with no
    /// `default-base-is-moof`, from the base the `tfhd` gives or, for the
    /// first track fragment, from 0. With `default-base-is-moof` it counts
    /// from the first byte of the `moof`, as any other does, and its samples
    /// are still returned only once that `imda` is read.
    ///
    /// # Panics
    /// Panics if called after [`finish`](Self::finish): a bug of the caller.
    ///
    /// ```
    /// use isobmff::framing::{BoxReader, Event, FramerConfig};
    /// use isobmff::structure_data::StructureDataReader;
    /// use isobmff_boxes::{MovieBox, MovieFragmentBox};
    /// use isobmff_core::{BoxDecode, BoxDefinition};
    ///
    /// // Resolve the `moov` and each `moof` of a fragmented file as they come
    /// let file: &[u8] = include_bytes!("../../tests/fixtures/fragmented.mp4");
    /// let mut boxes = BoxReader::new(FramerConfig::new(1024 * 1024));
    /// let mut structure_data = StructureDataReader::new(1024, 16);
    /// let mut locations = Vec::new();
    /// loop {
    ///     while let Some(event) = boxes.poll_event() {
    ///         if let Event::Gathered { offset, box_type, payload, .. } = event {
    ///             if box_type == MovieBox::BOX_TYPE {
    ///                 let movie = MovieBox::decode_payload(payload)?;
    ///                 locations.extend(structure_data.handle_movie(&movie)?);
    ///             } else if box_type == MovieFragmentBox::BOX_TYPE {
    ///                 let moof = MovieFragmentBox::decode_payload(payload)?;
    ///                 locations.extend(structure_data.handle_movie_fragment(&moof, offset)?);
    ///             }
    ///         }
    ///     }
    ///     let Some(required) = boxes.required_input() else { break };
    ///     match file.get(usize::try_from(required.position)?..) {
    ///         Some(bytes) if !bytes.is_empty() => boxes.handle_input(required.position, bytes),
    ///         _ => boxes.finish(),
    ///     }
    /// }
    ///
    /// // The samples of track 1 go on in decoding time across the fragments
    /// let track_1: Vec<_> = locations.iter().filter(|location| location.properties.track_id == 1).collect();
    /// assert_eq!(locations.len(), 12 + 58);
    /// assert!(track_1.windows(2).all(|pair| {
    ///     pair[1].properties.decoding_time
    ///         == pair[0].properties.decoding_time + u64::from(pair[0].properties.sample_duration)
    /// }));
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    pub fn handle_movie_fragment(
        &mut self,
        moof: &MovieFragmentBox,
        moof_offset: u64,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        self.resolve_movie_fragment(moof, moof_offset, Reading::Specification)
    }

    /// Returns the location of each sample the track fragments of `moof`, the `moof` at file offset `moof_offset`, declare, in the order of their file offsets, taking a `tfdt` that goes back
    ///
    /// Where [`handle_movie_fragment`](Self::handle_movie_fragment) refuses a
    /// track fragment whose `tfdt` is less than the durations of the samples
    /// before it in its track add up to, §8.8.12.1, the `tfdt` is taken, and
    /// the samples are resolved the same way otherwise. The samples already
    /// returned are left as they were.
    ///
    /// # Errors
    /// As [`handle_movie_fragment`](Self::handle_movie_fragment), but for
    /// [`Error::DecodingTimeMismatch`].
    ///
    /// # Panics
    /// Panics if called after [`finish`](Self::finish): a bug of the caller.
    ///
    /// ```
    /// use isobmff::structure_data::{Error, StructureDataReader};
    /// use isobmff_boxes::{MovieBox, MovieFragmentBox};
    ///
    /// // A fragment needs the `moov` of its tracks either way
    /// let mut reader = StructureDataReader::new(1024, 16);
    /// assert_eq!(
    ///     reader.handle_movie_fragment_lossy(&MovieFragmentBox::default(), 0).map(|_| ()),
    ///     Err(Error::MovieFragmentWithoutMovie)
    /// );
    /// let _ = reader.handle_movie(&MovieBox::default())?.count();
    /// assert_eq!(reader.handle_movie_fragment_lossy(&MovieFragmentBox::default(), 0)?.len(), 0);
    /// # Ok::<(), Error>(())
    /// ```
    pub fn handle_movie_fragment_lossy(
        &mut self,
        moof: &MovieFragmentBox,
        moof_offset: u64,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        self.resolve_movie_fragment(moof, moof_offset, Reading::Lossy)
    }

    #[expect(
        clippy::panic_in_result_fn,
        reason = "a box handed over after finish is a bug of the caller, not a box the reader refuses"
    )]
    fn resolve_movie_fragment(
        &mut self,
        moof: &MovieFragmentBox,
        moof_offset: u64,
        reading: Reading,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        assert!(!self.finished, "a moof handed over after finish");
        let tracks = self
            .tracks
            .as_mut()
            .ok_or(Error::MovieFragmentWithoutMovie)?;
        let sample_count = movie_fragment::movie_fragment_sample_count(moof)
            .saturating_add(self.identified.waiting_count());
        if sample_count > self.sample_count_limit {
            return Err(Error::SampleCountOverLimit {
                sample_count,
                limit: self.sample_count_limit,
            });
        }
        self.locations.clear();
        self.locations
            .reserve(usize::try_from(sample_count).unwrap_or(usize::MAX));
        let resolved = movie_fragment::resolve(
            moof,
            moof_offset,
            tracks,
            &mut self.locations,
            &mut self.identified,
            reading,
        );
        self.identified.end_box(resolved)?;
        self.locations.sort_by_key(|location| location.offset);
        Ok(self.locations.drain(..))
    }

    /// Takes where the payload of an `imda` lies, and returns the location of each sample that waits for it, in the order of their file offsets
    ///
    /// The [`IdentifiedMediaDataBox`](isobmff_boxes::IdentifiedMediaDataBox)
    /// (`imda`), ISO/IEC 14496-12 §8.1.4, of `imda_identifier`, whose payload
    /// starts at the file offset `payload_offset`: the first byte of the
    /// identifier, from which a sample that lies in it counts its offset,
    /// §8.7.2.1. An `imda` may come before the box that declares its samples
    /// or after it; the reader remembers where it lies until the input ends.
    ///
    /// # Errors
    /// * [`Error::DuplicateImdaIdentifier`]: an `imda` of the same identifier
    ///   was taken, which §8.1.4.3 forbids.
    /// * [`Error::IdentifiedMediaDataOverLimit`]: as many `imda` as the limit
    ///   were taken.
    /// * [`Error::Overflow`]: a sample that waits for it lies past the last
    ///   file offset.
    ///
    /// Refused, it remembers nothing, and the samples go on waiting.
    ///
    /// # Panics
    /// Panics if called after [`finish`](Self::finish): a bug of the caller.
    ///
    /// ```
    /// use isobmff::structure_data::{Error, StructureDataReader};
    ///
    /// // An identifier names one `imda` of the file
    /// let mut reader = StructureDataReader::new(1024, 16);
    /// assert_eq!(reader.handle_identified_media_data(7, 1000)?.len(), 0);
    /// assert_eq!(
    ///     reader.handle_identified_media_data(7, 2000).map(|_| ()),
    ///     Err(Error::DuplicateImdaIdentifier { imda_identifier: 7 })
    /// );
    /// # Ok::<(), Error>(())
    /// ```
    #[expect(
        clippy::panic_in_result_fn,
        reason = "a box handed over after finish is a bug of the caller, not a box the reader refuses"
    )]
    pub fn handle_identified_media_data(
        &mut self,
        imda_identifier: u32,
        payload_offset: u64,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        assert!(!self.finished, "an imda handed over after finish");
        self.locations.clear();
        self.identified
            .handle(imda_identifier, payload_offset, &mut self.locations)?;
        self.locations.sort_by_key(|location| location.offset);
        Ok(self.locations.drain(..))
    }

    /// Ends the input, refusing the samples that wait for an `imda` it never held
    ///
    /// A data reference to an `imda` the file does not hold leaves the
    /// samples of its track unplaced, ISO/IEC 14496-12 §8.7.2.1, E.15.2. The
    /// samples that wait for any `imda` are dropped; called again, it refuses
    /// nothing.
    ///
    /// # Errors
    /// * [`Error::IdentifiedMediaDataNotFound`]: samples wait for an `imda`;
    ///   the error names the least identifier among those they wait for,
    ///   and the samples that wait for it, though the samples that wait for
    ///   another are dropped too.
    ///
    /// ```
    /// use isobmff::structure_data::StructureDataReader;
    ///
    /// // A file that holds no `imda` and no sample of one ends without error
    /// let mut reader = StructureDataReader::new(1024, 16);
    /// assert_eq!(reader.finish(), Ok(()));
    /// ```
    pub fn finish(&mut self) -> Result<(), Error> {
        self.finished = true;
        self.identified.finish()
    }
}
