//! Files of compressed boxes (ISO/IEC 14496-12 §8.16) read by the demuxer, checked on the files of MPEG's `FileFormatConformance` under consideration
//!
//! The brands the demuxer reports are those of the `ftyp` GPAC finds in the
//! `otyp`, and the samples the demuxer reads out of a file are those it
//! reads out of the file with its compressed boxes read back.
//! `scripts/fetch-conformance.sh` fetches the files into the directory
//! `compressed_boxes` of the one `ISOBMFF_CONFORMANCE_DIR` names.

#![cfg(feature = "compressed-boxes")]

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use isobmff::framing::decompressed;
    use isobmff::{BoxHeader, Boxes, FourCc};
    use isobmff_mux::Demuxer;
    use isobmff_test_support::serde_json::Value;
    use isobmff_test_support::{compressed_box_conformance_files, gpac_children};

    /// Longest payload a compressed box of the files is read back into
    const LIMIT_BYTES: u64 = 1 << 20;

    /// Returns the first of `nodes` GPAC reports as a box of `box_type`
    fn of_type<'report>(nodes: &[&'report Value], box_type: &str) -> Option<&'report Value> {
        nodes
            .iter()
            .copied()
            .find(|node| node.get("@Type").and_then(Value::as_str) == Some(box_type))
    }

    /// Returns the major brand of the `ftyp` GPAC reports at file level in `report`, or in the `otyp` there where there is one
    fn gpac_major_brand(report: &Value) -> FourCc {
        let file = gpac_children(report.get("IsoMediaFile").unwrap());
        let ftyp = of_type(&file, "otyp")
            .map_or_else(
                || of_type(&file, "ftyp"),
                |otyp| of_type(&gpac_children(otyp), "ftyp"),
            )
            .unwrap();
        let brand = ftyp.get("@MajorBrand").and_then(Value::as_str).unwrap();
        FourCc::new(brand.as_bytes().try_into().unwrap())
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_brands_of_a_file_are_those_of_the_ftyp_in_its_otyp_as_gpac_reports_them() {
        for file in compressed_box_conformance_files() {
            let mut demuxer = Demuxer::new();
            loop {
                let event = match demuxer.wanted_input() {
                    Some(wanted) => {
                        let input = usize::try_from(wanted.offset())
                            .ok()
                            .and_then(|offset| file.bytes.get(offset..))
                            .unwrap_or_default();
                        if input.is_empty() {
                            demuxer.finish()
                        } else {
                            demuxer.handle_input(wanted.offset(), input)
                        }
                    }
                    // Nothing is wanted past a box that runs to the end of the file: it ends with the file.
                    None => match demuxer.finish() {
                        Ok(None) => break,
                        finished => finished,
                    },
                };
                event.unwrap();
            }

            assert_eq!(
                demuxer.file_type().map(|ftyp| ftyp.major_brand),
                Some(gpac_major_brand(&file.report)),
                "{}",
                file.name
            );
        }
    }

    /// Returns `file` with each of its compressed boxes replaced by the box it reads back into, §8.16.2.1, its `otyp` kept
    fn read_back(file: &[u8]) -> Vec<u8> {
        let mut uncompressed = Vec::new();
        for raw_box in Boxes::new(file) {
            let raw_box = raw_box.unwrap();
            if let Some((replacement_type, payload)) = decompressed(raw_box, LIMIT_BYTES).unwrap() {
                let header =
                    BoxHeader::with_payload_len(replacement_type, payload.len() as u64).unwrap();
                let mut bytes = vec![0; usize::try_from(header.encoded_len()).unwrap()];
                header.encode(&mut bytes).unwrap();
                uncompressed.extend_from_slice(&bytes);
                uncompressed.extend_from_slice(&payload);
            } else {
                let header = raw_box.header();
                let mut bytes = vec![0; usize::try_from(header.encoded_len()).unwrap()];
                header.encode(&mut bytes).unwrap();
                uncompressed.extend_from_slice(&bytes);
                uncompressed.extend_from_slice(raw_box.payload());
            }
        }
        uncompressed
    }

    /// Returns the track, the decoding time and the media data of each sample a demuxer that seeks reads out of `file`
    fn samples(file: &[u8]) -> Vec<(u32, u64, Vec<u8>)> {
        let mut demuxer = Demuxer::new();
        let mut samples = Vec::new();
        loop {
            let event = match demuxer.wanted_input() {
                Some(wanted) => {
                    let input = usize::try_from(wanted.offset())
                        .ok()
                        .and_then(|offset| file.get(offset..))
                        .unwrap_or_default();
                    if input.is_empty() {
                        demuxer.finish()
                    } else {
                        demuxer.handle_input(wanted.offset(), input)
                    }
                }
                // Nothing is wanted past a box that runs to the end of the file: it ends with the file.
                None => match demuxer.finish() {
                    Ok(None) => break,
                    finished => finished,
                },
            };
            event.unwrap();
            while let Some(sample) = demuxer.poll_sample() {
                let properties = sample.properties();
                samples.push((
                    properties.track_id,
                    properties.decoding_time,
                    sample.data().to_vec(),
                ));
            }
        }
        samples
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_samples_of_a_file_are_those_of_the_file_with_its_compressed_boxes_read_back() {
        let mut counts = BTreeMap::new();
        for file in compressed_box_conformance_files() {
            let read = samples(&file.bytes);
            assert_eq!(read, samples(&read_back(&file.bytes)), "{}", file.name);
            counts.insert(file.name, read.len());
        }

        assert!(
            counts.values().all(|&count| count > 0),
            "a file read no sample: {counts:?}"
        );
    }
}
