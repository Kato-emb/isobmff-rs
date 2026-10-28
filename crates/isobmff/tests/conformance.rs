//! Reading and writing of the boxes of a file as its bytes arrive, checked on the files of MPEG's `FileFormatConformance`
//!
//! What the reader frames out of a file cut into pieces is checked against
//! what [`Boxes`] frames out of the file whole, and what the writer writes
//! from the reader's events against the file itself. The boxes at file level
//! read into their values are checked against the boxes GPAC reports.
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use isobmff::file_level::FileLevelBox;
    use isobmff::framing::{BoxEvent, BoxReader, BoxWriter};
    use isobmff::{BoxHeader, BoxType, Boxes, ByteRange};
    use isobmff_test_support::{
        ConformanceFile, conformance_files, gpac_children, gpac_frame, gpac_type,
    };

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

    /// Passes `file` to `reader` from wherever it wants it, `piece_len` bytes at a time, and each event to `on_event`, returning the bytes passed
    fn feed(
        reader: &mut BoxReader,
        file: &[u8],
        piece_len: usize,
        mut on_event: impl FnMut(&mut BoxReader, BoxEvent<'_>),
    ) -> usize {
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
            for event in reader.handle_input(wanted.offset(), input).unwrap() {
                on_event(reader, event);
            }
        }
        for event in reader.finish().unwrap() {
            on_event(reader, event);
        }
        passed_bytes
    }

    /// Reads `file` from wherever the reader wants it, `piece_len` bytes at a time, answering every header with `answer`
    fn read(file: &[u8], piece_len: usize, answer: Answer) -> Read {
        let mut writer = BoxWriter::new();
        let mut boxes = Vec::new();
        let mut written = Vec::new();
        let passed_bytes = feed(&mut BoxReader::new(), file, piece_len, |reader, event| {
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
            write_back(&mut writer, reader, event, answer, &mut written);
        });
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

    /// Reads the boxes at file level of `file` from wherever the reader wants it, `piece_len` bytes at a time, with the header of each and where it lies
    fn read_file_level(file: &[u8], piece_len: usize) -> Vec<(BoxHeader, ByteRange, FileLevelBox)> {
        let mut boxes = Vec::new();
        let mut gathering = None;
        feed(
            &mut BoxReader::new(),
            file,
            piece_len,
            |reader, event| match event {
                BoxEvent::Header(header) => {
                    let byte_range = reader.open_box().unwrap();
                    match FileLevelBox::from_header(header, byte_range) {
                        Some(reported) => {
                            boxes.push((header, byte_range, reported));
                            reader.skip().unwrap();
                        }
                        None => {
                            reader.gather(u64::MAX).unwrap();
                            gathering = Some(byte_range);
                        }
                    }
                }
                BoxEvent::End => {
                    if let (Some(raw_box), Some(byte_range)) =
                        (reader.gathered_box(), gathering.take())
                    {
                        boxes.push((
                            raw_box.header(),
                            byte_range,
                            FileLevelBox::read(raw_box, byte_range).unwrap(),
                        ));
                    }
                }
                BoxEvent::Payload(_) | _ => {}
            },
        );
        boxes
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_boxes_at_file_level_read_into_their_values_as_gpac_reports_them() {
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let file_len = u64::try_from(bytes.len()).unwrap();
            let theirs = tally(
                gpac_children(report.get("IsoMediaFile").unwrap())
                    .into_iter()
                    .map(gpac_frame),
            );
            for piece_len in PIECE_LENS {
                let boxes = read_file_level(&bytes, piece_len);
                let ours = tally(boxes.iter().map(|(header, byte_range, _)| {
                    (
                        gpac_type(header.box_type().four_cc().as_bytes()),
                        byte_range
                            .length()
                            .unwrap_or(file_len.saturating_sub(byte_range.offset())),
                    )
                }));
                assert_eq!(ours, theirs, "{name} in pieces of {piece_len}");
                for (header, _, file_level_box) in &boxes {
                    if header.box_type() == BoxType::compact(*b"meta") {
                        assert!(
                            matches!(file_level_box, FileLevelBox::Other { .. }),
                            "{name}: meta at file level reads as {file_level_box:?}"
                        );
                    }
                }
            }
        }
    }

    /// Returns how many boxes of each type and total `frames` holds
    fn tally(frames: impl IntoIterator<Item = (String, u64)>) -> BTreeMap<(String, u64), usize> {
        frames
            .into_iter()
            .fold(BTreeMap::new(), |mut tally, frame| {
                let count = tally.entry(frame).or_insert(0_usize);
                *count = count.saturating_add(1);
                tally
            })
    }
}
