//! Where the `data origin`s of the samples of a file lie, and so where each sample lies in the file
//!
//! The location of a sample names the container its bytes lie in and the
//! origin its data offset counts from; which file offset that origin lies
//! at comes from the boxes as a caller reads or writes them.

use crate::structure_data::Error;
use crate::{DataOrigin, MediaDataContainer, SampleLocation};

/// The file offsets of the `data origin`s of a file, which turn the location of a sample into one at its file offset
///
/// The `data origin`, ISO/IEC 14496-12 §6.1 Table 1, and §8.8.7.1 Table 6
///
/// [`handle_movie_fragment`](Self::handle_movie_fragment) takes where a
/// `moof` lies, from which the samples of its track fragments that count
/// from it count. [`handle_location`](Self::handle_location) then returns a
/// location at its offset from the start of its container, its
/// `data_origin` [`Container`](DataOrigin::Container): a file offset where
/// the container is [`SameFile`](MediaDataContainer::SameFile), and one of
/// the resource where it is [`Resource`](MediaDataContainer::Resource). It
/// takes no `IdentifiedMediaDataBox`, so a sample that counts from one is
/// refused. It does not read the bytes of a file.
///
/// ```
/// use isobmff::boxes::SampleFlags;
/// use isobmff::structure_data::data_layout::DataOrigins;
/// use isobmff::{DataOrigin, MediaDataContainer, SampleLocation, SampleProperties};
///
/// // A sample 16 bytes into the movie fragment, in the file of the `moof`
/// let location = SampleLocation {
///     properties: SampleProperties {
///         track_id: 1,
///         decoding_time: 0,
///         sample_duration: 1024,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::default(),
///         sample_description_index: 1,
///     },
///     container: MediaDataContainer::SameFile,
///     data_origin: DataOrigin::MovieFragment,
///     offset: 16,
///     size: 100,
/// };
///
/// // The `moof` lies at file offset 1000
/// let mut data_origins = DataOrigins::new();
/// data_origins.handle_movie_fragment(1000);
/// let location = data_origins.handle_location(location)?;
/// assert_eq!((location.data_origin, location.offset), (DataOrigin::Container, 1016));
/// # Ok::<(), isobmff::structure_data::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct DataOrigins {
    /// File offset of the first byte of the `moof` taken last, or `None`
    /// before the first
    movie_fragment_offset: Option<u64>,
}

impl DataOrigins {
    /// Creates data origins of which no position is taken
    #[expect(
        clippy::new_without_default,
        reason = "data origins whose positions are taken from a file are no value the specification gives a default to, which `Default` stands for here"
    )]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            movie_fragment_offset: None,
        }
    }

    /// Takes the file offset of the `moof` whose samples come next
    ///
    /// The samples of its track fragments that count from the first byte of
    /// the `moof` count from `moof_offset`, ISO/IEC 14496-12 §8.8.7.1 Table
    /// 6, until the next `moof` is taken.
    pub const fn handle_movie_fragment(&mut self, moof_offset: u64) {
        self.movie_fragment_offset = Some(moof_offset);
    }

    /// Returns `location` at its offset from the start of its container, its `data_origin` [`Container`](DataOrigin::Container)
    ///
    /// A location that counts from the start of the same file or of a
    /// resource is returned as it is, unchecked: its data offset is an offset
    /// in that file, ISO/IEC 14496-12 §8.7.5.3 for a sample table and the
    /// last row of §8.8.7.1 Table 6 for a track fragment. One that counts
    /// from a movie fragment is returned at the file offset of the `moof`
    /// taken last plus its data offset, §8.8.7.1 Table 6, in the container
    /// it lies in: the same file where it lies in an `IdentifiedMediaDataBox`
    /// of the file, whose presence is not checked. A location returned is
    /// returned as it is when taken again.
    ///
    /// # Errors
    ///
    /// Refused, nothing is changed.
    ///
    /// * [`IdentifiedMediaDataNotRead`](Error::IdentifiedMediaDataNotRead):
    ///   `location` counts from the start of an `IdentifiedMediaDataBox`, none
    ///   of which is taken.
    /// * [`MovieFragmentPositionMissing`](Error::MovieFragmentPositionMissing):
    ///   `location` counts from a movie fragment, and no `moof` was taken.
    /// * [`FileOffsetOverflow`](Error::FileOffsetOverflow): `location` counts
    ///   from a movie fragment, and the end of its bytes, the file offset just
    ///   past the last of them, is past what a `u64` counts once the offset of
    ///   the `moof` is added.
    pub fn handle_location(&mut self, location: SampleLocation) -> Result<SampleLocation, Error> {
        let container = match (location.data_origin, location.container) {
            (
                DataOrigin::Container,
                MediaDataContainer::IdentifiedMediaData { imda_identifier },
            ) => {
                return Err(Error::IdentifiedMediaDataNotRead { imda_identifier });
            }
            (
                DataOrigin::Container,
                MediaDataContainer::SameFile | MediaDataContainer::Resource,
            ) => {
                return Ok(location);
            }
            (DataOrigin::MovieFragment, MediaDataContainer::IdentifiedMediaData { .. }) => {
                MediaDataContainer::SameFile
            }
            (
                DataOrigin::MovieFragment,
                container @ (MediaDataContainer::SameFile | MediaDataContainer::Resource),
            ) => container,
        };
        let moof_offset = self
            .movie_fragment_offset
            .ok_or(Error::MovieFragmentPositionMissing)?;
        // Why check the end, the offset just past the last byte, and not the
        // start alone: the byte range of a sample is taken up to its end,
        // which must be a `u64`, so a sample whose last byte is at `u64::MAX`
        // is refused too.
        let offset = moof_offset
            .checked_add(location.offset)
            .filter(|offset| offset.checked_add(u64::from(location.size)).is_some())
            .ok_or(Error::FileOffsetOverflow {
                data_origin_offset: moof_offset,
                offset: location.offset,
                size: location.size,
            })?;
        Ok(SampleLocation {
            container,
            data_origin: DataOrigin::Container,
            offset,
            ..location
        })
    }
}
