//! Boxes at file level, each read into its value or skipped
//!
//! A box at file level is one no other box contains (ISO/IEC 14496-12
//! §3.1.14). What becomes of it follows from its type alone: the boxes whose
//! values the demuxer reads are gathered, and every other box is skipped
//! (§4.2.2).
//! Nothing is held between boxes: the order the boxes come in is for the
//! reader of their values to judge.

use isobmff_boxes::{
    FileTypeBox, IdentifiedMediaDataBox, MediaDataBox, MovieBox, MovieFragmentBox,
    MovieFragmentRandomAccessBox, SegmentIndexBox, SegmentTypeBox,
};
use isobmff_core::{BoxDecode, BoxDefinition, BoxType, RawBox};

use crate::error::DemuxError;

/// What becomes of a box at file level, as its type says
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Handling {
    /// Gathered whole, and read into its value with [`read`]
    Gather,
    /// Media data box, skipped
    MediaData,
    /// Any other box, skipped
    Other,
}

/// Value of a box at file level that is read off its payload
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Value {
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
}

/// Returns what becomes of a box at file level of `box_type`
pub(crate) fn handling(box_type: BoxType) -> Handling {
    if value_reader(box_type).is_some() {
        return Handling::Gather;
    }
    match box_type {
        MediaDataBox::BOX_TYPE | IdentifiedMediaDataBox::BOX_TYPE => Handling::MediaData,
        BoxType::Compact(_) | BoxType::Extended(_) => Handling::Other,
    }
}

/// Reads a box gathered whole into its value, or returns `None` for a box [`handling`] does not gather
///
/// # Errors
///
/// * [`DemuxError::Box`]: the payload does not read as the box its type
///   names. The failure names that box as the outermost of the path it was
///   reached through.
pub(crate) fn read(raw_box: RawBox<'_>) -> Result<Option<Value>, DemuxError> {
    value_reader(raw_box.header().box_type())
        .map(|read_payload| read_payload(raw_box.payload()))
        .transpose()
}

/// Reads the payload of a box into its value
type ReadPayload = fn(&[u8]) -> Result<Value, DemuxError>;

/// Returns what reads the payload of a box of `box_type` into its value, or `None` for a box with no value
fn value_reader(box_type: BoxType) -> Option<ReadPayload> {
    match box_type {
        FileTypeBox::BOX_TYPE => Some(|payload| decode(payload).map(Value::FileType)),
        SegmentTypeBox::BOX_TYPE => Some(|payload| decode(payload).map(Value::SegmentType)),
        MovieBox::BOX_TYPE => Some(|payload| decode(payload).map(Value::Movie)),
        MovieFragmentBox::BOX_TYPE => Some(|payload| decode(payload).map(Value::MovieFragment)),
        SegmentIndexBox::BOX_TYPE => Some(|payload| decode(payload).map(Value::SegmentIndex)),
        MovieFragmentRandomAccessBox::BOX_TYPE => {
            Some(|payload| decode(payload).map(Value::MovieFragmentRandomAccess))
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

        assert_eq!(read(raw_box), Ok(Some(Value::FileType(file_type))));
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
    type Mapped = (BoxType, u64, Option<Value>);

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
                    Handling::MediaData | Handling::Other => {
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
