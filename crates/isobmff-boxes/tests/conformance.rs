//! Boxes read and written back on the files of MPEG's `FileFormatConformance`, and read as GPAC reports them
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests.

#[cfg(test)]
mod tests {
    use isobmff_boxes::{
        ExtendedTypeBox, FileTypeBox, Free, FreeSpaceBox, IdentifiedMediaDataBox, MediaDataBox,
        OriginalFileTypeBox, ProgressiveDownloadInfoBox, SegmentTypeBox, Skip,
    };
    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, FourCc, RawBox,
    };
    use isobmff_test_support::serde_json::Value;
    use isobmff_test_support::{ConformanceFile, conformance_files, gpac_children};

    /// Reads `payload` as a `Box` and returns the payload it writes back
    ///
    /// The payload is compared rather than the whole box: the size form a box
    /// was framed under, such as a size of zero running to the end of the
    /// file, is not a value the box holds.
    fn written_back<Box: BoxDecode + BoxEncode>(payload: &[u8]) -> Result<Vec<u8>, Error> {
        let read = Box::decode_payload(payload)?;
        let mut written = vec![0; usize::try_from(read.payload_len()).unwrap()];
        read.encode_payload(&mut written)?;

        Ok(written)
    }

    /// Reads a payload as one box and writes it back, as [`written_back`] does for a type
    type WrittenBack = fn(&[u8]) -> Result<Vec<u8>, Error>;

    /// Box types that can lie at the top level of a file and have a type here, each with how it is read and written back
    const TOP_LEVEL: [(BoxType, WrittenBack); 9] = [
        (FileTypeBox::BOX_TYPE, written_back::<FileTypeBox>),
        (SegmentTypeBox::BOX_TYPE, written_back::<SegmentTypeBox>),
        (ExtendedTypeBox::BOX_TYPE, written_back::<ExtendedTypeBox>),
        (
            OriginalFileTypeBox::BOX_TYPE,
            written_back::<OriginalFileTypeBox>,
        ),
        (
            FreeSpaceBox::<Free>::BOX_TYPE,
            written_back::<FreeSpaceBox<Free>>,
        ),
        (
            FreeSpaceBox::<Skip>::BOX_TYPE,
            written_back::<FreeSpaceBox<Skip>>,
        ),
        (MediaDataBox::BOX_TYPE, written_back::<MediaDataBox>),
        (
            IdentifiedMediaDataBox::BOX_TYPE,
            written_back::<IdentifiedMediaDataBox>,
        ),
        (
            ProgressiveDownloadInfoBox::BOX_TYPE,
            written_back::<ProgressiveDownloadInfoBox>,
        ),
    ];

    /// Returns the top-level boxes of a file
    fn top_level(bytes: &[u8]) -> Vec<RawBox<'_>> {
        Boxes::new(bytes).collect::<Result<_, _>>().unwrap()
    }

    /// Reads every box of `boxes` that is a `Box`, in the order they lie
    fn read_all<Box: BoxDecode + BoxDefinition>(boxes: &[RawBox<'_>]) -> Vec<Box> {
        boxes
            .iter()
            .filter(|framed| framed.header().box_type() == Box::BOX_TYPE)
            .map(|framed| Box::decode_payload(framed.payload()).unwrap())
            .collect()
    }

    /// Returns the `GeneralTypeBox` fields GPAC reports for `node`
    fn gpac_general_type(node: &Value) -> (FourCc, u32, Vec<FourCc>) {
        let brand = |node: &Value, field: &str| -> FourCc {
            let spelled = node.get(field).unwrap().as_str().unwrap();
            FourCc::new(spelled.as_bytes().try_into().unwrap())
        };
        let entries = match node.get("BrandEntry") {
            Some(Value::Array(entries)) => entries.iter().collect(),
            Some(entry) => vec![entry],
            None => Vec::new(),
        };
        let minor_version = node.get("@MinorVersion").unwrap().as_str().unwrap();

        (
            brand(node, "@MajorBrand"),
            minor_version.parse().unwrap(),
            entries
                .into_iter()
                .map(|entry| brand(entry, "@AlternateBrand"))
                .collect(),
        )
    }

    /// Returns the top-level boxes GPAC reports of the four-character code `box_type`
    fn gpac_top_level<'report>(report: &'report Value, box_type: &str) -> Vec<&'report Value> {
        gpac_children(report.get("IsoMediaFile").unwrap())
            .into_iter()
            .filter(|node| node.get("@Type").and_then(Value::as_str) == Some(box_type))
            .collect()
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn every_top_level_box_with_a_type_writes_back_the_payload_it_was_read_from() {
        let mut differences = Vec::new();
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            for framed in top_level(&bytes) {
                let box_type = framed.header().box_type();
                let Some((_, written_back)) =
                    TOP_LEVEL.iter().find(|(known, _)| *known == box_type)
                else {
                    continue;
                };
                match written_back(framed.payload()) {
                    Ok(written) if written == framed.payload() => {}
                    Ok(_) => {
                        differences.push(format!("{name}: {box_type} writes back other bytes"))
                    }
                    Err(error) => differences.push(format!("{name}: {box_type}: {error}")),
                }
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn ftyp_and_styp_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let boxes = top_level(&bytes);
            let ours = (
                read_all::<FileTypeBox>(&boxes),
                read_all::<SegmentTypeBox>(&boxes),
            );
            let theirs = (
                gpac_top_level(&report, "ftyp")
                    .into_iter()
                    .map(|node| {
                        let mut file_type = FileTypeBox::default();
                        (
                            file_type.major_brand,
                            file_type.minor_version,
                            file_type.compatible_brands,
                        ) = gpac_general_type(node);
                        file_type
                    })
                    .collect(),
                gpac_top_level(&report, "styp")
                    .into_iter()
                    .map(|node| {
                        let mut segment_type = SegmentTypeBox::default();
                        (
                            segment_type.major_brand,
                            segment_type.minor_version,
                            segment_type.compatible_brands,
                        ) = gpac_general_type(node);
                        segment_type
                    })
                    .collect(),
            );
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }
}
