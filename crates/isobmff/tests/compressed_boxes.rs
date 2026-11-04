//! Compressed boxes (ISO/IEC 14496-12 §8.16) read back into the boxes they replace, checked on the files of MPEG's `FileFormatConformance` under consideration
//!
//! Every compressed box of the files reads back into a box of its
//! replacement type. `scripts/fetch-conformance.sh` fetches the files into
//! the directory `compressed_boxes` of the one `ISOBMFF_CONFORMANCE_DIR`
//! names.

#![cfg(feature = "compressed-boxes")]

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use isobmff::boxes::{MovieBox, MovieFragmentBox, SegmentIndexBox, SubsegmentIndexBox};
    use isobmff::framing::decompressed;
    use isobmff::{BoxDecode, BoxDefinition, BoxType, Boxes};
    use isobmff_test_support::compressed_box_conformance_files;

    /// Longest payload a compressed box of the files is read back into
    const LIMIT_BYTES: u64 = 1 << 20;

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn every_compressed_box_of_the_files_reads_back_into_a_box_of_its_replacement_type() {
        let mut read_back = BTreeMap::new();
        for file in compressed_box_conformance_files() {
            for raw_box in Boxes::new(&file.bytes) {
                let raw_box = raw_box.unwrap();
                let Some((replacement_type, payload)) = decompressed(raw_box, LIMIT_BYTES).unwrap()
                else {
                    continue;
                };
                let decoded = match replacement_type {
                    MovieBox::BOX_TYPE => Some(MovieBox::decode_payload(&payload).map(|_| ())),
                    MovieFragmentBox::BOX_TYPE => {
                        Some(MovieFragmentBox::decode_payload(&payload).map(|_| ()))
                    }
                    SegmentIndexBox::BOX_TYPE => {
                        Some(SegmentIndexBox::decode_payload(&payload).map(|_| ()))
                    }
                    SubsegmentIndexBox::BOX_TYPE => {
                        Some(SubsegmentIndexBox::decode_payload(&payload).map(|_| ()))
                    }
                    BoxType::Compact(_) | BoxType::Extended(_) => None,
                };
                assert_eq!(decoded, Some(Ok(())), "{}: {replacement_type}", file.name);
                *read_back
                    .entry(*raw_box.header().box_type().four_cc().as_bytes())
                    .or_insert(0) += 1;
            }
        }

        assert_eq!(
            read_back,
            BTreeMap::from([(*b"!mof", 10), (*b"!mov", 3), (*b"!six", 3), (*b"!ssx", 2)])
        );
    }
}
