//! Reading and writing of the boxes of a file as its bytes arrive, checked on the files of MPEG's `FileFormatConformance`
//!
//! What the reader frames out of a file cut into pieces is checked against
//! what [`Boxes`] frames out of the file whole, and what the writer writes
//! back against the file itself. `scripts/fetch-conformance.sh` fetches the
//! files into a directory, which `ISOBMFF_CONFORMANCE_DIR` names to these
//! tests.

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
        Stream,
        Skip,
    }

    /// What a caller saw reading a file
    struct Read {
        /// Header of each box, and where it lies
        boxes: Vec<(BoxHeader, ByteRange)>,
        /// Bytes the caller passed to the reader
        passed_bytes: usize,
        /// The file as a writer wrote back the streamed boxes
        written: Vec<u8>,
    }

    /// Reads `file` from wherever the reader wants it, `piece_len` bytes at a time, answering every header with `answer`
    fn read(file: &[u8], piece_len: usize, answer: Answer) -> Read {
        let mut reader = BoxReader::new();
        let mut writer = BoxWriter::new();
        let mut boxes: Vec<(BoxHeader, ByteRange)> = Vec::new();
        let mut payload = Vec::new();
        let mut passed_bytes = 0_usize;
        while let Some(wanted) = reader.wanted_input() {
            let rest = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            let input = rest.get(..piece_len).unwrap_or(rest);
            if input.is_empty() {
                break;
            }
            passed_bytes = passed_bytes.saturating_add(input.len());
            let mut skipped = None;
            for event in reader.handle_input(wanted.offset(), input).unwrap() {
                match event.unwrap() {
                    BoxEvent::Header(open) => {
                        boxes.push((open.header(), open.byte_range()));
                        match answer {
                            Answer::Stream => open.stream(),
                            Answer::Skip => {
                                skipped = Some(open.byte_range());
                                open.skip();
                            }
                        }
                    }
                    BoxEvent::Payload(bytes) => payload.extend_from_slice(bytes),
                    event @ (BoxEvent::End | BoxEvent::Gathered(_) | _) => {
                        assert!(
                            matches!(event, BoxEvent::End),
                            "{event:?} of a box streamed or skipped"
                        );
                        close(&mut writer, answer, &boxes, &mut payload);
                    }
                }
            }
            if let Some(byte_range) = skipped {
                assert_eq!(
                    reader.wanted_input(),
                    byte_range.length().map(|length| {
                        ByteRange::new(byte_range.offset().saturating_add(length), None)
                    }),
                    "the reader wants the file offset past the box it skips"
                );
            }
        }
        for event in reader.finish().unwrap() {
            assert!(matches!(event.unwrap(), BoxEvent::End));
            close(&mut writer, answer, &boxes, &mut payload);
        }
        Read {
            boxes,
            passed_bytes,
            written: core::iter::from_fn(|| writer.poll_output())
                .flatten()
                .collect(),
        }
    }

    /// Writes back the box `boxes` ends with, when it was streamed, out of the `payload` gathered from its pieces
    fn close(
        writer: &mut BoxWriter,
        answer: Answer,
        boxes: &[(BoxHeader, ByteRange)],
        payload: &mut Vec<u8>,
    ) {
        if answer == Answer::Stream {
            let (header, _) = boxes.last().unwrap();
            writer.handle_box(*header, [payload.as_slice()]).unwrap();
        }
        payload.clear();
    }

    /// Header of each box [`Boxes`] frames out of the whole of `file`, and where it lies
    fn framed_whole(file: &[u8]) -> Vec<(BoxHeader, ByteRange)> {
        let mut offset = 0;
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
            for piece_len in PIECE_LENS {
                let streamed = read(&bytes, piece_len, Answer::Stream);
                assert_eq!(streamed.boxes, expected, "{name} in pieces of {piece_len}");
                assert!(
                    streamed.written == bytes,
                    "{name} in pieces of {piece_len} writes back to other bytes"
                );
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
