//! [`Samples`], the locations of the samples the demuxer has read, in the order of their offsets, and how many are reported

use alloc::vec::Vec;

use isobmff_boxes::MovieBox;
use isobmff_core::BoxDefinition;

use super::error::Error;
use crate::SampleLocation;
use crate::structure_data::MovieReader;
use crate::structure_data::data_layout::DataOrigins;

/// The locations of the samples of the `moov`, at their offsets in their containers and in the order of those offsets, and how far they are reported
#[derive(Debug, Clone)]
pub(super) struct Samples {
    movie_reader: MovieReader,
    data_origins: DataOrigins,
    sample_count_limit: u64,
    /// The location of each sample, in the order of their offsets
    locations: Vec<SampleLocation>,
    // Why count the locations reported rather than remove each from
    // `locations` as it is reported: removing the first of a `Vec` moves all
    // the others.
    reported: usize,
}

impl Samples {
    /// Creates samples of no `moov`, which hold the locations of at most `sample_count_limit` samples
    pub(super) const fn new(sample_count_limit: u64) -> Self {
        Self {
            movie_reader: MovieReader::new(),
            data_origins: DataOrigins::new(),
            sample_count_limit,
            locations: Vec::new(),
            reported: 0,
        }
    }

    /// Takes the locations of the samples of `movie`, the `moov` at file offset `offset`
    ///
    /// # Errors
    ///
    /// Refused, nothing is changed.
    ///
    /// * [`StructureData`](Error::StructureData): the structure-data refuses
    ///   the samples of `movie`, or one of their locations.
    /// * [`SampleCountOverLimit`](Error::SampleCountOverLimit): `movie`
    ///   declares more samples than the limit; none is located.
    pub(super) fn handle_movie(&mut self, offset: u64, movie: &MovieBox) -> Result<(), Error> {
        let refused = |source| Error::StructureData {
            offset,
            box_type: MovieBox::BOX_TYPE,
            source,
        };
        let locations = self.movie_reader.handle_movie(movie).map_err(refused)?;
        let location_count = locations.remaining();
        if location_count > self.sample_count_limit {
            return Err(Error::SampleCountOverLimit {
                offset,
                box_type: MovieBox::BOX_TYPE,
                location_count,
                limit: self.sample_count_limit,
            });
        }
        let mut located = Vec::with_capacity(usize::try_from(location_count).unwrap_or_default());
        for location in locations {
            located.push(
                self.data_origins
                    .handle_location(location)
                    .map_err(refused)?,
            );
        }
        // Why not sort_unstable_by_key, which allocates nothing: samples at the
        // same offset, such as those of no bytes, would come in an order of
        // the sort's choosing rather than that of the tracks and of decoding.
        located.sort_by_key(|location| location.offset);
        self.locations = located;
        self.reported = 0;
        Ok(())
    }

    /// The first location not yet reported
    pub(super) fn unreported(&self) -> Option<SampleLocation> {
        self.locations.get(self.reported).copied()
    }

    /// Counts the first location not yet reported as reported
    pub(super) const fn report(&mut self) {
        self.reported = self.reported.saturating_add(1);
    }
}
