//! [`StructureDataReader`], which resolves the samples a box declares to where their bytes lie

use alloc::vec::Vec;

use isobmff_boxes::{MovieBox, MovieFragmentBox};

use super::leniency::Leniencies;
use super::movie_fragment::{self, FragmentTrack};
use super::{Error, Leniency, sample_table};
use crate::SampleLocation;

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
/// their track, the `moov`'s and those of the fragments before. It does not
/// read the bytes of the file: reading the boxes out of them, and the samples
/// where their locations say, is up to the caller.
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
    /// What the boxes taken were read past, the most recent of them
    leniencies: Leniencies,
}

impl StructureDataReader {
    /// Creates a reader that resolves at most `sample_count_limit` samples from one box, and holds the most recent `leniency_limit` of what it reads past
    ///
    /// The number of samples is read from the input, and a box of a few bytes
    /// can declare millions, so the reader holds the declared number against
    /// this limit before it allocates room for them. A file can disagree with
    /// itself in each of its boxes, so the reader holds no more than
    /// `leniency_limit` of the [`Leniency`]s it reads past.
    #[must_use]
    pub fn new(sample_count_limit: u64, leniency_limit: u64) -> Self {
        Self {
            sample_count_limit,
            locations: Vec::new(),
            tracks: None,
            leniencies: Leniencies::new(leniency_limit),
        }
    }

    /// Returns what the reader read past of the boxes it took, the oldest first
    ///
    /// A box the reader refuses adds none. Past the `leniency_limit` the
    /// reader was created with, the oldest is dropped for each new one.
    ///
    /// ```
    /// use isobmff::structure_data::StructureDataReader;
    ///
    /// let reader = StructureDataReader::new(1024, 16);
    /// assert_eq!(reader.leniencies().len(), 0);
    /// ```
    pub fn leniencies(&self) -> impl ExactSizeIterator<Item = &Leniency> + '_ {
        self.leniencies.iter()
    }

    /// Returns the location of each sample the tracks of `movie` declare, in the order of their file offsets
    ///
    /// Samples at the same offset keep the order of their tracks in `movie`,
    /// and within a track the decoding order. A table that counts another
    /// number of samples than the sample size table, and an entry of the sync
    /// sample table that names no sample, are read past as FFmpeg and GPAC
    /// read them, and held for [`leniencies`](Self::leniencies); a box refused
    /// adds none.
    ///
    /// # Errors
    /// * [`Error::SampleCountOverLimit`]: the tracks declare more samples than
    ///   the limit, checked before anything is allocated for them.
    /// * [`Error::MissingTable`], [`Error::NoSamplePlaced`],
    ///   [`Error::InvalidEntry`]: the sample table of a track lacks a table it
    ///   needs, places no sample, or has a sample-to-chunk entry out of order.
    /// * [`Error::MediaDataElsewhere`]: the samples of a track lie in another
    ///   file, or in an `IdentifiedMediaDataBox`.
    /// * [`Error::Overflow`]: a sample lies past the last file offset or
    ///   decoding time.
    pub fn handle_movie(
        &mut self,
        movie: &MovieBox,
    ) -> Result<impl ExactSizeIterator<Item = SampleLocation> + '_, Error> {
        let sample_count = movie
            .trak
            .iter()
            .map(sample_table::sample_count)
            .try_fold(0_u64, |total, count| {
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
        let mut leniencies = self.leniencies.emptied();
        for track in &movie.trak {
            let next_decoding_time =
                sample_table::resolve(track, &mut self.locations, &mut leniencies)?;
            tracks.push(FragmentTrack::new(movie, track, next_decoding_time));
        }
        self.tracks = Some(tracks);
        self.leniencies.append(leniencies);
        self.locations.sort_by_key(|location| location.offset);
        Ok(self.locations.drain(..))
    }

    /// Returns the location of each sample the track fragments of `moof`, the `moof` at file offset `moof_offset`, declare, in the order of their file offsets
    ///
    /// A track fragment counts its data from the base the `tfhd` gives, or
    /// from the first byte of the `moof` or the end of the data of the track
    /// fragment before it, as §8.8.7.1 Table 6 sets out. Its decoding times
    /// start at the `tfdt` where there is one, and otherwise go on from the
    /// samples before it in its track, §8.8.12. Each value a sample does not
    /// carry is taken from the `tfhd`, then from the `trex` of its track.
    ///
    /// # Errors
    /// * [`Error::MovieFragmentWithoutMovie`]: no `moov` was taken; nothing
    ///   says what the tracks of the fragment are.
    /// * [`Error::SampleCountOverLimit`]: the track runs declare more samples
    ///   than the limit, checked before anything is allocated for them.
    /// * [`Error::UnknownTrack`]: a track fragment is of a track the `moov`
    ///   does not have.
    /// * [`Error::MissingDefault`]: a sample takes a value neither the `tfhd`
    ///   nor a `trex` gives.
    /// * [`Error::MediaDataElsewhere`]: a sample lies in another file, or in
    ///   an `IdentifiedMediaDataBox`.
    /// * [`Error::SampleDescriptionIndexOutOfRange`]: a sample names a sample
    ///   entry its track does not have.
    /// * [`Error::Overflow`]: a sample lies past the last file offset or
    ///   decoding time.
    ///
    /// A `tfdt` that gives another decoding time than the samples before it
    /// in its track add up to is taken, and held for
    /// [`leniencies`](Self::leniencies) as
    /// [`Leniency::DecodingTimeMismatch`]; the samples already returned are
    /// left as they were. A box refused leaves the decoding times of the
    /// tracks as they were, and adds no leniency.
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
        let tracks = self
            .tracks
            .as_mut()
            .ok_or(Error::MovieFragmentWithoutMovie)?;
        let sample_count = movie_fragment::movie_fragment_sample_count(moof);
        if sample_count > self.sample_count_limit {
            return Err(Error::SampleCountOverLimit {
                sample_count,
                limit: self.sample_count_limit,
            });
        }
        self.locations.clear();
        self.locations
            .reserve(usize::try_from(sample_count).unwrap_or(usize::MAX));
        let mut leniencies = self.leniencies.emptied();
        movie_fragment::resolve(
            moof,
            moof_offset,
            tracks,
            &mut self.locations,
            &mut leniencies,
        )?;
        self.leniencies.append(leniencies);
        self.locations.sort_by_key(|location| location.offset);
        Ok(self.locations.drain(..))
    }
}
