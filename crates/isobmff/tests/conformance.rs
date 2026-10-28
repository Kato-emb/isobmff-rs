//! Reading and writing of the boxes of a file as its bytes arrive, checked on the files of MPEG's `FileFormatConformance`
//!
//! What the reader frames out of a file cut into pieces is checked against
//! what [`Boxes`] frames out of the file whole, and what the writer writes
//! from the reader's events against the file itself.
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests.

#[cfg(test)]
mod tests {
    use isobmff::framing::{BoxEvent, BoxReader, BoxWriter};
    use isobmff::{BoxHeader, Boxes, ByteRange};
    use isobmff_test_support::{ConformanceFile, conformance_files};

    /// Lengths the files are cut into, the last of them longer than any file
    const PIECE_LENS: [usize; 4] = [1, 7, 4096, usize::MAX];

    /// What the caller answers each header with
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Answer {
        Gather,
        Stream,
        Skip,
    }

    /// What a caller saw reading a file
    struct Read {
        /// Header of each box, and where it lies
        boxes: Vec<(BoxHeader, ByteRange)>,
        /// Bytes the caller passed to the reader
        passed_bytes: usize,
        /// The file as a writer wrote it from the events of the boxes gathered or streamed
        written: Vec<u8>,
    }

    /// Reads `file` from wherever the reader wants it, `piece_len` bytes at a time, answering every header with `answer`
    fn read(file: &[u8], piece_len: usize, answer: Answer) -> Read {
        let mut reader = BoxReader::new();
        let mut writer = BoxWriter::new();
        let mut boxes = Vec::new();
        let mut passed_bytes = 0_usize;
        let mut written = Vec::new();
        while let Some(wanted) = reader.wanted_input() {
            let rest = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            let input = rest.get(..piece_len).unwrap_or(rest);
            if input.is_empty() {
                break;
            }
            passed_bytes = passed_bytes.saturating_add(input.len());
            for event in reader.handle_input(wanted.offset(), input).unwrap() {
                if let BoxEvent::Header(header) = event {
                    let byte_range = reader.open_box().unwrap();
                    boxes.push((header, byte_range));
                    match answer {
                        Answer::Gather => reader.gather(u64::MAX).unwrap(),
                        Answer::Stream => reader.stream().unwrap(),
                        Answer::Skip => {
                            reader.skip().unwrap();
                            assert_eq!(
                                reader.wanted_input(),
                                byte_range.length().map(|length| {
                                    ByteRange::new(byte_range.offset().saturating_add(length), None)
                                }),
                                "the reader wants the file offset past the box it skips"
                            );
                        }
                    }
                }
                write_back(&mut writer, &reader, event, answer, &mut written);
            }
        }
        for event in reader.finish().unwrap() {
            write_back(&mut writer, &reader, event, answer, &mut written);
        }
        writer.finish().unwrap();
        Read {
            boxes,
            passed_bytes,
            written,
        }
    }

    /// Passes `event` of a box gathered or streamed on to `writer`, and keeps the bytes it returns
    fn write_back(
        writer: &mut BoxWriter,
        reader: &BoxReader,
        event: BoxEvent<'_>,
        answer: Answer,
        written: &mut Vec<u8>,
    ) {
        if answer == Answer::Skip {
            return;
        }
        if let Some(raw_box) = reader.gathered_box() {
            written.extend_from_slice(
                writer
                    .handle_event(BoxEvent::Payload(raw_box.payload()))
                    .unwrap(),
            );
        }
        written.extend_from_slice(writer.handle_event(event).unwrap());
    }

    /// Header of each box [`Boxes`] frames out of the whole of `file`, and where it lies
    fn framed_whole(file: &[u8]) -> Vec<(BoxHeader, ByteRange)> {
        let mut offset = 0_u64;
        Boxes::new(file)
            .map(|framed| {
                let framed = framed.unwrap();
                let header = framed.header();
                let byte_range = ByteRange::new(offset, header.size().total_bytes());
                offset = offset
                    .saturating_add(header.encoded_len())
                    .saturating_add(u64::try_from(framed.payload().len()).unwrap());
                (header, byte_range)
            })
            .collect()
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_boxes_of_a_file_cut_into_pieces_frame_as_the_whole_file_does_and_write_back_to_it() {
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            let expected = framed_whole(&bytes);
            for answer in [Answer::Gather, Answer::Stream] {
                for piece_len in PIECE_LENS {
                    let read = read(&bytes, piece_len, answer);
                    assert_eq!(
                        read.boxes, expected,
                        "{name} in pieces of {piece_len}, {answer:?}"
                    );
                    assert!(
                        read.written == bytes,
                        "{name} in pieces of {piece_len}, {answer:?}, writes back to other bytes"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_payloads_of_skipped_boxes_are_never_wanted() {
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            let expected = framed_whole(&bytes);
            let headers_bytes: u64 = expected
                .iter()
                .map(|(header, _)| header.encoded_len())
                .sum();
            for piece_len in PIECE_LENS {
                let skipped = read(&bytes, piece_len, Answer::Skip);
                assert_eq!(skipped.boxes, expected, "{name} in pieces of {piece_len}");
                if piece_len == 1 {
                    assert_eq!(
                        u64::try_from(skipped.passed_bytes).unwrap(),
                        headers_bytes,
                        "{name} byte by byte passes more than the headers"
                    );
                }
            }
        }
    }
}
