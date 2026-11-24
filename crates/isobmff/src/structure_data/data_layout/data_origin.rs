//! Where the `data origin`s of the samples of a file lie, and so where each sample lies in the file
//!
//! The location of a sample names the origin its data offset counts from;
//! which file offset that origin lies at comes from the boxes as a caller
//! reads or writes them.

use isobmff_boxes::DataEntry;

use super::data_reference::media_data_container;
use crate::structure_data::Error;
use crate::{DataOrigin, MediaDataContainer, SampleLocation};

/// The file offsets of the `data origin`s of a file, which turn the location of a sample into one at its file offset
///
/// The `data origin`, ISO/IEC 14496-12 §6.1 Table 1, and §8.8.7.1 Table 6
///
/// [`handle_movie_fragment`](Self::handle_movie_fragment) takes where a
/// `moof` lies, from which the samples of its track fragments that count
/// from it count. [`handle_location`](Self::handle_location) then returns a
/// location at its file offset, its `data_origin` [`File`](DataOrigin::File).
/// It takes no `IdentifiedMediaDataBox`, so a sample that counts from one is
/// refused. It does not read the bytes of a file, nor tell a file from
/// another: a location whose data reference entry names another file is
/// returned at its offset in that file, and
/// [`DataEntry::is_in_same_file`] tells which.
///
/// ```
/// use isobmff::boxes::{DataEntry, DataEntryUrlBox, SampleFlags};
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
/// let same_file = DataEntry::Url(DataEntryUrlBox::default());
///
/// // The `moof` lies at file offset 1000
/// let mut data_origins = DataOrigins::new();
/// data_origins.handle_movie_fragment(1000, 1);
/// let location = data_origins.handle_location(location, &same_file)?;
/// assert_eq!((location.data_origin, location.offset), (DataOrigin::File, 1016));
/// # Ok::<(), isobmff::structure_data::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct DataOrigins {
    /// Where the `moof` taken last lies, or `None` before the first
    movie_fragment: Option<MovieFragmentPosition>,
}

/// Where a `moof` lies, and the `sequence_number` of its `mfhd`, ISO/IEC 14496-12 §8.8.5
#[derive(Debug, Clone, Copy)]
struct MovieFragmentPosition {
    /// File offset of the first byte of the `moof`
    offset: u64,
    sequence_number: u32,
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
            movie_fragment: None,
        }
    }

    /// Takes where the `moof` whose samples come next lies: its file offset, and the `sequence_number` of its `mfhd`
    ///
    /// The samples of its track fragments that count from the first byte of
    /// the `moof` count from `moof_offset`, ISO/IEC 14496-12 §8.8.7.1 Table
    /// 6, until the next `moof` is taken. A `DataEntrySeqNumImdaBox` names
    /// the `IdentifiedMediaDataBox` of `sequence_number`, §8.7.2.1.
    pub const fn handle_movie_fragment(&mut self, moof_offset: u64, sequence_number: u32) {
        self.movie_fragment = Some(MovieFragmentPosition {
            offset: moof_offset,
            sequence_number,
        });
    }

    /// Returns `location` at its file offset, its `data_origin` [`File`](DataOrigin::File)
    ///
    /// `data_entry` is the data reference entry of the sample entry of
    /// `location`, as [`data_entry`](super::data_entry) looks it up. A
    /// location that counts from the start of a file is returned as it is,
    /// unchecked: its data offset is a file offset in the file the entry
    /// identifies, ISO/IEC 14496-12 §8.7.5.3 for a sample table and the last
    /// row of §8.8.7.1 Table 6 for a track fragment. One that counts from a
    /// movie fragment is
    /// returned at the file offset of the `moof` taken last plus its data
    /// offset, whether its media data lies in the file or in an
    /// `IdentifiedMediaDataBox`, §8.8.7.1 Table 6; that such an
    /// `IdentifiedMediaDataBox` lies in the file is not checked.
    ///
    /// # Errors
    ///
    /// The data reference entry is looked at first, and then the data origin
    /// of `location`. Refused, nothing is changed.
    ///
    /// * [`SequenceNumberOutsideMovieFragment`](Error::SequenceNumberOutsideMovieFragment):
    ///   `data_entry` is a `DataEntrySeqNumImdaBox`, and no `moof` was taken,
    ///   §8.7.2.1.
    /// * [`DataEntryNotDecoded`](Error::DataEntryNotDecoded): `data_entry` is
    ///   a `DataEntryImdaBox` or a `DataEntrySeqNumImdaBox` that does not
    ///   decode.
    /// * [`MovieFragmentPositionMissing`](Error::MovieFragmentPositionMissing):
    ///   `location` counts from a movie fragment, and no `moof` was taken.
    /// * [`FileOffsetOverflow`](Error::FileOffsetOverflow): `location` counts
    ///   from a movie fragment, and the end of its bytes, the file offset just
    ///   past the last of them, is past what a `u64` counts once the offset of
    ///   the `moof` is added.
    /// * [`IdentifiedMediaDataNotRead`](Error::IdentifiedMediaDataNotRead):
    ///   `location` counts from an `IdentifiedMediaDataBox`, none of which is
    ///   taken.
    /// * [`DataOriginMismatch`](Error::DataOriginMismatch): `location` counts
    ///   from the start of its file and `data_entry` places the media data in
    ///   an `IdentifiedMediaDataBox`, or `location` counts from an
    ///   `IdentifiedMediaDataBox` and `data_entry` places it in none.
    pub fn handle_location(
        &mut self,
        location: SampleLocation,
        data_entry: &DataEntry,
    ) -> Result<SampleLocation, Error> {
        let sample_description_index = location.properties.sample_description_index;
        let container = media_data_container(
            data_entry,
            sample_description_index,
            self.movie_fragment
                .map(|movie_fragment| movie_fragment.sequence_number),
        )?;
        match (location.data_origin, container) {
            (DataOrigin::File, MediaDataContainer::SameFile | MediaDataContainer::Resource) => {
                Ok(SampleLocation {
                    container,
                    ..location
                })
            }
            (DataOrigin::MovieFragment, _) => {
                let movie_fragment = self
                    .movie_fragment
                    .ok_or(Error::MovieFragmentPositionMissing)?;
                // Why check the end, the offset just past the last byte, and
                // not the start alone: the byte range of a sample is taken up
                // to its end, which must be a `u64`, so a sample whose last
                // byte is at `u64::MAX` is refused too.
                let offset = movie_fragment
                    .offset
                    .checked_add(location.offset)
                    .filter(|offset| offset.checked_add(u64::from(location.size)).is_some())
                    .ok_or(Error::FileOffsetOverflow {
                        data_origin_offset: movie_fragment.offset,
                        offset: location.offset,
                        size: location.size,
                    })?;
                Ok(SampleLocation {
                    container: match container {
                        MediaDataContainer::IdentifiedMediaData { .. } => {
                            MediaDataContainer::SameFile
                        }
                        MediaDataContainer::SameFile | MediaDataContainer::Resource => container,
                    },
                    data_origin: DataOrigin::File,
                    offset,
                    ..location
                })
            }
            (
                DataOrigin::IdentifiedMediaData,
                MediaDataContainer::IdentifiedMediaData { imda_identifier },
            ) => Err(Error::IdentifiedMediaDataNotRead { imda_identifier }),
            (DataOrigin::File, MediaDataContainer::IdentifiedMediaData { imda_identifier }) => {
                Err(Error::DataOriginMismatch {
                    sample_description_index,
                    imda_identifier: Some(imda_identifier),
                })
            }
            (
                DataOrigin::IdentifiedMediaData,
                MediaDataContainer::SameFile | MediaDataContainer::Resource,
            ) => Err(Error::DataOriginMismatch {
                sample_description_index,
                imda_identifier: None,
            }),
        }
    }
}
