//! Boxes at file level: what becomes of each as its type says, and the value of each that has one
//!
//! A box at file level is one no other box contains (ISO/IEC 14496-12
//! §3.1.14). What becomes of it follows from its type alone ([`handling`]),
//! and the boxes gathered whole read into their values ([`read`]). Nothing is
//! held between boxes: the order the boxes come in is for the reader of their
//! values to judge.
//!
//! These are the values a reader reads, not a report of what a reader did: a
//! [`Demuxer`](crate::Demuxer) reads each box at file level with this
//! module and reports it as a [`DemuxEvent`](crate::DemuxEvent), the
//! vocabulary of its own, with the byte range and the sample locations it
//! resolves.
//!
//! A reader of its own lists the boxes at file level with a
//! [`BoxReader`](crate::framing::BoxReader), answering each header as
//! [`handling`] says:
//!
//! ```
//! use isobmff::file_level::{self, Handling, TopLevelBox};
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
//!             // Gather the boxes that have a value, and skip the others
//!             BoxEvent::Header(header) => {
//!                 match file_level::handling(header.box_type()) {
//!                     Handling::Gather => reader.gather(1024 * 1024)?,
//!                     _ => reader.skip()?,
//!                 }
//!             }
//!             // Read the box just gathered into its value
//!             BoxEvent::End => {
//!                 if let Some(raw_box) = reader.gathered_box() {
//!                     values.extend(file_level::read(raw_box)?);
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

use crate::error::DemuxError;

/// What becomes of a box at file level, as its type says
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handling {
    /// Structure-data, or a compressed box (§8.16): gathered whole, and read into its value with [`read`]
    Gather,
    /// Media data box (`mdat`, §8.1.1): skipped
    MediaData,
    /// Identified media data box (`imda`, §8.1.4): its `imda_identifier` read, and the rest skipped
    IdentifiedMediaData,
    /// Any other box: skipped
    Other,
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

/// Returns what becomes of a box at file level of `box_type`
///
/// # Examples
///
/// ```
/// use isobmff::BoxType;
/// use isobmff::file_level::{Handling, handling};
///
/// assert_eq!(handling(BoxType::compact(*b"moov")), Handling::Gather);
/// assert_eq!(handling(BoxType::compact(*b"mdat")), Handling::MediaData);
/// assert_eq!(handling(BoxType::compact(*b"imda")), Handling::IdentifiedMediaData);
/// assert_eq!(handling(BoxType::compact(*b"free")), Handling::Other);
/// ```
#[must_use]
pub fn handling(box_type: BoxType) -> Handling {
    if value_reader(box_type).is_some() || is_compressed(box_type) {
        return Handling::Gather;
    }
    match box_type {
        MediaDataBox::BOX_TYPE => Handling::MediaData,
        IdentifiedMediaDataBox::BOX_TYPE => Handling::IdentifiedMediaData,
        BoxType::Compact(_) | BoxType::Extended(_) => Handling::Other,
    }
}

/// Returns whether a box of `box_type` is a compressed box, which is gathered and read back into the box it replaces (§8.16)
///
/// Without the feature `compressed-boxes`, no box is.
///
/// # Examples
///
/// ```
/// use isobmff::BoxType;
/// use isobmff::file_level::is_compressed;
///
/// assert_eq!(is_compressed(BoxType::compact(*b"!mov")), cfg!(feature = "compressed-boxes"));
/// assert!(!is_compressed(BoxType::compact(*b"moov")));
/// ```
#[must_use]
pub fn is_compressed(box_type: BoxType) -> bool {
    #[cfg(feature = "compressed-boxes")]
    return crate::framing::replacement_type(box_type).is_some();
    #[cfg(not(feature = "compressed-boxes"))]
    return {
        let _ = box_type;
        false
    };
}

/// Reads a box gathered whole into its value, or returns `None` for a box [`handling`] does not gather
///
/// A compressed box reads into `None`: [`read_payload`] reads the payload of the box it replaces.
///
/// # Errors
///
/// * [`DemuxError::Box`]: the payload does not read as the box its type
///   names. The failure names that box as the outermost of the path it was
///   reached through.
///
/// # Examples
///
/// ```
/// use isobmff::RawBox;
/// use isobmff::file_level::{TopLevelBox, read};
///
/// let (ftyp, _) = RawBox::split_first(b"\0\0\0\x10ftypisom\0\0\x02\0")?;
/// let Some(TopLevelBox::FileType(ftyp)) = read(ftyp)? else { return Err("no ftyp".into()) };
/// assert_eq!(ftyp.major_brand.as_bytes(), b"isom");
///
/// let (mdat, _) = RawBox::split_first(b"\0\0\0\x0amdatAB")?;
/// assert_eq!(read(mdat)?, None);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
pub fn read(raw_box: RawBox<'_>) -> Result<Option<TopLevelBox>, DemuxError> {
    read_payload(raw_box.header().box_type(), raw_box.payload())
}

/// Reads `payload`, that of a box of `box_type`, into its value, or returns `None` for a box [`handling`] does not gather
///
/// # Errors
///
/// As for [`read`].
///
/// # Examples
///
/// ```
/// use isobmff::BoxType;
/// use isobmff::file_level::{TopLevelBox, read_payload};
///
/// let value = read_payload(BoxType::compact(*b"ftyp"), b"isom\0\0\x02\0")?;
/// assert!(matches!(value, Some(TopLevelBox::FileType(_))));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
pub fn read_payload(box_type: BoxType, payload: &[u8]) -> Result<Option<TopLevelBox>, DemuxError> {
    value_reader(box_type)
        .map(|read_payload| read_payload(payload))
        .transpose()
}

/// Reads the payload of a box into its value
type ReadPayload = fn(&[u8]) -> Result<TopLevelBox, DemuxError>;

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
/// * [`DemuxError::Box`]: the innermost `otyp` does not read.
#[cfg(feature = "compressed-boxes")]
fn innermost_file_type(payload: &[u8]) -> Result<FileTypeBox, DemuxError> {
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
fn decode<Decoded: BoxDecode + BoxDefinition>(payload: &[u8]) -> Result<Decoded, DemuxError> {
    Decoded::decode_payload(payload).map_err(|error| DemuxError::Box {
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

        assert_eq!(read(raw_box), Ok(Some(TopLevelBox::FileType(file_type))));
    }

    #[test]
    fn a_media_data_box_reads_into_no_value() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x10mdatpayload!").unwrap();

        assert_eq!(read(raw_box), Ok(None));
    }

    #[test]
    fn a_payload_that_does_not_read_fails_naming_its_box_outermost_on_the_path() {
        let (raw_box, _) = RawBox::split_first(b"\0\0\0\x14moov\0\0\0\x0cmvhd\0\0\0\0").unwrap();

        assert_eq!(
            read(raw_box),
            Err(DemuxError::Box {
                error: Error::truncated_payload(8, 4)
                    .in_container(MovieHeaderBox::BOX_TYPE)
                    .in_container(MovieBox::BOX_TYPE)
            })
        );
    }

    /// Box at file level as the mapping leaves it: its type, its total, and whether a value was read off it
    type Mapped = (BoxType, u64, Option<TopLevelBox>);

    /// Reads the boxes at file level of `file`, answering each header as [`handling`] says, and reading each gathered box with [`read`]
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
                match handling(header.box_type()) {
                    Handling::Gather => reader.gather(u64::MAX).unwrap(),
                    Handling::MediaData | Handling::IdentifiedMediaData | Handling::Other => {
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
                    mapped.push((raw_box.header().box_type(), total, read(raw_box).unwrap()));
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
                    handling(*box_type) == Handling::Gather,
                    "{name}: {box_type}"
                );
                if *box_type == BoxType::compact(*b"meta") {
                    assert_eq!(handling(*box_type), Handling::Other, "{name}");
                    metas.push(name.clone());
                }
            }
        }
        assert_eq!(metas, ["05_bifs_video_protected_v2.mp4", "12_metas_v2.mp4"]);
    }
}
