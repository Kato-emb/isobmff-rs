//! Boxes at file level, each read into its value or reported by where it lies
//!
//! A box at file level is one no other box contains. The structure-data boxes
//! among them are read into their values, the media data boxes are reported as
//! the byte range they span, and the rest by their header and byte range alone.
//! What a box is follows from its header, so [`FileLevelBox::from_header`]
//! says from a header whether its payload is to be gathered and read with
//! [`FileLevelBox::read`]. Neither holds anything between boxes: the order the
//! boxes come in is for the reader of their values to judge.

use isobmff_boxes::{
    FileTypeBox, IdentifiedMediaDataBox, MediaDataBox, MovieBox, MovieFragmentBox,
    MovieFragmentRandomAccessBox, SegmentIndexBox, SegmentTypeBox,
};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader, BoxType, RawBox};

use crate::byte_range::ByteRange;
use crate::error::DemuxError;

/// Box at file level, read into its value or reported by where it lies, ISO/IEC 14496-12 §3.1.14
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum FileLevelBox {
    /// `ftyp`, §5.2
    FileType(FileTypeBox),
    /// `styp`, §8.14.2
    SegmentType(SegmentTypeBox),
    /// `moov`, §8.2.1
    Movie(MovieBox),
    /// `moof`, §8.8.4
    MovieFragment(MovieFragmentBox),
    /// `sidx`, §8.14.3
    SegmentIndex(SegmentIndexBox),
    /// `mfra`, §8.8.9
    MovieFragmentRandomAccess(MovieFragmentRandomAccessBox),
    /// `mdat`, §8.1.1, by the byte range of the whole box
    MediaData(ByteRange),
    /// `imda`, §8.1.4, by the byte range of the whole box
    IdentifiedMediaData(ByteRange),
    /// Any other box, by its header and the byte range of the whole box
    ///
    /// A reader skips a box it does not recognize (§4.2.2); one at file level is
    /// reported so that the caller still learns where it lies.
    #[non_exhaustive]
    Other {
        /// Header of the box
        header: BoxHeader,
        /// Byte range of the whole box
        byte_range: ByteRange,
    },
}

impl FileLevelBox {
    /// Returns the box with `header`, which lies at `byte_range`, as its header reports it, or `None` when its value is read off its payload
    ///
    /// A box this returns `None` for is to be gathered whole and read with
    /// [`read`](Self::read). Of the boxes it returns, the payload of
    /// [`MediaData`](Self::MediaData) and
    /// [`IdentifiedMediaData`](Self::IdentifiedMediaData) is for the caller to
    /// stream or skip, and that of [`Other`](Self::Other) is skipped.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::file_level::FileLevelBox;
    /// use isobmff::{BoxHeader, BoxType, ByteRange};
    ///
    /// // A movie is gathered and read into its value
    /// let movie = BoxHeader::with_payload_len(BoxType::compact(*b"moov"), 100).unwrap();
    /// assert_eq!(FileLevelBox::from_header(movie, ByteRange::new(32, Some(108))), None);
    ///
    /// // Media data is reported by where it lies
    /// let media_data = BoxHeader::with_payload_len(BoxType::compact(*b"mdat"), 4000).unwrap();
    /// let byte_range = ByteRange::new(140, Some(4008));
    /// assert_eq!(
    ///     FileLevelBox::from_header(media_data, byte_range),
    ///     Some(FileLevelBox::MediaData(byte_range))
    /// );
    /// ```
    #[must_use]
    pub fn from_header(header: BoxHeader, byte_range: ByteRange) -> Option<Self> {
        value_reader(header.box_type())
            .is_none()
            .then(|| Self::reported(header, byte_range))
    }

    /// Reads a box gathered whole, which lies at `byte_range`, into its value
    ///
    /// A box whose value [`from_header`](Self::from_header) reports reads as
    /// it reports it, so gathering a box never loses it.
    ///
    /// # Errors
    ///
    /// * [`DemuxError::Box`]: the payload does not read as the box its type
    ///   names. The failure names that box as the outermost of the path it was
    ///   reached through.
    pub fn read(raw_box: RawBox<'_>, byte_range: ByteRange) -> Result<Self, DemuxError> {
        let header = raw_box.header();
        match value_reader(header.box_type()) {
            Some(read_payload) => read_payload(raw_box.payload()),
            None => Ok(Self::reported(header, byte_range)),
        }
    }

    /// Returns the box with `header`, which lies at `byte_range` and has no value read off its payload
    fn reported(header: BoxHeader, byte_range: ByteRange) -> Self {
        match header.box_type() {
            MediaDataBox::BOX_TYPE => Self::MediaData(byte_range),
            IdentifiedMediaDataBox::BOX_TYPE => Self::IdentifiedMediaData(byte_range),
            BoxType::Compact(_) | BoxType::Extended(_) => Self::Other { header, byte_range },
        }
    }
}

/// Reads the payload of a box into its value
type ReadPayload = fn(&[u8]) -> Result<FileLevelBox, DemuxError>;

/// Returns what reads the payload of a box of `box_type` into its value, or `None` for a box whose value is not read off its payload
fn value_reader(box_type: BoxType) -> Option<ReadPayload> {
    match box_type {
        FileTypeBox::BOX_TYPE => Some(|payload| decode(payload).map(FileLevelBox::FileType)),
        SegmentTypeBox::BOX_TYPE => Some(|payload| decode(payload).map(FileLevelBox::SegmentType)),
        MovieBox::BOX_TYPE => Some(|payload| decode(payload).map(FileLevelBox::Movie)),
        MovieFragmentBox::BOX_TYPE => {
            Some(|payload| decode(payload).map(FileLevelBox::MovieFragment))
        }
        SegmentIndexBox::BOX_TYPE => {
            Some(|payload| decode(payload).map(FileLevelBox::SegmentIndex))
        }
        MovieFragmentRandomAccessBox::BOX_TYPE => {
            Some(|payload| decode(payload).map(FileLevelBox::MovieFragmentRandomAccess))
        }
        BoxType::Compact(_) | BoxType::Extended(_) => None,
    }
}

/// Reads `payload` as the box `Decoded`, naming that box on the path of a failure
fn decode<Decoded: BoxDecode + BoxDefinition>(payload: &[u8]) -> Result<Decoded, DemuxError> {
    Decoded::decode_payload(payload).map_err(|error| DemuxError::Box {
        error: error.in_container(Decoded::BOX_TYPE),
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::MovieHeaderBox;
    use isobmff_core::{Error, FourCc};

    use super::*;

    fn header(box_type: BoxType) -> BoxHeader {
        BoxHeader::with_payload_len(box_type, 8).unwrap()
    }

    const BYTE_RANGE: ByteRange = ByteRange::new(64, Some(16));

    #[test]
    fn the_structure_data_boxes_are_to_be_gathered_and_read_into_their_values() {
        for box_type in [
            FileTypeBox::BOX_TYPE,
            SegmentTypeBox::BOX_TYPE,
            MovieBox::BOX_TYPE,
            MovieFragmentBox::BOX_TYPE,
            SegmentIndexBox::BOX_TYPE,
            MovieFragmentRandomAccessBox::BOX_TYPE,
        ] {
            assert_eq!(
                FileLevelBox::from_header(header(box_type), BYTE_RANGE),
                None,
                "{box_type}"
            );
        }
    }

    #[test]
    fn the_media_data_boxes_are_reported_by_the_byte_range_they_span() {
        assert_eq!(
            FileLevelBox::from_header(header(MediaDataBox::BOX_TYPE), BYTE_RANGE),
            Some(FileLevelBox::MediaData(BYTE_RANGE))
        );
        assert_eq!(
            FileLevelBox::from_header(header(IdentifiedMediaDataBox::BOX_TYPE), BYTE_RANGE),
            Some(FileLevelBox::IdentifiedMediaData(BYTE_RANGE))
        );
    }

    #[test]
    fn any_other_box_is_reported_by_its_header_and_byte_range() {
        for box_type in [
            BoxType::compact(*b"free"),
            BoxType::compact(*b"meta"),
            BoxType::compact(*b"vndr"),
            BoxType::extended([0xab; 16]),
        ] {
            let header = header(box_type);
            assert_eq!(
                FileLevelBox::from_header(header, BYTE_RANGE),
                Some(FileLevelBox::Other {
                    header,
                    byte_range: BYTE_RANGE
                }),
                "{box_type}"
            );
        }
    }

    #[test]
    fn a_gathered_box_reads_into_its_value() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2").unwrap();
        let mut file_type = FileTypeBox::default();
        file_type.major_brand = FourCc::new(*b"isom");
        file_type.minor_version = 0x200;
        file_type.compatible_brands = vec![FourCc::new(*b"isom"), FourCc::new(*b"iso2")];

        assert_eq!(
            FileLevelBox::read(raw_box, BYTE_RANGE),
            Ok(FileLevelBox::FileType(file_type))
        );
    }

    #[test]
    fn a_gathered_box_with_no_value_reads_as_its_header_reports_it() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x10mdatpayload!").unwrap();

        assert_eq!(
            FileLevelBox::read(raw_box, BYTE_RANGE),
            Ok(FileLevelBox::MediaData(BYTE_RANGE))
        );
    }

    #[test]
    fn a_payload_that_does_not_read_fails_naming_its_box_outermost_on_the_path() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x14moov\0\0\0\x0cmvhd\0\0\0\0").unwrap();

        assert_eq!(
            FileLevelBox::read(raw_box, BYTE_RANGE),
            Err(DemuxError::Box {
                error: Error::truncated_payload(8, 4)
                    .in_container(MovieHeaderBox::BOX_TYPE)
                    .in_container(MovieBox::BOX_TYPE)
            })
        );
    }
}
