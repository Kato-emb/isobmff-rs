//! The `structure-data` of a file, which gives the samples their location, size, timing and characteristics
//!
//! The `moov` and `moof` of a file declare its samples in tables, ISO/IEC
//! 14496-12 §3.1.54. [`MovieReader`] reads the [`SampleLocation`] of each
//! sample the sample tables of a `moov` declare. [`timeline`] reads when the
//! samples of a sample table are decoded and composed, and how long they
//! last; [`data_layout`] where their bytes lie, which data reference entry
//! specifies their container, and, with
//! [`DataOrigins`](data_layout::DataOrigins), at which file offset.

use alloc::boxed::Box;
use alloc::vec::{self, Vec};
use core::iter::FusedIterator;

use isobmff_boxes::MovieBox;

use crate::SampleLocation;

pub mod data_layout;
mod error;
mod sample_table;
pub mod timeline;

pub use error::Error;

use sample_table::Samples;

/// Reader of the `location of samples` the `structure-data` of a movie declares, from its boxes read into their values
///
/// The `structure-data`, ISO/IEC 14496-12 §3.1.54, of the `movie box`, §3.1.30
///
/// [`handle_movie`](Self::handle_movie) takes a `moov` and returns the
/// [`SampleLocation`] of each sample its sample tables declare. It does not
/// read the bytes of a file: reading the `moov` out of them, and the bytes of
/// the samples where their locations say, is up to the caller.
///
/// ```
/// use isobmff::boxes::MovieBox;
/// use isobmff::file_level::{BoxReader, BoxReaderConfig, Event};
/// use isobmff::structure_data::MovieReader;
/// use isobmff::{BoxDecode, BoxDefinition};
///
/// let file: &[u8] = include_bytes!("../tests/fixtures/movie_moov_first.mp4");
///
/// // Read the `moov` out of the file
/// let mut boxes = BoxReader::new(BoxReaderConfig::new(1024 * 1024));
/// let mut movie = None;
/// loop {
///     while let Some(event) = boxes.poll_event() {
///         if let Event::BoxPayload { box_type, payload, .. } = event
///             && box_type == MovieBox::BOX_TYPE
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
/// // Read where its samples lie, and when they are decoded
/// let movie = movie.ok_or("no moov")?;
/// let locations: Vec<_> = MovieReader::new().handle_movie(&movie)?.collect();
/// assert_eq!(locations.len(), 12 + 58);
///
/// // The samples of each track come in decoding order
/// assert!(locations.windows(2).all(|pair| {
///     let (before, after) = (&pair[0].properties, &pair[1].properties);
///     before.track_id != after.track_id
///         || after.decoding_time == before.decoding_time + u64::from(before.sample_duration)
/// }));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct MovieReader {}

impl MovieReader {
    /// Creates a reader that has taken no `moov`
    #[expect(
        clippy::new_without_default,
        reason = "a reader is no value the specification gives a default to, which `Default` stands for here"
    )]
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    /// Returns the location of each sample the sample tables of the tracks of `movie` declare: the samples of each track in decoding order, track after track
    ///
    /// The tracks come in the order of the `trak`s of `movie`. Every sample
    /// table is checked before anything is returned, so a `moov` is refused
    /// whole, or read whole.
    ///
    /// # Errors
    ///
    /// * [`Track`](Error::Track): the sample table of the first track whose
    ///   tables are refused, with why. They are refused, in this order: where
    ///   a table the sample table must hold is absent; where they disagree, as
    ///   a sample-to-chunk table whose `first_chunk`s do not go up from 1,
    ///   ISO/IEC 14496-12 §8.7.4.3, a table that covers another number of
    ///   samples than the sample size table, §8.7.3.2, or a sync sample table
    ///   not in strictly increasing order of sample number, §8.6.2.1; where
    ///   the sample entry of a sample names no data reference entry, §8.5.2.3,
    ///   one whose samples are each in a movie fragment, §8.7.2.1, or one of an
    ///   `IdentifiedMediaDataBox` that does not decode; and where the bytes of
    ///   a sample end past the last data offset.
    pub fn handle_movie<'a>(&mut self, movie: &'a MovieBox) -> Result<SampleLocations<'a>, Error> {
        let tracks = movie
            .trak
            .iter()
            .map(|trak| {
                let track_id = trak.tkhd.track_id;
                sample_table::samples(&trak.mdia.minf.stbl, &trak.mdia.minf.dinf.dref, track_id)
                    .map_err(|source| Error::Track {
                        track_id,
                        source: Box::new(source),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SampleLocations {
            tracks: tracks.into_iter(),
        })
    }
}

/// The `location of samples` the structure-data of a movie declares, the samples of each track in decoding order, track after track
///
/// The `location of samples`, ISO/IEC 14496-12 §6.1
///
/// [`MovieReader::handle_movie`] returns them. They are not in the order of
/// their data offsets, nor of their presentation times: no edit list is
/// applied, and each [`SampleLocation`] gives the decoding time and the
/// composition offset of its sample.
#[derive(Debug, Clone)]
pub struct SampleLocations<'a> {
    /// The samples of the tracks not yet returned, from those of the current
    /// track
    tracks: vec::IntoIter<Samples<'a>>,
}

impl SampleLocations<'_> {
    /// Returns the number of samples left to return
    ///
    /// Before any sample is returned, it is the number of samples the sample
    /// tables declare, so a caller can hold it against a limit before it keeps
    /// the samples.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.tracks
            .as_slice()
            .iter()
            .map(|samples| u64::from(samples.remaining()))
            .sum()
    }
}

impl Iterator for SampleLocations<'_> {
    type Item = SampleLocation;

    fn next(&mut self) -> Option<SampleLocation> {
        loop {
            if let Some(location) = self.tracks.as_mut_slice().first_mut()?.next() {
                return Some(location);
            }
            self.tracks.next();
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::try_from(self.remaining()).ok();
        (remaining.unwrap_or(usize::MAX), remaining)
    }
}

impl FusedIterator for SampleLocations<'_> {}
