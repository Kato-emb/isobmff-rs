//! [`StructureDataReader`], which reads the structure-data of a file into the samples it declares

use alloc::collections::BTreeMap;
use alloc::vec::{self, Vec};
use core::iter::FusedIterator;
use core::ops::Range;

use isobmff_boxes::{MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox};
use isobmff_core::BoxDefinition;

use super::movie_fragment::ReachedTime;
use super::sample_description::DataReference;
use super::segment_index::SegmentIndex;
use super::{movie_fragment, sample_description, sample_table};
use crate::error::DemuxError;
use crate::framing::to_u64;
use crate::leniency::Leniency;
use crate::sample::{DataReferenceKey, SampleLocation};

/// Limits a [`StructureDataReader`] resolves and keeps structure-data up to
///
/// Each bounds what the reader allocates for values the file declares: the
/// samples of one `moov` or `moof`, and the segment indexes, with their
/// subsegments, the places of `IdentifiedMediaDataBox`es and the samples
/// whose `IdentifiedMediaDataBox` is not read yet, that the reader keeps over
/// the whole file. Each takes the same room whatever the file, so its count
/// bounds its bytes.
///
/// # Examples
///
/// ```
/// use isobmff::structure_data::StructureDataLimits;
///
/// let limits = StructureDataLimits {
///     sample_count: 1_048_576,
///     segment_index_count: 1024,
///     subsegment_count: 1_048_576,
///     leniency_count: 65_536,
///     identified_media_data_count: 65_536,
///     unresolved_sample_count: 1_048_576,
/// };
/// assert_eq!(limits.segment_index_count, 1024);
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "a limit added later is one every caller has to set, not one that quietly takes a default"
)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StructureDataLimits {
    /// Number of samples one `moov`, or one `moof`, is resolved up to
    pub sample_count: u64,
    /// Number of segment indexes the reader keeps up to
    pub segment_index_count: u64,
    /// Number of subsegments the segment indexes the reader keeps hold up to, together
    pub subsegment_count: u64,
    /// Number of leniencies the reader keeps up to, over the whole file
    pub leniency_count: u64,
    /// Number of `IdentifiedMediaDataBox`es the reader keeps the places of up to, over the whole file
    pub identified_media_data_count: u64,
    /// Number of samples whose `IdentifiedMediaDataBox` is not read yet the reader holds up to, together
    pub unresolved_sample_count: u64,
}

/// Reader of the structure-data of a file: the movie and its fragments, resolved to the samples they declare
///
/// The reader keeps what structure-data declares: the movie, whose tracks the
/// fragments extend, the decoding time each track has reached (§8.8.12), the
/// segment indexes, and the movie fragment random access box.
///
/// The data offsets of samples whose data reference entry names an
/// `IdentifiedMediaDataBox` count from the first byte of its payload, its
/// `imda_identifier` (§8.7.2, Table 1 of §6.1): the media data starts at
/// offset 4. The reader keeps where the payload of each box it is told of
/// with [`handle_identified_media_data`](Self::handle_identified_media_data)
/// starts, and locates such samples in the file. It holds those of a box it
/// has not been told of, and returns them once it is.
///
/// A refused call leaves the reader as it was.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::MovieBox;
/// use isobmff::structure_data::{StructureDataLimits, StructureDataReader};
///
/// let mut reader = StructureDataReader::new(StructureDataLimits {
///     sample_count: 1_048_576,
///     segment_index_count: 1024,
///     subsegment_count: 1_048_576,
///     leniency_count: 65_536,
///     identified_media_data_count: 65_536,
///     unresolved_sample_count: 1_048_576,
/// });
/// // A movie of no track declares no sample.
/// let samples = reader.handle_movie(MovieBox::default())?;
/// assert_eq!(samples.len(), 0);
/// assert!(reader.movie().is_some());
/// # Ok::<(), isobmff::DemuxError>(())
/// ```
#[derive(Debug)]
pub struct StructureDataReader {
    limits: StructureDataLimits,
    movie: Option<MovieBox>,
    /// Decoding time past the last sample of each track of the movie, in the order of its tracks, `None` once it is not known
    decoding_times: Vec<Option<ReachedTime>>,
    segment_indexes: Vec<SegmentIndex>,
    movie_fragment_random_access: Option<MovieFragmentRandomAccessBox>,
    leniencies: Vec<Leniency>,
    /// File offset of the first byte of the payload of each `IdentifiedMediaDataBox` read, by its `imda_identifier`
    identified_media_data: BTreeMap<u32, u64>,
    /// Samples whose `IdentifiedMediaDataBox` is not read yet, by its `imda_identifier`, each located from the first byte of the box's payload
    unresolved: Held,
}

impl StructureDataReader {
    /// Creates the reader of a file whose structure-data it resolves and keeps up to `limits`
    #[must_use]
    pub const fn new(limits: StructureDataLimits) -> Self {
        Self {
            limits,
            movie: None,
            decoding_times: Vec::new(),
            segment_indexes: Vec::new(),
            movie_fragment_random_access: None,
            leniencies: Vec::new(),
            identified_media_data: BTreeMap::new(),
            unresolved: BTreeMap::new(),
        }
    }

    /// Resolves the samples the sample tables of `movie` declare, and keeps the movie
    ///
    /// The samples of every track come out in the order their media data lies
    /// in the file, those of one chunk in sample order. The decoding time of
    /// each track starts at zero and runs on into the fragments that follow
    /// (§8.6.1.2, §8.8.12).
    ///
    /// A table that states another number of samples than its track has is
    /// read past, as FFmpeg does, and kept as a [`Leniency::SampleTable`]
    /// that [`leniencies`](Self::leniencies) returns: the track has the
    /// samples its chunks lay out, as far as its `stsz` or `stz2` gives them
    /// sizes; the samples past the end of a short `stts` take its last
    /// duration, those past the end of another short table take zero; and
    /// the entries of a table past the last sample are left out, as are the
    /// sample numbers the `stss` marks out of order.
    ///
    /// # Errors
    ///
    /// * [`DuplicateBox`](DemuxError::DuplicateBox): the reader already holds
    ///   a movie.
    /// * [`SampleCountLimitExceeded`](DemuxError::SampleCountLimitExceeded):
    ///   the sample size tables of every track together count more samples
    ///   than the limit, and none is resolved.
    /// * [`UnresolvedSampleCountLimitExceeded`](DemuxError::UnresolvedSampleCountLimitExceeded):
    ///   holding the samples in an `IdentifiedMediaDataBox` not read yet would
    ///   take those the reader holds past the limit.
    /// * [`LeniencyCountLimitExceeded`](DemuxError::LeniencyCountLimitExceeded):
    ///   keeping what the movie is read past in would take the leniencies the
    ///   reader keeps past the limit.
    /// * [`SampleTableMismatch`](DemuxError::SampleTableMismatch): the tables
    ///   of a track cannot lay out its samples: the track declares samples,
    ///   but its `stsz` or `stz2`, its `stco` or `co64`, or its `stts`
    ///   states none, or its `stsc` lays no sample in a chunk; or its `stsc`
    ///   lays its runs over chunks it does not have.
    /// * [`UnknownSampleDescriptionIndex`](DemuxError::UnknownSampleDescriptionIndex),
    ///   [`UnknownDataReferenceIndex`](DemuxError::UnknownDataReferenceIndex):
    ///   samples are described by a sample entry, or a sample entry names a
    ///   data reference entry, that the track has none of.
    /// * [`UnsupportedDataReference`](DemuxError::UnsupportedDataReference):
    ///   samples of the movie name a `DataEntrySeqNumImdaBox`, which names
    ///   only the media data of samples in a movie fragment (§8.7.2).
    /// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange),
    ///   [`DecodingTimeOverflow`](DemuxError::DecodingTimeOverflow): the media
    ///   data or the decoding times of a track run past 64 bits.
    pub fn handle_movie(&mut self, movie: MovieBox) -> Result<SampleLocations, DemuxError> {
        if self.movie.is_some() {
            return Err(DemuxError::DuplicateBox {
                box_type: MovieBox::BOX_TYPE,
            });
        }
        let declared_samples = movie
            .trak
            .iter()
            .map(sample_table::sample_count)
            .fold(0, u64::saturating_add);
        let mut locations = self.room_for(declared_samples)?;
        let mut decoding_times = Vec::with_capacity(movie.trak.len());
        let mut identified = Vec::new();
        let mut leniencies = Vec::new();
        for (track_position, trak) in movie.trak.iter().enumerate() {
            let earlier_samples = locations.len();
            let time = sample_table::resolve_track(
                trak,
                track_position,
                &mut locations,
                &mut identified,
                &mut leniencies,
            )?;
            decoding_times.push(Some(ReachedTime {
                time,
                after_sample: locations.len() > earlier_samples,
            }));
        }
        self.check_room_for_leniencies(&leniencies)?;
        let (mut locations, unresolved) = self.place(locations, &identified)?;
        in_file_order(&mut locations);

        self.movie = Some(movie);
        self.decoding_times = decoding_times;
        self.leniencies = leniencies;
        for (identifier, mut held) in unresolved {
            self.unresolved
                .entry(identifier)
                .or_default()
                .append(&mut held);
        }
        Ok(SampleLocations {
            locations: locations.into_iter(),
        })
    }

    /// Resolves the samples the track fragments of `movie_fragment`, whose `moof` starts at file offset `moof_offset`, declare
    ///
    /// The samples come out in the order their media data lies in the file.
    /// A track fragment without a `tfdt` takes up the decoding time its track
    /// has reached in the movie and the fragments before it (§8.8.12), and
    /// the fragment moves that time on. The data offsets resolve to file
    /// offsets as Table 6 of §8.8.7.1 sets out.
    ///
    /// A `tfdt` past the decoding time the earlier samples of its track have
    /// reached extends the duration of the sample before it (§8.8.12). That
    /// sample is already returned, so the first sample of the track fragment
    /// takes the decoding time of the `tfdt`, and the reader keeps the gap
    /// as a [`Leniency::DecodingTimeGap`] that
    /// [`leniencies`](Self::leniencies) returns. A `tfdt` before that time,
    /// which §8.8.12 does not allow, is taken as it is and kept as a
    /// [`Leniency::DecodingTimeOverlap`]. A track that has no earlier sample,
    /// or whose time is not known since the reader resumed, is compared with
    /// nothing.
    ///
    /// # Errors
    ///
    /// * [`MissingMovie`](DemuxError::MissingMovie): the reader holds no
    ///   movie.
    /// * [`SampleCountLimitExceeded`](DemuxError::SampleCountLimitExceeded):
    ///   the track runs together count more samples than the limit, and none
    ///   is resolved.
    /// * [`UnresolvedSampleCountLimitExceeded`](DemuxError::UnresolvedSampleCountLimitExceeded):
    ///   holding the samples in an `IdentifiedMediaDataBox` not read yet would
    ///   take those the reader holds past the limit.
    /// * [`LeniencyCountLimitExceeded`](DemuxError::LeniencyCountLimitExceeded):
    ///   keeping what the fragment is read past in would take the leniencies
    ///   the reader keeps past the limit.
    /// * [`UnknownTrackId`](DemuxError::UnknownTrackId),
    ///   [`MissingTrackExtends`](DemuxError::MissingTrackExtends): a track
    ///   fragment is of a track the movie, or its `mvex`, has none of.
    /// * [`MissingDecodingTime`](DemuxError::MissingDecodingTime): a track
    ///   fragment has no `tfdt`, and the reader has resumed since its track
    ///   last had one.
    /// * The failures of the sample entries and the data references, and of
    ///   offsets and decoding times past 64 bits, as for
    ///   [`handle_movie`](Self::handle_movie).
    pub fn handle_movie_fragment(
        &mut self,
        movie_fragment: &MovieFragmentBox,
        moof_offset: u64,
    ) -> Result<SampleLocations, DemuxError> {
        let movie = self.movie.as_ref().ok_or(DemuxError::MissingMovie)?;
        let mut locations = self.room_for(movie_fragment::sample_count(movie_fragment))?;
        let mut decoding_times = self.decoding_times.clone();
        let mut identified = Vec::new();
        let mut leniencies = Vec::new();
        movie_fragment::resolve_fragment(
            movie,
            &mut decoding_times,
            movie_fragment,
            moof_offset,
            &mut locations,
            &mut identified,
            &mut leniencies,
        )?;
        self.check_room_for_leniencies(&leniencies)?;
        let (mut locations, unresolved) = self.place(locations, &identified)?;
        in_file_order(&mut locations);

        self.decoding_times = decoding_times;
        self.leniencies.append(&mut leniencies);
        for (identifier, mut held) in unresolved {
            self.unresolved
                .entry(identifier)
                .or_default()
                .append(&mut held);
        }
        Ok(SampleLocations {
            locations: locations.into_iter(),
        })
    }

    /// Resolves `sidx`, which lies at the file offsets `byte_range`, its anchor point the first byte after it, and keeps the segment index
    ///
    /// A segment index the reader already keeps, as it does after resuming
    /// before the `sidx`, is kept once, and counts once toward the limits.
    ///
    /// # Errors
    ///
    /// * [`SubsegmentCountLimitExceeded`](DemuxError::SubsegmentCountLimitExceeded):
    ///   the `sidx` references more subsegments than the limit, and none is
    ///   resolved; or keeping its index would take the subsegments of the
    ///   kept indexes past the limit.
    /// * [`SegmentIndexCountLimitExceeded`](DemuxError::SegmentIndexCountLimitExceeded):
    ///   keeping the index would take the kept indexes past the limit.
    /// * [`SegmentIndexOutOfRange`](DemuxError::SegmentIndexOutOfRange): the
    ///   byte ranges or the presentation times run past 64 bits.
    pub fn handle_segment_index(
        &mut self,
        sidx: &SegmentIndexBox,
        byte_range: Range<u64>,
    ) -> Result<(), DemuxError> {
        let subsegments_exceeded =
            |retained_subsegments| DemuxError::SubsegmentCountLimitExceeded {
                retained_subsegments,
                limit_subsegments: self.limits.subsegment_count,
            };
        let declared_subsegments = to_u64(sidx.references.len());
        if declared_subsegments > self.limits.subsegment_count {
            return Err(subsegments_exceeded(declared_subsegments));
        }
        let segment_index = SegmentIndex::new(sidx, byte_range)?;
        if self.segment_indexes.contains(&segment_index) {
            return Ok(());
        }

        let retained_segment_indexes = to_u64(self.segment_indexes.len()).saturating_add(1);
        if retained_segment_indexes > self.limits.segment_index_count {
            return Err(DemuxError::SegmentIndexCountLimitExceeded {
                retained_segment_indexes,
                limit_segment_indexes: self.limits.segment_index_count,
            });
        }
        let retained_subsegments = self
            .segment_indexes
            .iter()
            .map(|kept| to_u64(kept.subsegments().len()))
            .fold(declared_subsegments, u64::saturating_add);
        if retained_subsegments > self.limits.subsegment_count {
            return Err(subsegments_exceeded(retained_subsegments));
        }
        self.segment_indexes.push(segment_index);

        Ok(())
    }

    /// Keeps `mfra`, in place of one the reader kept before
    pub fn handle_movie_fragment_random_access(&mut self, mfra: MovieFragmentRandomAccessBox) {
        self.movie_fragment_random_access = Some(mfra);
    }

    /// Forgets the decoding time each track has reached, as a reader does that resumes reading at a later box of the file
    ///
    /// The fragments read after it resolve only where their `tfdt` gives the
    /// decoding time (§8.8.12). The movie and the entries of its data
    /// references, the segment indexes and the movie fragment random access
    /// box are kept, and so are the leniencies.
    pub fn resume(&mut self) {
        self.decoding_times.fill(None);
        self.unresolved.clear();
    }

    /// Keeps where the payload of the `IdentifiedMediaDataBox` of `identifier` starts, at file offset `payload_offset`, and returns the samples held for it, located in the file
    ///
    /// The payload starts with the `imda_identifier`, and the data offsets of
    /// the samples in the box count from its first byte (§8.7.2): media data
    /// starts at offset 4. The samples come in the order their media data
    /// lies in the file. Samples whose box is read before them are located in
    /// the file as [`handle_movie`](Self::handle_movie) and
    /// [`handle_movie_fragment`](Self::handle_movie_fragment) resolve them.
    ///
    /// # Errors
    ///
    /// * [`DuplicateIdentifiedMediaData`](DemuxError::DuplicateIdentifiedMediaData):
    ///   the reader keeps a box of the same `imda_identifier`, which §8.1.4
    ///   does not allow.
    /// * [`IdentifiedMediaDataCountLimitExceeded`](DemuxError::IdentifiedMediaDataCountLimitExceeded):
    ///   keeping the place of the box would take those the reader keeps past
    ///   the limit.
    /// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange): the
    ///   media data of a sample lies past 64-bit file offsets.
    pub fn handle_identified_media_data(
        &mut self,
        identifier: u32,
        payload_offset: u64,
    ) -> Result<SampleLocations, DemuxError> {
        if self.identified_media_data.contains_key(&identifier) {
            return Err(DemuxError::DuplicateIdentifiedMediaData { identifier });
        }
        let retained_identified_media_data =
            to_u64(self.identified_media_data.len()).saturating_add(1);
        if retained_identified_media_data > self.limits.identified_media_data_count {
            return Err(DemuxError::IdentifiedMediaDataCountLimitExceeded {
                retained_identified_media_data,
                limit_identified_media_data: self.limits.identified_media_data_count,
            });
        }
        let mut locations = self
            .unresolved
            .get(&identifier)
            .into_iter()
            .flatten()
            .map(|location| located(location, payload_offset))
            .collect::<Result<Vec<_>, _>>()?;
        in_file_order(&mut locations);

        self.identified_media_data
            .insert(identifier, payload_offset);
        self.unresolved.remove(&identifier);
        Ok(SampleLocations {
            locations: locations.into_iter(),
        })
    }

    /// Returns the `imda_identifier` of an `IdentifiedMediaDataBox` the reader holds samples for, not read yet, if any
    #[must_use]
    pub fn unresolved_identified_media_data(&self) -> Option<u32> {
        self.unresolved.keys().next().copied()
    }

    /// Returns the movie the reader holds, once one is read
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.movie.as_ref()
    }

    /// Returns the segment indexes the reader keeps, in the order their `sidx`es were read
    #[must_use]
    pub fn segment_indexes(&self) -> &[SegmentIndex] {
        &self.segment_indexes
    }

    /// Returns the movie fragment random access box the reader keeps, the last one read
    #[must_use]
    pub const fn movie_fragment_random_access(&self) -> Option<&MovieFragmentRandomAccessBox> {
        self.movie_fragment_random_access.as_ref()
    }

    /// Returns the entries of the data references of the tracks of the movie, in the order of their keys, once the movie is read, §8.7.2
    ///
    /// The location of each sample carries the key of the entry its sample
    /// entry names. An entry that
    /// [`is_in_this_file`](isobmff_boxes::DataEntry::is_in_this_file) says
    /// the media data is in the file itself, or in an `IdentifiedMediaDataBox`
    /// of it. Any other entry names where else it is: another file, which a
    /// `url ` with a location or a `urn ` names and whose file offsets the
    /// byte ranges of the locations are in.
    pub fn data_references(&self) -> impl Iterator<Item = DataReference<'_>> {
        self.movie
            .iter()
            .flat_map(sample_description::data_references)
    }

    /// Returns the entry of the data reference of key `key`, as [`data_references`](Self::data_references) lists it, or `None` where it lists none
    #[must_use]
    pub fn data_reference(&self, key: DataReferenceKey) -> Option<DataReference<'_>> {
        sample_description::data_reference(self.movie.as_ref()?, key)
    }

    /// Returns what the reader has read past in the structure-data, in the order it read it
    #[must_use]
    pub fn leniencies(&self) -> &[Leniency] {
        &self.leniencies
    }

    /// Refuses `leniencies` where keeping them besides those kept would take the leniencies past the limit
    fn check_room_for_leniencies(&self, leniencies: &[Leniency]) -> Result<(), DemuxError> {
        let retained_leniencies =
            to_u64(self.leniencies.len()).saturating_add(to_u64(leniencies.len()));
        if retained_leniencies > self.limits.leniency_count {
            return Err(DemuxError::LeniencyCountLimitExceeded {
                retained_leniencies,
                limit_leniencies: self.limits.leniency_count,
            });
        }
        Ok(())
    }

    /// Splits `locations` into those located in a file and those held for an `IdentifiedMediaDataBox` not read yet, the locations at the indexes `identified` names counting from the first byte of the payload of the box of their `imda_identifier`
    ///
    /// # Errors
    ///
    /// * [`UnresolvedSampleCountLimitExceeded`](DemuxError::UnresolvedSampleCountLimitExceeded):
    ///   holding the samples would take those the reader holds past the limit.
    /// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange): the media
    ///   data of a sample lies past 64-bit file offsets.
    fn place(
        &self,
        locations: Vec<SampleLocation>,
        identified: &[(usize, u32)],
    ) -> Result<(Vec<SampleLocation>, Held), DemuxError> {
        if identified.is_empty() {
            return Ok((locations, BTreeMap::new()));
        }
        let held_samples = identified
            .iter()
            .filter(|(_, identifier)| !self.identified_media_data.contains_key(identifier))
            .count();
        let held_before: usize = self.unresolved.values().map(Vec::len).sum();
        let unresolved_samples = to_u64(held_before).saturating_add(to_u64(held_samples));
        if unresolved_samples > self.limits.unresolved_sample_count {
            return Err(DemuxError::UnresolvedSampleCountLimitExceeded {
                unresolved_samples,
                limit_samples: self.limits.unresolved_sample_count,
            });
        }
        let mut identified = identified.iter().peekable();
        let mut located_in_file = Vec::with_capacity(locations.len().saturating_sub(held_samples));
        let mut unresolved = Held::new();
        for (index, location) in locations.into_iter().enumerate() {
            let Some(&(_, identifier)) = identified.next_if(|&&(at, _)| at == index) else {
                located_in_file.push(location);
                continue;
            };
            match self.identified_media_data.get(&identifier) {
                Some(&payload_offset) => {
                    located_in_file.push(located(&location, payload_offset)?);
                }
                None => unresolved.entry(identifier).or_default().push(location),
            }
        }
        Ok((located_in_file, unresolved))
    }

    /// Returns room for the locations of `declared_samples` samples, or the failure of more than the limit
    fn room_for(&self, declared_samples: u64) -> Result<Vec<SampleLocation>, DemuxError> {
        if declared_samples > self.limits.sample_count {
            return Err(DemuxError::SampleCountLimitExceeded {
                declared_samples,
                limit_samples: self.limits.sample_count,
            });
        }

        Ok(Vec::with_capacity(
            usize::try_from(declared_samples).unwrap_or_default(),
        ))
    }
}

/// Locations of the samples a `moov` or a `moof` declares, in the order their media data lies in the file
///
/// The iterator owns the locations: it borrows nothing of the reader that
/// resolved them.
#[derive(Clone, Debug)]
pub struct SampleLocations {
    locations: vec::IntoIter<SampleLocation>,
}

impl SampleLocations {
    /// Returns the locations left, by track in decoding order (§8.6.1.2)
    ///
    /// The chunks of a track need not lie in the file in the order of their
    /// decoding times (§8.7.5), so the order of the media data is not the
    /// order a decoder takes the samples in. The locations come by track ID,
    /// then by decoding time; samples of one decoding time come in the order
    /// their media data lies. The decoding times of tracks count in the
    /// timescales of their media, so the order across tracks is the
    /// caller's.
    ///
    /// # Examples
    ///
    /// A caller that can seek reads the samples of the file in decoding
    /// order, each from where its media data lies:
    ///
    /// ```
    /// use isobmff::{DemuxEvent, Demuxer};
    ///
    /// // Chunk 2 of the track lies before chunk 1
    /// let file: &[u8] = include_bytes!("../../tests/data/out_of_order.mp4");
    /// let mut demuxer = Demuxer::new();
    /// let mut locations = Vec::new();
    /// loop {
    ///     let event = match demuxer.wanted_input() {
    ///         Some(wanted) => match file.get(usize::try_from(wanted.offset())?..).unwrap_or_default() {
    ///             [] => demuxer.finish(),
    ///             input => demuxer.handle_input(wanted.offset(), input),
    ///         },
    ///         None => match demuxer.finish() {
    ///             Ok(None) => break,
    ///             finished => finished,
    ///         },
    ///     };
    ///     while demuxer.poll_sample().is_some() {}
    ///     if let Some(DemuxEvent::Movie { samples, .. }) = event? {
    ///         locations = samples.into_decoding_order();
    ///     }
    /// }
    ///
    /// // Seek to the media data of each sample, and read it
    /// let data = locations.iter().map(|location| {
    ///     let range = location.byte_range();
    ///     file.get(usize::try_from(range.start)?..usize::try_from(range.end)?).ok_or("short".into())
    /// }).collect::<Result<Vec<_>, Box<dyn core::error::Error>>>()?;
    /// // Chunk 1 holds the sample of decoding time 0
    /// assert_eq!(data, [b"CD", b"AB"]);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    #[must_use]
    pub fn into_decoding_order(self) -> Vec<SampleLocation> {
        let mut locations: Vec<_> = self.locations.collect();
        // Why not a stable sort: it allocates scratch in proportion to the locations; the start of the media data breaks ties instead.
        locations.sort_unstable_by_key(|location| {
            let properties = location.properties();
            (
                properties.track_id,
                properties.decoding_time,
                location.byte_range().start,
            )
        });
        locations
    }
}

impl SampleLocations {
    /// Returns the locations left, without taking them
    pub(crate) fn as_slice(&self) -> &[SampleLocation] {
        self.locations.as_slice()
    }

    /// Returns the locations left, each replaced by what `replace` returns for it
    ///
    /// # Errors
    ///
    /// The first failure of `replace`.
    pub(crate) fn try_map(
        self,
        replace: impl FnMut(SampleLocation) -> Result<SampleLocation, DemuxError>,
    ) -> Result<Self, DemuxError> {
        Ok(Self {
            locations: self
                .locations
                .map(replace)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter(),
        })
    }
}

impl Iterator for SampleLocations {
    type Item = SampleLocation;

    fn next(&mut self) -> Option<SampleLocation> {
        self.locations.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.locations.size_hint()
    }
}

impl ExactSizeIterator for SampleLocations {}

impl FusedIterator for SampleLocations {}

/// Samples held for `IdentifiedMediaDataBox`es not read yet, by their `imda_identifier`, each located from the first byte of the box's payload
type Held = BTreeMap<u32, Vec<SampleLocation>>;

/// Returns `location`, which counts from the first byte of the payload of an `IdentifiedMediaDataBox`, located in the file, that payload starting at file offset `payload_offset`
///
/// # Errors
///
/// * [`DataOffsetOutOfRange`](DemuxError::DataOffsetOutOfRange): the media
///   data lies past 64-bit file offsets.
fn located(location: &SampleLocation, payload_offset: u64) -> Result<SampleLocation, DemuxError> {
    let range = location.byte_range();
    let out_of_range = DemuxError::DataOffsetOutOfRange {
        track_id: location.properties().track_id,
    };
    let start = payload_offset
        .checked_add(range.start)
        .ok_or(out_of_range)?;
    let end = payload_offset.checked_add(range.end).ok_or(out_of_range)?;
    Ok(location.with_byte_range(start..end))
}

/// Sorts `locations` into the order their media data lies in the file, samples of no bytes at one offset by track and decoding time
fn in_file_order(locations: &mut [SampleLocation]) {
    // Why not a stable sort by offset alone: it allocates scratch in proportion to the locations, which the sample limit lets grow large.
    locations.sort_unstable_by_key(|location| {
        let properties = location.properties();
        (
            location.byte_range().start,
            properties.track_id,
            properties.decoding_time,
        )
    });
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::{
        ChunkOffsetBox, CompactSampleSizeBox, CompositionOffsetBox, CompositionOffsetEntry,
        DataEntry, DataEntryImdaBox, DegradationPriorityBox, MovieBox, PaddingBitsBox,
        SampleDependencyTypeBox, SampleFlags, SampleSizeBox, SampleSizeVariant, SampleToChunkBox,
        SyncSampleBox, TimeToSampleBox, TimeToSampleEntry, TrackBox,
    };
    use isobmff_core::{BoxDecode, BoxDefinition, BoxType};

    use super::{StructureDataLimits, StructureDataReader};
    use crate::error::DemuxError;
    use crate::leniency::{Leniency, SampleTableDisagreement};
    use crate::sample::{DataReferenceKey, SampleLocation, SampleProperties};
    use crate::structure_data::DataReference;
    use crate::structure_data::fixtures::{UNLIMITED, in_another_file, movie, track};

    /// Resolves the movie of `tracks` with no limit to speak of
    fn resolve(
        tracks: impl IntoIterator<Item = TrackBox>,
    ) -> Result<Vec<SampleLocation>, DemuxError> {
        StructureDataReader::new(UNLIMITED)
            .handle_movie(movie(tracks))
            .map(Iterator::collect)
    }

    /// Returns the location of a sample of 10 ticks of track 1, described by sample entry 1 and in the file itself
    fn sample(
        decoding_time: u64,
        byte_range: Range<u64>,
        sample_composition_time_offset: i64,
        sample_flags: u32,
    ) -> SampleLocation {
        SampleLocation::new(
            SampleProperties {
                track_id: 1,
                decoding_time,
                sample_duration: 10,
                sample_composition_time_offset,
                sample_flags: SampleFlags::from_bits(sample_flags),
                sample_description_index: 1,
            },
            DataReferenceKey::of(0, 1),
            byte_range,
        )
    }

    /// Returns the track, decoding time and byte range of each location
    fn placed(locations: &[SampleLocation]) -> Vec<(u32, u64, Range<u64>)> {
        locations
            .iter()
            .map(|location| {
                (
                    location.properties().track_id,
                    location.properties().decoding_time,
                    location.byte_range(),
                )
            })
            .collect()
    }

    #[test]
    fn the_samples_of_a_chunk_lie_one_after_another_from_its_offset() {
        // Chunk 1 at 100 holds samples 1 and 2, chunk 2 at 200 holds sample 3.
        let locations = resolve([track(1, 10, &[5, 6, 7], &[(1, 2), (2, 1)], &[100, 200])]);

        assert_eq!(
            placed(&locations.unwrap()),
            [(1, 0, 100..105), (1, 10, 105..111), (1, 20, 200..207)]
        );
    }

    #[test]
    fn the_samples_of_two_tracks_come_out_in_the_order_their_media_data_lies() {
        let audio = track(1, 1024, &[4, 4, 4, 4], &[(1, 2)], &[100, 300]);
        let video = track(2, 3000, &[50], &[(1, 1)], &[200]);

        assert_eq!(
            placed(&resolve([audio, video]).unwrap()),
            [
                (1, 0, 100..104),
                (1, 1024, 104..108),
                (2, 0, 200..250),
                (1, 2048, 300..304),
                (1, 3072, 304..308),
            ]
        );
    }

    #[test]
    fn chunks_laid_out_of_decoding_order_come_by_track_in_decoding_order() {
        let audio = track(2, 1024, &[4, 4], &[(1, 1)], &[300, 100]);
        let video = track(1, 3000, &[50, 50], &[(1, 1)], &[450, 350]);
        let locations = StructureDataReader::new(UNLIMITED)
            .handle_movie(movie([audio, video]))
            .unwrap();

        assert_eq!(
            placed(&locations.clone().collect::<Vec<_>>()),
            [
                (2, 1024, 100..104),
                (2, 0, 300..304),
                (1, 3000, 350..400),
                (1, 0, 450..500),
            ]
        );
        assert_eq!(
            placed(&locations.into_decoding_order()),
            [
                (1, 0, 450..500),
                (1, 3000, 350..400),
                (2, 0, 300..304),
                (2, 1024, 100..104),
            ]
        );
    }

    #[test]
    fn an_stz2_and_a_co64_read_as_an_stsz_and_an_stco() {
        let mut trak = track(1, 10, &[3, 4], &[(1, 2)], &[0x1_0000_0000]);
        let stz2 =
            CompactSampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\x08\0\0\0\x02\x03\x04").unwrap();
        trak.mdia.minf.stbl.sample_size = Some(SampleSizeVariant::CompactSampleSize(stz2));

        assert_eq!(
            placed(&resolve([trak]).unwrap()),
            [
                (1, 0, 0x1_0000_0000..0x1_0000_0003),
                (1, 10, 0x1_0000_0003..0x1_0000_0007)
            ]
        );
    }

    #[test]
    fn the_composition_offsets_of_a_ctts_of_version_1_may_be_negative() {
        let mut trak = track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut ctts = CompositionOffsetBox::default();
        ctts.version = 1;
        ctts.entries = [(1, 20), (2, -10)]
            .map(|(sample_count, sample_offset)| {
                let mut entry = CompositionOffsetEntry::default();
                entry.sample_count = sample_count;
                entry.sample_offset = sample_offset;
                entry
            })
            .to_vec();
        trak.mdia.minf.stbl.ctts = Some(ctts);

        assert_eq!(
            resolve([trak]),
            Ok(Vec::from([
                sample(0, 0..1, 20, 0),
                sample(10, 1..2, -10, 0),
                sample(20, 2..3, -10, 0),
            ]))
        );
    }

    #[test]
    fn the_stss_and_the_sdtp_make_up_the_sample_flags() {
        let mut trak = track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = Vec::from([1, 3]);
        trak.mdia.minf.stbl.stss = Some(stss);
        // is_leading 0, sample_depends_on 2, then 1 and 1, sample_is_depended_on 1, 2, 1
        trak.mdia.minf.stbl.sdtp =
            Some(SampleDependencyTypeBox::decode_payload(b"\0\0\0\0\x24\x18\x14").unwrap());

        assert_eq!(
            resolve([trak]),
            Ok(Vec::from([
                sample(0, 0..1, 0, 0x0240_0000),
                sample(10, 1..2, 0, 0x0181_0000),
                sample(20, 2..3, 0, 0x0140_0000),
            ]))
        );
    }

    #[test]
    fn the_padb_and_the_stdp_make_up_the_sample_flags() {
        let mut trak = track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        // Three samples padded with 3, 5 and 6 bits, the second half of the last byte unused
        trak.mdia.minf.stbl.padb =
            Some(PaddingBitsBox::decode_payload(b"\0\0\0\0\0\0\0\x03\x35\x60").unwrap());
        trak.mdia.minf.stbl.stdp =
            Some(DegradationPriorityBox::decode_payload(b"\0\0\0\0\0\0\0\x07\x01\0").unwrap());

        assert_eq!(
            resolve([trak]),
            Ok(Vec::from([
                sample(0, 0..1, 0, 0x0006_0000),
                sample(10, 1..2, 0, 0x000a_0007),
                sample(20, 2..3, 0, 0x000c_0100),
            ]))
        );
    }

    #[test]
    fn every_sample_of_a_track_without_an_stss_is_a_sync_sample() {
        assert_eq!(
            resolve([track(1, 10, &[1, 1], &[(1, 2)], &[0])]),
            Ok(Vec::from([sample(0, 0..1, 0, 0), sample(10, 1..2, 0, 0)]))
        );
    }

    /// Returns the location of a sample of track 7, described by sample entry 1 and in the file itself
    fn located(
        decoding_time: u64,
        sample_duration: u32,
        sample_composition_time_offset: i64,
        sample_flags: u32,
        byte_range: Range<u64>,
    ) -> SampleLocation {
        SampleLocation::new(
            SampleProperties {
                track_id: 7,
                decoding_time,
                sample_duration,
                sample_composition_time_offset,
                sample_flags: SampleFlags::from_bits(sample_flags),
                sample_description_index: 1,
            },
            DataReferenceKey::of(0, 1),
            byte_range,
        )
    }

    /// Resolves the movie of `tracks`, and returns the locations of its samples and what the reader read past
    fn resolve_leniently(
        tracks: impl IntoIterator<Item = TrackBox>,
    ) -> (Vec<SampleLocation>, Vec<Leniency>) {
        let mut reader = StructureDataReader::new(UNLIMITED);
        let locations = reader.handle_movie(movie(tracks)).unwrap().collect();
        (locations, reader.leniencies().to_vec())
    }

    /// Returns the leniency of the table `box_type` of track 7 of `kind`
    fn read_past(box_type: &[u8; 4], kind: SampleTableDisagreement) -> Leniency {
        Leniency::SampleTable {
            track_id: 7,
            box_type: BoxType::compact(*box_type),
            kind,
        }
    }

    #[test]
    fn the_samples_past_a_short_stts_take_its_last_duration() {
        let mut trak = track(7, 0, &[1, 1, 1], &[(1, 3)], &[0]);
        if let Some(stts) = trak.mdia.minf.stbl.stts.as_mut() {
            stts.entries = [(1, 10), (1, 20)]
                .map(|(sample_count, sample_delta)| {
                    let mut entry = TimeToSampleEntry::default();
                    entry.sample_count = sample_count;
                    entry.sample_delta = sample_delta;
                    entry
                })
                .to_vec();
        }

        assert_eq!(
            resolve_leniently([trak]),
            (
                Vec::from([
                    located(0, 10, 0, 0, 0..1),
                    located(10, 20, 0, 0, 1..2),
                    located(30, 20, 0, 0, 2..3)
                ]),
                Vec::from([read_past(
                    b"stts",
                    SampleTableDisagreement::Short { missing_samples: 1 }
                )])
            )
        );
    }

    #[test]
    fn the_samples_a_track_has_are_those_its_chunks_lay_out_as_far_as_its_stsz_gives_sizes() {
        // Chunk 2 has room for two more samples than the stsz gives sizes for
        let chunk_past_the_samples = track(7, 10, &[1, 1], &[(1, 2)], &[0, 9]);
        // The stsz and the stts state a third sample no chunk lays out
        let samples_past_the_chunks = track(7, 10, &[1, 1, 1], &[(1, 2)], &[0]);
        let first_two = Vec::from([located(0, 10, 0, 0, 0..1), located(10, 10, 0, 0, 1..2)]);

        assert_eq!(
            [
                resolve_leniently([chunk_past_the_samples]),
                resolve_leniently([samples_past_the_chunks]),
            ],
            [
                (
                    first_two.clone(),
                    Vec::from([read_past(
                        b"stsc",
                        SampleTableDisagreement::Long { excess_samples: 2 }
                    )])
                ),
                (
                    first_two,
                    Vec::from([
                        read_past(b"stsz", SampleTableDisagreement::Long { excess_samples: 1 }),
                        read_past(b"stts", SampleTableDisagreement::Long { excess_samples: 1 }),
                    ])
                ),
            ]
        );
    }

    #[test]
    fn samples_an_stz2_gives_sizes_past_the_chunks_are_left_out_of_the_stz2() {
        let mut trak = track(7, 10, &[1, 1], &[(1, 1)], &[0]);
        trak.mdia.minf.stbl.sample_size = Some(SampleSizeVariant::CompactSampleSize(
            CompactSampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\x08\0\0\0\x02\x01\x01").unwrap(),
        ));

        assert_eq!(
            resolve_leniently([trak]),
            (
                Vec::from([located(0, 10, 0, 0, 0..1)]),
                Vec::from([
                    read_past(b"stz2", SampleTableDisagreement::Long { excess_samples: 1 }),
                    read_past(b"stts", SampleTableDisagreement::Long { excess_samples: 1 }),
                ])
            )
        );
    }

    #[test]
    fn the_samples_past_another_short_table_take_zero_and_the_entries_past_the_last_sample_are_left_out()
     {
        let mut trak = track(7, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let stbl = &mut trak.mdia.minf.stbl;
        let mut ctts = CompositionOffsetBox::default();
        let mut entry = CompositionOffsetEntry::default();
        entry.sample_count = 1;
        entry.sample_offset = 5;
        ctts.entries = Vec::from([entry]);
        stbl.ctts = Some(ctts);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = Vec::from([1, 4, 5]);
        stbl.stss = Some(stss);
        let mut padb = PaddingBitsBox::default();
        padb.pads = Vec::from([3]);
        stbl.padb = Some(padb);
        let mut stdp = DegradationPriorityBox::default();
        stdp.priorities = Vec::from([1, 2, 3, 4]);
        stbl.stdp = Some(stdp);

        assert_eq!(
            resolve_leniently([trak]),
            (
                Vec::from([
                    located(0, 10, 5, 0x0006_0001, 0..1),
                    located(10, 10, 0, 0x0001_0002, 1..2),
                    located(20, 10, 0, 0x0001_0003, 2..3),
                ]),
                Vec::from([
                    read_past(
                        b"ctts",
                        SampleTableDisagreement::Short { missing_samples: 2 }
                    ),
                    read_past(
                        b"padb",
                        SampleTableDisagreement::Short { missing_samples: 2 }
                    ),
                    read_past(b"stdp", SampleTableDisagreement::Long { excess_samples: 1 }),
                    read_past(b"stss", SampleTableDisagreement::Long { excess_samples: 2 }),
                ])
            )
        );
    }

    #[test]
    fn the_sample_numbers_an_stss_marks_out_of_order_are_passed_over() {
        let mut trak = track(7, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = Vec::from([2, 1, 3]);
        trak.mdia.minf.stbl.stss = Some(stss);

        assert_eq!(
            resolve_leniently([trak]),
            (
                Vec::from([
                    located(0, 10, 0, 0x0001_0000, 0..1),
                    located(10, 10, 0, 0, 1..2),
                    located(20, 10, 0, 0, 2..3),
                ]),
                Vec::from([read_past(
                    b"stss",
                    SampleTableDisagreement::Unordered { sample_numbers: 1 }
                )])
            )
        );
    }

    #[test]
    fn a_track_whose_tables_cannot_lay_out_its_samples_is_refused() {
        let without = |box_type: &[u8; 4]| {
            let mut trak = track(7, 10, &[1, 1], &[(1, 2)], &[0]);
            let stbl = &mut trak.mdia.minf.stbl;
            match box_type {
                b"stsz" => stbl.sample_size = None,
                b"stco" => stbl.chunk_offset = None,
                b"stsc" => stbl.stsc = None,
                _ => stbl.stts = None,
            }
            trak
        };
        let runs_past_the_chunks = track(7, 10, &[1, 1], &[(1, 1), (3, 1)], &[0, 1]);
        let runs_of_no_sample = track(7, 10, &[1, 1], &[(1, 0)], &[0]);
        let mut empty_stz2 = track(7, 10, &[1, 1], &[(1, 2)], &[0]);
        empty_stz2.mdia.minf.stbl.sample_size = Some(SampleSizeVariant::CompactSampleSize(
            CompactSampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\x08\0\0\0\0").unwrap(),
        ));

        for (trak, box_type) in [
            (without(b"stsz"), SampleSizeBox::BOX_TYPE),
            (empty_stz2, CompactSampleSizeBox::BOX_TYPE),
            (runs_of_no_sample, SampleToChunkBox::BOX_TYPE),
            (without(b"stco"), ChunkOffsetBox::BOX_TYPE),
            (without(b"stsc"), SampleToChunkBox::BOX_TYPE),
            (without(b"stts"), TimeToSampleBox::BOX_TYPE),
            (runs_past_the_chunks, SampleToChunkBox::BOX_TYPE),
        ] {
            assert_eq!(
                resolve([trak]),
                Err(DemuxError::SampleTableMismatch {
                    track_id: 7,
                    box_type
                })
            );
        }
    }

    #[test]
    fn a_track_of_no_sample_is_read_without_a_leniency() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        let locations = reader.handle_movie(movie([track(7, 0, &[], &[], &[])]));

        assert_eq!(
            (locations.map(Iterator::count), reader.leniencies()),
            (Ok(0), &[][..])
        );
    }

    #[test]
    fn media_data_in_an_imda_of_a_sequence_number_or_in_a_data_reference_entry_the_track_has_none_of_is_refused()
     {
        let mut imda = track(3, 10, &[1], &[(1, 1)], &[0]);
        imda.mdia.minf.dinf.dref.data_entries = Vec::from([DataEntry::SeqNumImda(
            isobmff_boxes::DataEntrySeqNumImdaBox::default(),
        )]);
        let mut unknown = track(3, 10, &[1], &[(1, 1)], &[0]);
        unknown.mdia.minf.dinf.dref.data_entries.clear();

        assert_eq!(
            resolve([imda]),
            Err(DemuxError::UnsupportedDataReference {
                track_id: 3,
                data_reference_index: 1
            })
        );
        assert_eq!(
            resolve([unknown]),
            Err(DemuxError::UnknownDataReferenceIndex {
                track_id: 3,
                data_reference_index: 1
            })
        );
    }

    /// Returns track 1, its samples of sizes 4 and 6 in a chunk at offset 4 of the payload of the `imda` of `identifier`
    fn identified(identifier: u32) -> TrackBox {
        let mut trak = track(1, 10, &[4, 6], &[(1, 2)], &[4]);
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = identifier;
        trak.mdia.minf.dinf.dref.data_entries = Vec::from([DataEntry::Imda(imdt)]);
        trak
    }

    #[test]
    fn samples_in_an_imda_not_read_yet_are_held_until_it_is_and_then_located_from_its_payload() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        let in_movie: Vec<_> = reader
            .handle_movie(movie([identified(7)]))
            .unwrap()
            .collect();
        let held = reader.unresolved_identified_media_data();
        let located: Vec<_> = reader
            .handle_identified_media_data(7, 1000)
            .unwrap()
            .collect();

        assert_eq!(
            (
                in_movie,
                held,
                placed(&located),
                reader.unresolved_identified_media_data()
            ),
            (
                Vec::new(),
                Some(7),
                Vec::from([(1, 0, 1004..1008), (1, 10, 1008..1014)]),
                None
            )
        );
    }

    #[test]
    fn samples_in_an_imda_read_before_them_are_located_from_its_payload_at_once() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        let before: Vec<_> = reader
            .handle_identified_media_data(7, 1000)
            .unwrap()
            .collect();
        let located: Vec<_> = reader
            .handle_movie(movie([identified(7)]))
            .unwrap()
            .collect();

        assert_eq!(
            (before, placed(&located)),
            (
                Vec::new(),
                Vec::from([(1, 0, 1004..1008), (1, 10, 1008..1014)])
            )
        );
    }

    #[test]
    fn a_second_imda_of_an_identifier_and_imdas_or_held_samples_past_the_limits_are_refused() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader.handle_identified_media_data(7, 1000).unwrap();
        let few = StructureDataLimits {
            identified_media_data_count: 1,
            unresolved_sample_count: 1,
            ..UNLIMITED
        };
        let mut limited = StructureDataReader::new(few);
        limited.handle_identified_media_data(1, 0).unwrap();

        assert_eq!(
            [
                reader.handle_identified_media_data(7, 2000).map(|_| ()),
                limited.handle_identified_media_data(2, 0).map(|_| ()),
                limited.handle_movie(movie([identified(9)])).map(|_| ()),
            ],
            [
                Err(DemuxError::DuplicateIdentifiedMediaData { identifier: 7 }),
                Err(DemuxError::IdentifiedMediaDataCountLimitExceeded {
                    retained_identified_media_data: 2,
                    limit_identified_media_data: 1
                }),
                Err(DemuxError::UnresolvedSampleCountLimitExceeded {
                    unresolved_samples: 2,
                    limit_samples: 1
                }),
            ]
        );
        assert_eq!(limited.movie(), None);
    }

    #[test]
    fn a_resumed_reader_forgets_the_samples_it_held_and_keeps_the_imdas_it_read() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader.handle_identified_media_data(3, 500).unwrap();
        reader.handle_movie(movie([identified(7)])).unwrap();
        reader.resume();

        assert_eq!(
            (
                reader.unresolved_identified_media_data(),
                reader.handle_identified_media_data(3, 600).map(|_| ())
            ),
            (
                None,
                Err(DemuxError::DuplicateIdentifiedMediaData { identifier: 3 })
            )
        );
    }

    #[test]
    fn media_data_in_another_file_is_located_in_that_file_and_its_data_reference_names_the_file() {
        let external = in_another_file(track(3, 10, &[4], &[(1, 1)], &[100]));
        let internal = track(1, 10, &[2], &[(1, 1)], &[0]);
        let entries = [&internal, &external]
            .map(|trak| trak.mdia.minf.dinf.dref.data_entries.first().cloned());
        let mut reader = StructureDataReader::new(UNLIMITED);

        let locations: Vec<_> = reader
            .handle_movie(movie([internal, external]))
            .unwrap()
            .collect();

        assert_eq!(
            locations
                .iter()
                .map(|location| (
                    location.properties().track_id,
                    location.byte_range(),
                    reader
                        .data_reference(location.data_reference())
                        .map(|data_reference| data_reference.entry().clone())
                ))
                .collect::<Vec<_>>(),
            [
                (1, 0..2, entries[0].clone()),
                (3, 100..104, entries[1].clone())
            ]
        );
    }

    #[test]
    fn each_entry_of_the_data_reference_of_each_track_is_listed_under_a_key_of_its_own() {
        let mut first = track(1, 10, &[2], &[(1, 1)], &[0]);
        let elsewhere = in_another_file(track(1, 10, &[], &[], &[]));
        first
            .mdia
            .minf
            .dinf
            .dref
            .data_entries
            .extend(elsewhere.mdia.minf.dinf.dref.data_entries);
        let second = in_another_file(track(3, 10, &[4], &[(1, 1)], &[100]));
        let entries: Vec<_> = [&first, &second]
            .iter()
            .flat_map(|trak| trak.mdia.minf.dinf.dref.data_entries.clone())
            .collect();
        let mut reader = StructureDataReader::new(UNLIMITED);
        let locations: Vec<_> = reader
            .handle_movie(movie([first, second]))
            .unwrap()
            .collect();
        reader.resume();

        let expected: Vec<_> = [(0, 1, 1), (0, 1, 2), (1, 3, 1)]
            .into_iter()
            .zip(&entries)
            .map(|((track_position, track_id, index), entry)| DataReference {
                key: DataReferenceKey::of(track_position, index),
                track_id,
                index,
                entry,
            })
            .collect();
        let named: Vec<_> = locations
            .iter()
            .map(|location| reader.data_reference(location.data_reference()))
            .collect();
        assert_eq!(
            (reader.data_references().collect::<Vec<_>>(), named),
            (
                expected.clone(),
                Vec::from([expected.first().copied(), expected.last().copied()])
            )
        );
        assert_eq!(reader.data_reference(DataReferenceKey::new(u64::MAX)), None);
    }

    #[test]
    fn a_movie_of_more_samples_than_the_limit_is_refused_and_not_kept() {
        let mut reader = StructureDataReader::new(StructureDataLimits {
            sample_count: 3,
            ..UNLIMITED
        });
        let tracks = [
            track(1, 10, &[1, 1], &[(1, 2)], &[0]),
            track(2, 10, &[1, 1], &[(1, 2)], &[2]),
        ];

        assert_eq!(
            reader.handle_movie(movie(tracks)).map(|_| ()),
            Err(DemuxError::SampleCountLimitExceeded {
                declared_samples: 4,
                limit_samples: 3
            })
        );
        assert!(reader.movie().is_none());
    }

    #[test]
    fn a_second_movie_is_refused() {
        let mut reader = StructureDataReader::new(UNLIMITED);
        reader.handle_movie(movie([])).unwrap();

        assert_eq!(
            reader.handle_movie(movie([])).map(|_| ()),
            Err(DemuxError::DuplicateBox {
                box_type: MovieBox::BOX_TYPE
            })
        );
    }
}
