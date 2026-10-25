//! Framing checked against the box structure GPAC reports for the files of MPEG's `FileFormatConformance`
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::{env, fs};

    use isobmff_core::{BoxType, Boxes, RawBox};
    use serde_json::Value;

    /// Box types whose payload is a sequence of boxes and nothing else
    const CONTAINERS: [[u8; 4]; 12] = [
        *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"dinf", *b"edts", *b"mvex", *b"moof",
        *b"traf", *b"mfra", *b"udta",
    ];

    /// Boxes a file holds that GPAC leaves out of the structure it reports, by file, container path, type and size
    ///
    /// GPAC reports one `strk` of the two the `udta` of this file holds, whose
    /// sizes add up to the `udta` with the one GPAC leaves out.
    const GPAC_OMISSIONS: [(&str, &str, &str, u64); 1] =
        [("sg-tl-st.mp4", "/moov/trak/udta", "strk", 58)];

    /// Type and total of a box, as GPAC spells them
    type Frame = (String, u64);

    /// Returns the type of a box as GPAC spells it: printable ASCII as it is, any other byte in hexadecimal
    fn gpac_type(box_type: BoxType) -> String {
        box_type
            .four_cc()
            .as_bytes()
            .iter()
            .map(|byte| {
                if byte.is_ascii_graphic() || *byte == b' ' {
                    char::from(*byte).to_string()
                } else {
                    format!("{byte:02X}")
                }
            })
            .collect()
    }

    /// Returns the boxes GPAC reports directly inside `node`
    fn gpac_children(node: &Value) -> Vec<&Value> {
        node.as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| !key.starts_with('@'))
            .flat_map(|(_, value)| {
                if let Value::Array(items) = value {
                    items.iter().collect()
                } else {
                    vec![value]
                }
            })
            .filter(|item| item.get("@Type").is_some() && item.get("@Size").is_some())
            .collect()
    }

    /// Returns the type and total GPAC reports for a box
    fn gpac_frame(node: &Value) -> Frame {
        let box_type = node
            .get("@Type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let size = node
            .get("@Size")
            .and_then(Value::as_str)
            .and_then(|size| size.parse().ok())
            .unwrap_or_default();

        (box_type, size)
    }

    /// Returns how many of each box `frames` holds
    fn tally(frames: impl IntoIterator<Item = Frame>) -> BTreeMap<Frame, usize> {
        frames
            .into_iter()
            .fold(BTreeMap::new(), |mut tally, frame| {
                let count = tally.entry(frame).or_insert(0_usize);
                *count = count.saturating_add(1);
                tally
            })
    }

    /// Records where the boxes in `payload` differ from what GPAC reports in `node`, then descends into containers
    fn compare(
        file: &str,
        path: &str,
        payload: &[u8],
        node: &Value,
        differences: &mut Vec<String>,
    ) {
        let framed = match Boxes::new(payload).collect::<Result<Vec<RawBox<'_>>, _>>() {
            Ok(framed) => framed,
            Err(error) => {
                differences.push(format!("{file}{path}: {error}"));
                return;
            }
        };

        let total_of = |child: &RawBox<'_>| {
            child
                .header()
                .encoded_len()
                .checked_add(child.payload().len())
                .and_then(|total| u64::try_from(total).ok())
                .unwrap_or(u64::MAX)
        };
        let ours = tally(
            framed
                .iter()
                .map(|child| (gpac_type(child.header().box_type()), total_of(child))),
        );
        let omitted = GPAC_OMISSIONS
            .iter()
            .filter(|(omitted_file, omitted_path, _, _)| {
                *omitted_file == file && *omitted_path == path
            })
            .map(|(_, _, box_type, size)| ((*box_type).to_owned(), *size));
        let gpac_nodes = gpac_children(node);
        let theirs = tally(
            gpac_nodes
                .iter()
                .map(|child| gpac_frame(child))
                .chain(omitted),
        );
        if ours != theirs {
            differences.push(format!(
                "{file}{path}: framed {ours:?}, GPAC reports {theirs:?}"
            ));
        }

        let mut unmatched = gpac_nodes;
        for child in &framed {
            let box_type = child.header().box_type();
            if !CONTAINERS.contains(box_type.four_cc().as_bytes()) {
                continue;
            }

            let frame = (gpac_type(box_type), total_of(child));
            if let Some(position) = unmatched.iter().position(|node| gpac_frame(node) == frame) {
                let node = unmatched.remove(position);
                compare(
                    file,
                    &format!("{path}/{box_type}"),
                    child.payload(),
                    node,
                    differences,
                );
            }
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn every_box_of_a_conformance_file_frames_as_gpac_reports_it() {
        let directory = PathBuf::from(env::var_os("ISOBMFF_CONFORMANCE_DIR").expect(
            "ISOBMFF_CONFORMANCE_DIR names the directory scripts/fetch-conformance.sh fetched into",
        ));
        let mut files = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "mp4"))
            .collect::<Vec<_>>();
        files.sort();
        assert!(
            !files.is_empty(),
            "{} holds no conformance file",
            directory.display()
        );

        let mut differences = Vec::new();
        for file in files {
            let name = file.file_name().unwrap().to_str().unwrap().to_owned();
            let stem = file.file_stem().unwrap().to_str().unwrap();
            let bytes = fs::read(&file).unwrap();
            let report: Value = serde_json::from_slice(
                &fs::read(file.with_file_name(format!("{stem}_gpac.json"))).unwrap(),
            )
            .unwrap();

            compare(
                &name,
                "",
                &bytes,
                report.get("IsoMediaFile").unwrap(),
                &mut differences,
            );
        }

        assert_eq!(differences, Vec::<String>::new());
    }
}
