//! [`StructureDataReader`], which resolves the samples a box declares to where their bytes lie

use alloc::vec::Vec;

use isobmff_boxes::MovieBox;

use super::{Error, sample_table};
use crate::SampleLocation;

/// Reader of the samples the structure-data of a file declares, from the boxes read into their values
///
/// The sample tables of the tracks of a movie, ISO/IEC 14496-12 §8.5.
///
/// [`handle_movie`](Self::handle_movie) takes a `moov` and returns the
/// [`SampleLocation`] of each sample of its tracks, in the order of their file
/// offsets. It refuses a sample table whose tables count different numbers of
/// samples, or a track whose samples lie in another file. It does not read the
/// bytes of the file: reading the `moov` out of them, and the samples where
/// their locations say, is up to the caller.
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
/// let mut structure_data = StructureDataReader::new(1024);
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
}

impl StructureDataReader {
    /// Creates a reader that resolves at most `sample_count_limit` samples from one box
    ///
    /// The number of samples is read from the input, and a box of a few bytes
    /// can declare millions, so the reader holds the declared number against
    /// this limit before it allocates room for them.
    #[must_use]
    pub const fn new(sample_count_limit: u64) -> Self {
        Self {
            sample_count_limit,
            locations: Vec::new(),
        }
    }

    /// Returns the location of each sample the tracks of `movie` declare, in the order of their file offsets
    ///
    /// Samples at the same offset keep the order of their tracks in `movie`,
    /// and within a track the decoding order.
    ///
    /// # Errors
    /// * [`Error::SampleCountOverLimit`]: the tracks declare more samples than
    ///   the limit, checked before anything is allocated for them.
    /// * [`Error::MissingTable`], [`Error::SampleCountMismatch`],
    ///   [`Error::InvalidEntry`]: the sample table of a track lacks a table it
    ///   needs, or its tables disagree.
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
        for track in &movie.trak {
            sample_table::resolve(track, &mut self.locations)?;
        }
        self.locations.sort_by_key(|location| location.offset);
        Ok(self.locations.drain(..))
    }
}
