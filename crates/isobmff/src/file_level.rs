//! Boxes at file level: the kind of each as its type says, and the value of each that has one
//!
//! A box at file level is one no other box contains (ISO/IEC 14496-12
//! §3.1.14). Its kind follows from its type alone ([`Kind::of`]): the
//! structure-data, which has a value ([`TopLevelBox::read`]), the media data,
//! which the structure-data locates samples in, and any other box (A.3).
//! Nothing is held between boxes: the order the boxes come in is for the
//! reader of their values to judge.
//!
//! These are the values a reader reads, not a report of what a reader did: a
//! [`Demuxer`](crate::Demuxer) reads each box at file level with this
//! module and reports it as a [`DemuxEvent`](crate::DemuxEvent), the
//! vocabulary of its own, with the byte range and the sample locations it
//! resolves.
//!
//! A reader of its own lists the boxes at file level with a
//! [`BoxReader`](crate::framing::BoxReader), gathering the structure-data
//! into its values and skipping the rest:
//!
//! ```
//! use isobmff::file_level::{Kind, TopLevelBox};
//! use isobmff::framing::{BoxEvent, BoxReader};
//!
//! let file: &[u8] = include_bytes!("../tests/data/movie.mp4");
//! let (mut reader, mut values) = (BoxReader::new(), Vec::new());
//! while let Some(wanted) = reader.wanted_input() {
//!     let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
//!     if input.is_empty() {
//!         break;
//!     }
//!     while let Some(event) = reader.handle_input(wanted.offset(), input)? {
//!         match event {
//!             // Gather the structure-data, and skip the other boxes
//!             BoxEvent::Header(header) => match Kind::of(header.box_type()) {
//!                 Kind::StructureData => reader.gather(1024 * 1024)?,
//!                 _ => reader.skip()?,
//!             },
//!             // Read the box just gathered into its value
//!             BoxEvent::End => {
//!                 if let Some(raw_box) = reader.gathered_box() {
//!                     values.extend(TopLevelBox::read(raw_box)?);
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//! }
//! assert!(matches!(values.as_slice(), [TopLevelBox::FileType(_), TopLevelBox::Movie(_)]));
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! A file whose track fragments take their `moof` as the base data offset is
//! written under a brand from `iso5` on (§8.8.7.1,
//! [`supports_default_base_is_moof`]).

#[cfg(feature = "compressed-boxes")]
use isobmff_boxes::OriginalFileTypeBox;
use isobmff_boxes::{
    FileTypeBox, IdentifiedMediaDataBox, MediaDataBox, MovieBox, MovieFragmentBox,
    MovieFragmentRandomAccessBox, SegmentIndexBox, SegmentTypeBox,
};
#[cfg(feature = "compressed-boxes")]
use isobmff_core::Boxes;
use isobmff_core::{BoxDecode, BoxDefinition, BoxType, RawBox};

use crate::error::ReadError;

/// Kind of a box at file level, as its type says, ISO/IEC 14496-12 A.3
///
/// The structure-data of a file is read into values; the media data is the
/// bytes of samples, which the structure-data locates by byte range.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Structure-data, with a value [`TopLevelBox::read`] reads, or, with the feature `compressed-boxes`, a compressed box that replaces one (§8.16)
    StructureData,
    /// Media data box (`mdat`, §8.1.1)
    MediaData,
    /// Identified media data box (`imda`, §8.1.4), whose `imda_identifier` the data references of samples name
    IdentifiedMediaData,
    /// Any other box
    Other,
}

impl Kind {
    /// Returns the kind of a box at file level of `box_type`
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::BoxType;
    /// use isobmff::file_level::Kind;
    ///
    /// assert_eq!(Kind::of(BoxType::compact(*b"moov")), Kind::StructureData);
    /// assert_eq!(Kind::of(BoxType::compact(*b"mdat")), Kind::MediaData);
    /// assert_eq!(Kind::of(BoxType::compact(*b"imda")), Kind::IdentifiedMediaData);
    /// assert_eq!(Kind::of(BoxType::compact(*b"free")), Kind::Other);
    /// ```
    #[must_use]
    pub fn of(box_type: BoxType) -> Self {
        #[cfg(feature = "compressed-boxes")]
        let compressed = crate::framing::replacement_type(box_type).is_some();
        #[cfg(not(feature = "compressed-boxes"))]
        let compressed = false;
        if value_reader(box_type).is_some() || compressed {
            return Self::StructureData;
        }
        match box_type {
            MediaDataBox::BOX_TYPE => Self::MediaData,
            IdentifiedMediaDataBox::BOX_TYPE => Self::IdentifiedMediaData,
            BoxType::Compact(_) | BoxType::Extended(_) => Self::Other,
        }
    }
}

/// Value of a box at file level that is read off its payload
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum TopLevelBox {
    /// `ftyp`, §5.2, or, with the feature `compressed-boxes`, the `ftyp` of the file before its box structure was transformed, which the innermost `otyp` holds, §8.16.5
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
}

impl TopLevelBox {
    /// Reads a box gathered whole into its value, or returns `None` for a box with no value
    ///
    /// A compressed box reads into `None`: [`read_payload`](Self::read_payload) reads the payload of the box it replaces.
    ///
    /// # Errors
    ///
    /// * [`ReadError::Box`]: the payload does not read as the box its type
    ///   names. The failure names that box as the outermost of the path it
    ///   was reached through.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::RawBox;
    /// use isobmff::file_level::TopLevelBox;
    ///
    /// let (ftyp, _) = RawBox::split_first(b"\0\0\0\x10ftypisom\0\0\x02\0")?;
    /// let Some(TopLevelBox::FileType(ftyp)) = TopLevelBox::read(ftyp)? else { return Err("no ftyp".into()) };
    /// assert_eq!(ftyp.major_brand.as_bytes(), b"isom");
    ///
    /// let (mdat, _) = RawBox::split_first(b"\0\0\0\x0amdatAB")?;
    /// assert_eq!(TopLevelBox::read(mdat)?, None);
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    pub fn read(raw_box: RawBox<'_>) -> Result<Option<Self>, ReadError> {
        Self::read_payload(raw_box.header().box_type(), raw_box.payload())
    }

    /// Reads `payload`, that of a box of `box_type`, into its value, or returns `None` for a box with no value
    ///
    /// # Errors
    ///
    /// As for [`read`](Self::read).
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff::BoxType;
    /// use isobmff::file_level::TopLevelBox;
    ///
    /// let value = TopLevelBox::read_payload(BoxType::compact(*b"ftyp"), b"isom\0\0\x02\0")?;
    /// assert!(matches!(value, Some(TopLevelBox::FileType(_))));
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    pub fn read_payload(box_type: BoxType, payload: &[u8]) -> Result<Option<Self>, ReadError> {
        value_reader(box_type)
            .map(|read_payload| read_payload(payload))
            .transpose()
    }
}

/// Reads the payload of a box into its value
type ReadPayload = fn(&[u8]) -> Result<TopLevelBox, ReadError>;

/// Returns what reads the payload of a box of `box_type` into its value, or `None` for a box with no value
fn value_reader(box_type: BoxType) -> Option<ReadPayload> {
    match box_type {
        FileTypeBox::BOX_TYPE => Some(|payload| decode(payload).map(TopLevelBox::FileType)),
        SegmentTypeBox::BOX_TYPE => Some(|payload| decode(payload).map(TopLevelBox::SegmentType)),
        MovieBox::BOX_TYPE => Some(|payload| decode(payload).map(TopLevelBox::Movie)),
        MovieFragmentBox::BOX_TYPE => {
            Some(|payload| decode(payload).map(TopLevelBox::MovieFragment))
        }
        SegmentIndexBox::BOX_TYPE => Some(|payload| decode(payload).map(TopLevelBox::SegmentIndex)),
        MovieFragmentRandomAccessBox::BOX_TYPE => {
            Some(|payload| decode(payload).map(TopLevelBox::MovieFragmentRandomAccess))
        }
        #[cfg(feature = "compressed-boxes")]
        OriginalFileTypeBox::BOX_TYPE => {
            Some(|payload| innermost_file_type(payload).map(TopLevelBox::FileType))
        }
        BoxType::Compact(_) | BoxType::Extended(_) => None,
    }
}

/// Returns the `ftyp` of the innermost `otyp` the `otyp` of payload `payload` holds, or of that `otyp` itself if it holds none, §8.16.5
///
/// A file transformed more than once nests an `otyp` in the one before, the
/// innermost holding the `ftyp` of the file as it was first written.
///
/// # Errors
///
/// * [`ReadError::Box`]: the innermost `otyp` does not read.
#[cfg(feature = "compressed-boxes")]
fn innermost_file_type(payload: &[u8]) -> Result<FileTypeBox, ReadError> {
    // Why not decode each `otyp` and look among its other boxes: each level
    // would copy what it holds, the nested ones with it. Why not recurse: the
    // input alone bounds the depth of nesting.
    let mut innermost = payload;
    while let Some(nested) = Boxes::new(innermost)
        .map_while(Result::ok)
        .find(|child| child.header().box_type() == OriginalFileTypeBox::BOX_TYPE)
    {
        innermost = nested.payload();
    }
    decode::<OriginalFileTypeBox>(innermost).map(|otyp| otyp.ftyp)
}

/// Reads `payload` as the box `Decoded`, naming that box on the path of a failure
fn decode<Decoded: BoxDecode + BoxDefinition>(payload: &[u8]) -> Result<Decoded, ReadError> {
    Decoded::decode_payload(payload).map_err(|error| ReadError::Box {
        error: error.in_container(Decoded::BOX_TYPE),
    })
}

/// Brands under which a track fragment may take the first byte of its `moof` as its base data offset, ISO/IEC 14496-12 §8.8.7.1 and Annex E
const DEFAULT_BASE_IS_MOOF_BRANDS: [[u8; 4]; 9] = [
    *b"iso5", *b"iso6", *b"iso7", *b"iso8", *b"iso9", *b"isoa", *b"isob", *b"isoc", *b"isod",
];

/// Returns whether the brands of `file_type` name one of `iso5` to `isod`, ISO/IEC 14496-12 §8.8.7.1
///
/// Under these brands a reader takes the first byte of a `moof` as the base
/// data offset of a track fragment that says so (`default-base-is-moof`).
/// A fragmented movie file, or a segment, whose track fragments take the
/// `moof` as their base data offset declares one of these brands as its major
/// brand or a compatible brand: the movie fragments a
/// [`Muxer`](crate::Muxer) and a
/// [`MovieFragmentWriter`](crate::structure_data::MovieFragmentWriter) write
/// do.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::FileTypeBox;
/// use isobmff::{FourCc, supports_default_base_is_moof};
///
/// let mut ftyp = FileTypeBox::default();
/// ftyp.major_brand = FourCc::new(*b"mp42");
/// assert!(!supports_default_base_is_moof(&ftyp));
/// ftyp.compatible_brands.push(FourCc::new(*b"iso6"));
/// assert!(supports_default_base_is_moof(&ftyp));
/// ```
#[must_use]
pub fn supports_default_base_is_moof(file_type: &FileTypeBox) -> bool {
    core::iter::once(&file_type.major_brand)
        .chain(&file_type.compatible_brands)
        .any(|brand| DEFAULT_BASE_IS_MOOF_BRANDS.contains(brand.as_bytes()))
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_boxes::MovieHeaderBox;
    use isobmff_core::{Error, FourCc};
    use isobmff_test_support::{
        ConformanceFile, conformance_files, gpac_children, gpac_frame, gpac_type, tally,
    };

    use super::*;
    use crate::framing::{BoxEvent, BoxReader};

    #[test]
    fn a_box_with_a_value_reads_into_it() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x18ftypisom\0\0\x02\0isomiso2").unwrap();
        let mut file_type = FileTypeBox::default();
        file_type.major_brand = FourCc::new(*b"isom");
        file_type.minor_version = 0x200;
        file_type.compatible_brands = vec![FourCc::new(*b"isom"), FourCc::new(*b"iso2")];

        assert_eq!(
            TopLevelBox::read(raw_box),
            Ok(Some(TopLevelBox::FileType(file_type)))
        );
    }

    #[test]
    fn a_media_data_box_reads_into_no_value() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x10mdatpayload!").unwrap();

        assert_eq!(TopLevelBox::read(raw_box), Ok(None));
    }

    #[test]
    fn a_payload_that_does_not_read_fails_naming_its_box_outermost_on_the_path() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x14moov\0\0\0\x0cmvhd\0\0\0\0").unwrap();

        assert_eq!(
            TopLevelBox::read(raw_box),
            Err(ReadError::Box {
                error: Error::truncated_payload(8, 4)
                    .in_container(MovieHeaderBox::BOX_TYPE)
                    .in_container(MovieBox::BOX_TYPE)
            })
        );
    }

    /// Box at file level as the mapping leaves it: its type, its total, and whether a value was read off it
    type Mapped = (BoxType, u64, Option<TopLevelBox>);

    /// Reads the boxes at file level of `file`, answering each header as [`Kind::of`] says, and reading each gathered box with [`TopLevelBox::read`]
    fn map_file_level(file: &[u8]) -> Vec<Mapped> {
        let file_length = u64::try_from(file.len()).unwrap();
        let mut reader = BoxReader::new();
        let mut mapped = Vec::new();
        let mut map = |reader: &mut BoxReader, event| match event {
            BoxEvent::Header(header) => {
                let byte_range = reader.open_box().unwrap();
                let total = byte_range
                    .length()
                    .unwrap_or(file_length.saturating_sub(byte_range.offset()));
                match Kind::of(header.box_type()) {
                    Kind::StructureData => reader.gather(u64::MAX).unwrap(),
                    Kind::MediaData | Kind::IdentifiedMediaData | Kind::Other => {
                        reader.skip().unwrap();
                        mapped.push((header.box_type(), total, None));
                    }
                }
            }
            BoxEvent::End => {
                if let Some(raw_box) = reader.gathered_box() {
                    let total = raw_box
                        .header()
                        .encoded_len()
                        .saturating_add(u64::try_from(raw_box.payload().len()).unwrap());
                    mapped.push((
                        raw_box.header().box_type(),
                        total,
                        TopLevelBox::read(raw_box).unwrap(),
                    ));
                }
            }
            BoxEvent::Payload(_) => {}
        };
        while let Some(wanted) = reader.wanted_input() {
            let input = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            if input.is_empty() {
                break;
            }
            while let Some(event) = reader.handle_input(wanted.offset(), input).unwrap() {
                map(&mut reader, event);
            }
        }
        if let Some(event) = reader.finish().unwrap() {
            map(&mut reader, event);
        }

        mapped
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_boxes_at_file_level_map_as_gpac_reports_them() {
        let mut metas = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let mapped = map_file_level(&bytes);
            let theirs = tally(
                gpac_children(report.get("IsoMediaFile").unwrap())
                    .into_iter()
                    .map(gpac_frame),
            );
            let ours =
                tally(mapped.iter().map(|(box_type, total, _)| {
                    (gpac_type(box_type.four_cc().as_bytes()), *total)
                }));
            assert_eq!(ours, theirs, "{name}");
            for (box_type, _, value) in &mapped {
                assert_eq!(
                    value.is_some(),
                    Kind::of(*box_type) == Kind::StructureData,
                    "{name}: {box_type}"
                );
                if *box_type == BoxType::compact(*b"meta") {
                    assert_eq!(Kind::of(*box_type), Kind::Other, "{name}");
                    metas.push(name.clone());
                }
            }
        }
        assert_eq!(metas, ["05_bifs_video_protected_v2.mp4", "12_metas_v2.mp4"]);
    }
}
