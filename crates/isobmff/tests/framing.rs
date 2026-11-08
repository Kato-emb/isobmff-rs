//! The boxes at file level, read from bytes cut anywhere, checked against GPAC on the conformance files

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use isobmff::framing::{BoxReader, Error, Event, FramerConfig};
    use isobmff_core::{BoxHeader, BoxType};
    use isobmff_test_support::fixtures::written;
    use isobmff_test_support::{
        compressed_box_conformance_files, conformance_files, gpac_children, gpac_frame, gpac_type,
        tally,
    };

    const FIXTURES: [(&str, &[u8]); 4] = [
        (
            "movie_moov_last.mp4",
            include_bytes!("fixtures/movie_moov_last.mp4"),
        ),
        (
            "movie_moov_first.mp4",
            include_bytes!("fixtures/movie_moov_first.mp4"),
        ),
        ("fragmented.mp4", include_bytes!("fixtures/fragmented.mp4")),
        ("segment.mp4", include_bytes!("fixtures/segment.mp4")),
    ];

    /// A limit no structure-data of the files read here reaches
    const PAYLOAD_LIMIT: u64 = 64 * 1024 * 1024;

    /// An [`Event`] that owns what it lent
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Reported {
        Box {
            offset: u64,
            header: BoxHeader,
        },
        Gathered {
            offset: u64,
            box_type: BoxType,
            payload_offset: u64,
            payload: Vec<u8>,
        },
        Error(Error),
        /// An event of a variant added after this test was written
        Other(String),
    }

    impl Reported {
        /// The type, file offset and total of the box a box event reports
        fn frame(&self, file: &[u8]) -> Option<(BoxType, u64, u64)> {
            match self {
                Self::Box { offset, header } => {
                    Some((header.box_type(), *offset, total(*offset, *header, file)))
                }
                Self::Gathered {
                    offset,
                    box_type,
                    payload_offset,
                    payload,
                } => Some((
                    *box_type,
                    *offset,
                    payload_offset
                        .checked_sub(*offset)?
                        .checked_add(u64::try_from(payload.len()).ok()?)?,
                )),
                Self::Error(_) | Self::Other(_) => None,
            }
        }
    }

    fn drain(reader: &mut BoxReader, reported: &mut Vec<Reported>) {
        while let Some(event) = reader.poll_event() {
            reported.push(match event {
                Event::Box { offset, header } => Reported::Box { offset, header },
                Event::Gathered {
                    offset,
                    box_type,
                    payload_offset,
                    payload,
                } => Reported::Gathered {
                    offset,
                    box_type,
                    payload_offset,
                    payload: payload.to_vec(),
                },
                Event::Error(error) => Reported::Error(error),
                other => Reported::Other(format!("{other:?}")),
            });
        }
    }

    /// Hands `bytes`, which start at the file offset `position`, over until the reader wants what lies past them, reporting each event
    fn hand_over(
        reader: &mut BoxReader,
        position: u64,
        bytes: &[u8],
        reported: &mut Vec<Reported>,
    ) {
        let end = position
            .checked_add(u64::try_from(bytes.len()).unwrap())
            .unwrap();
        while reader
            .required_input()
            .is_some_and(|required| required.position < end)
        {
            reader.handle_input(position, bytes);
            drain(reader, reported);
        }
    }

    /// Reads `file` handed over from its start in pieces of `piece` bytes, as a caller that cannot seek does
    fn read_in_pieces(file: &[u8], config: &FramerConfig, piece: usize) -> Vec<Reported> {
        let mut reader = BoxReader::new(config.clone());
        let mut reported = Vec::new();
        for (index, bytes) in file.chunks(piece).enumerate() {
            let position = u64::try_from(index.checked_mul(piece).unwrap()).unwrap();
            hand_over(&mut reader, position, bytes, &mut reported);
        }
        reader.finish();
        drain(&mut reader, &mut reported);
        reported
    }

    /// Reads `file` where the reader asks, as a caller that seeks does, and returns what it read with the ranges it handed over
    fn read_where_required(
        file: &[u8],
        config: &FramerConfig,
    ) -> (Vec<Reported>, Vec<Range<usize>>) {
        let mut reader = BoxReader::new(config.clone());
        let (mut reported, mut handed) = (Vec::new(), Vec::new());
        loop {
            drain(&mut reader, &mut reported);
            let Some(required) = reader.required_input() else {
                break;
            };
            let start = usize::try_from(required.position).unwrap();
            let length = required
                .length
                .map_or(4096, |length| usize::try_from(length).unwrap());
            let range = start.min(file.len())..start.saturating_add(length).min(file.len());
            if range.is_empty() {
                reader.finish();
            } else {
                reader.handle_input(required.position, file.get(range.clone()).unwrap());
                handed.push(range);
            }
        }
        (reported, handed)
    }

    /// The events of `file`, the same however it is handed over
    fn read_every_way(name: &str, file: &[u8], config: &FramerConfig) -> Vec<Reported> {
        let whole = read_in_pieces(file, config, file.len().max(1));
        assert_eq!(
            read_in_pieces(file, config, 1),
            whole,
            "{name} in 1-byte pieces"
        );
        assert_eq!(
            read_in_pieces(file, config, 4096),
            whole,
            "{name} in 4 KiB pieces"
        );
        assert_eq!(
            read_where_required(file, config).0,
            whole,
            "{name} read where required"
        );
        whole
    }

    fn total(offset: u64, header: BoxHeader, file: &[u8]) -> u64 {
        header.size().total_bytes().unwrap_or_else(|| {
            u64::try_from(file.len())
                .unwrap()
                .checked_sub(offset)
                .unwrap()
        })
    }

    #[test]
    fn a_file_reads_as_the_same_boxes_however_its_bytes_are_cut() {
        for (name, file) in FIXTURES {
            let reported = read_every_way(name, file, &FramerConfig::new(PAYLOAD_LIMIT));
            let covered = reported
                .iter()
                .map(|reported| reported.frame(file).map(|(_, _, total)| total))
                .sum::<Option<u64>>();
            assert_eq!(
                covered,
                Some(u64::try_from(file.len()).unwrap()),
                "{name}: {reported:?}"
            );
        }
    }

    /// The type of each box of `reported`, with whether its payload was gathered
    fn layout(reported: &[Reported]) -> Vec<(String, bool)> {
        reported
            .iter()
            .map(|reported| match reported {
                Reported::Box { header, .. } => (header.box_type().to_string(), false),
                Reported::Gathered { box_type, .. } => (box_type.to_string(), true),
                Reported::Error(error) => (error.to_string(), false),
                Reported::Other(other) => (other.clone(), false),
            })
            .collect()
    }

    #[test]
    fn the_fixtures_frame_as_the_boxes_ffmpeg_lays_out() {
        let config = FramerConfig::new(PAYLOAD_LIMIT);
        let gathered = |box_type: &str| (box_type.to_owned(), true);
        let skipped = |box_type: &str| (box_type.to_owned(), false);
        let fragment = [gathered("moof"), skipped("mdat")];
        let segment = [gathered("styp"), gathered("moof"), skipped("mdat")];
        let expected: [(&str, Vec<(String, bool)>); 4] = [
            (
                "movie_moov_last.mp4",
                vec![
                    gathered("ftyp"),
                    skipped("free"),
                    skipped("mdat"),
                    gathered("moov"),
                ],
            ),
            (
                "movie_moov_first.mp4",
                vec![
                    gathered("ftyp"),
                    gathered("moov"),
                    skipped("free"),
                    skipped("mdat"),
                ],
            ),
            (
                "fragmented.mp4",
                [
                    vec![gathered("ftyp"), gathered("moov")],
                    fragment.iter().cloned().cycle().take(6).collect(),
                    vec![gathered("mfra")],
                ]
                .concat(),
            ),
            (
                "segment.mp4",
                [
                    vec![gathered("ftyp"), gathered("moov")],
                    segment.iter().cloned().cycle().take(9).collect(),
                ]
                .concat(),
            ),
        ];
        for ((name, file), (expected_name, boxes)) in FIXTURES.into_iter().zip(expected) {
            assert_eq!(name, expected_name);
            assert_eq!(
                layout(&read_every_way(name, file, &config)),
                boxes,
                "{name}"
            );
        }
    }

    #[test]
    fn the_events_across_boxes_that_cannot_be_read() {
        let config = FramerConfig::new(16);
        let over_limit_then_malformed = [
            &compact_box(b"moov", 32)[..],
            &compact_box(b"moof", 8),
            &compact_box(b"mdat", 4),
            b"\0\0\0\x04abcd",
            &compact_box(b"moov", 4),
        ]
        .concat();
        let truncated_payload = [&compact_box(b"ftyp", 8)[..], &compact_box(b"moov", 12)]
            .concat()
            .get(..24)
            .unwrap()
            .to_vec();
        let truncated_header = [&compact_box(b"ftyp", 8)[..], b"\0\0\0\x10mo"].concat();
        let lines = [
            over_limit_then_malformed,
            truncated_payload,
            truncated_header,
        ]
        .iter()
        .flat_map(|bytes| {
            let reported = read_cut_everywhere(bytes, &config);
            layout(&reported)
                .into_iter()
                .map(|(event, gathered)| {
                    if gathered {
                        format!("{event} gathered")
                    } else {
                        event
                    }
                })
                .chain(["--".to_owned()])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
        insta::assert_debug_snapshot!(lines);
    }

    #[test]
    fn a_reader_that_seeks_reads_no_byte_of_a_box_it_skips() {
        for (name, file) in FIXTURES {
            let (reported, handed) = read_where_required(file, &FramerConfig::new(PAYLOAD_LIMIT));
            let skipped = reported
                .iter()
                .filter_map(|reported| match reported {
                    Reported::Box { offset, header } => {
                        let start = offset.checked_add(header.encoded_len()).unwrap();
                        let end = offset.checked_add(total(*offset, *header, file)).unwrap();
                        Some(usize::try_from(start).unwrap()..usize::try_from(end).unwrap())
                    }
                    Reported::Gathered { .. } | Reported::Error(_) | Reported::Other(_) => None,
                })
                .collect::<Vec<_>>();
            assert!(
                skipped.iter().any(|range| !range.is_empty()),
                "{name} skips no payload"
            );
            let read: usize = handed.iter().map(ExactSizeIterator::len).sum();
            let skipped_bytes: usize = skipped.iter().map(ExactSizeIterator::len).sum();
            assert_eq!(Some(read), file.len().checked_sub(skipped_bytes), "{name}");
            for range in &handed {
                assert!(
                    skipped
                        .iter()
                        .all(|payload| range.end <= payload.start || payload.end <= range.start),
                    "{name}: read {range:?} of a payload skipped"
                );
            }
        }
    }

    /// One box of `payload_len` bytes of payload
    fn compact_box(box_type: &[u8; 4], payload_len: usize) -> Vec<u8> {
        written(box_type, &[&vec![0xab; payload_len]])
    }

    /// The events of `bytes` handed over in two pieces cut at each position, and at none
    fn read_cut_everywhere(bytes: &[u8], config: &FramerConfig) -> Vec<Reported> {
        let whole = read_in_pieces(bytes, config, bytes.len().max(1));
        for cut in 0..=bytes.len() {
            let mut reader = BoxReader::new(config.clone());
            let mut reported = Vec::new();
            hand_over(&mut reader, 0, bytes.get(..cut).unwrap(), &mut reported);
            hand_over(
                &mut reader,
                u64::try_from(cut).unwrap(),
                bytes.get(cut..).unwrap(),
                &mut reported,
            );
            reader.finish();
            drain(&mut reader, &mut reported);
            assert_eq!(reported, whole, "cut at {cut}");
        }
        whole
    }

    #[test]
    fn a_header_whose_size_is_smaller_than_the_header_ends_reading() {
        let bytes = [
            &compact_box(b"free", 4)[..],
            b"\0\0\0\x04abcd",
            &compact_box(b"moov", 4),
        ]
        .concat();
        let reported = read_cut_everywhere(&bytes, &FramerConfig::new(PAYLOAD_LIMIT));
        assert!(
            matches!(
                reported.as_slice(),
                [
                    Reported::Box { offset: 0, .. },
                    Reported::Error(Error::MalformedHeader { offset: 12, .. })
                ]
            ),
            "{reported:?}"
        );
    }

    #[test]
    fn input_that_ends_inside_a_box_gathered_reports_it_truncated() {
        let config = FramerConfig::new(PAYLOAD_LIMIT);
        let header = |bytes: &[u8]| BoxHeader::decode(bytes).unwrap().0;
        let gathered = compact_box(b"moov", 16);
        assert_eq!(
            read_cut_everywhere(gathered.get(..20).unwrap(), &config),
            [Reported::Error(Error::TruncatedBox {
                offset: 0,
                header: header(&gathered)
            })]
        );
        let skipped = compact_box(b"mdat", 16);
        assert_eq!(
            read_cut_everywhere(skipped.get(..20).unwrap(), &config),
            [Reported::Box {
                offset: 0,
                header: header(&skipped)
            }]
        );
        assert_eq!(
            read_cut_everywhere(b"\0\0\0\x10mo", &config),
            [Reported::Error(Error::TruncatedHeader { offset: 0 })]
        );
    }

    #[test]
    fn a_payload_over_the_limit_is_skipped_and_reading_goes_on() {
        let bytes = [&compact_box(b"moov", 32)[..], &compact_box(b"moof", 8)].concat();
        let reported = read_cut_everywhere(&bytes, &FramerConfig::new(16));
        let header = BoxHeader::decode(&bytes).unwrap().0;
        assert_eq!(
            reported,
            [
                Reported::Error(Error::PayloadOverLimit {
                    offset: 0,
                    header,
                    limit: 16
                }),
                Reported::Gathered {
                    offset: 40,
                    box_type: BoxType::compact(*b"moof"),
                    payload_offset: 48,
                    payload: vec![0xab; 8],
                },
            ]
        );
    }

    #[test]
    fn a_payload_under_the_limit_is_gathered_whatever_follows_it() {
        let bytes = [&compact_box(b"moov", 8)[..], &compact_box(b"mdat", 64)].concat();
        let reported = read_cut_everywhere(&bytes, &FramerConfig::new(16));
        assert!(
            matches!(
                &reported[..],
                [
                    Reported::Gathered { offset: 0, .. },
                    Reported::Box { offset: 16, .. }
                ]
            ),
            "{reported:?}"
        );
    }

    #[test]
    fn a_box_of_size_0_runs_to_the_end_of_the_input() {
        let config = FramerConfig::new(PAYLOAD_LIMIT);
        let gathered = [&compact_box(b"ftyp", 8)[..], b"\0\0\0\0moov", &[0xcd; 5]].concat();
        let reported = read_cut_everywhere(&gathered, &config);
        assert!(
            matches!(
                &reported[..],
                [Reported::Gathered { offset: 0, .. }, Reported::Gathered { offset: 16, payload, .. }] if payload == &[0xcd; 5]
            ),
            "{reported:?}"
        );

        let skipped = [b"\0\0\0\0mdat", &[0xcd; 5][..]].concat();
        let mut reader = BoxReader::new(config.clone());
        let mut reported = Vec::new();
        hand_over(&mut reader, 0, &skipped, &mut reported);
        assert_eq!(reader.required_input(), None);
        assert!(
            matches!(&reported[..], [Reported::Box { offset: 0, .. }]),
            "{reported:?}"
        );

        let over_limit = read_cut_everywhere(&gathered, &FramerConfig::new(4));
        assert!(
            matches!(
                &over_limit[..],
                [
                    Reported::Error(Error::PayloadOverLimit { offset: 0, .. }),
                    Reported::Error(Error::PayloadOverLimit { offset: 16, .. })
                ]
            ),
            "{over_limit:?}"
        );
    }

    #[test]
    fn a_header_with_a_largesize_or_a_usertype_frames_its_box() {
        let large = [
            &1_u32.to_be_bytes()[..],
            b"free",
            &24_u64.to_be_bytes(),
            &[0; 8],
        ]
        .concat();
        let user = [&28_u32.to_be_bytes()[..], b"uuid", &[0x11; 16], &[0; 4]].concat();
        let bytes = [large.as_slice(), &user].concat();
        let reported = read_cut_everywhere(&bytes, &FramerConfig::new(PAYLOAD_LIMIT));
        assert_eq!(
            reported,
            [
                Reported::Box {
                    offset: 0,
                    header: BoxHeader::decode(&large).unwrap().0
                },
                Reported::Box {
                    offset: 24,
                    header: BoxHeader::decode(&user).unwrap().0
                },
            ]
        );
        assert!(matches!(
            reported.get(1),
            Some(Reported::Box { header, .. }) if matches!(header.box_type(), BoxType::Extended(_))
        ));
    }

    #[test]
    #[should_panic(expected = "after the offset")]
    fn input_that_starts_after_the_required_offset_is_a_bug_of_the_caller() {
        let mut reader = BoxReader::new(FramerConfig::new(PAYLOAD_LIMIT));
        reader.handle_input(1, b"\0\0\0\x08free");
    }

    #[test]
    #[should_panic(expected = "after finish")]
    fn input_after_finish_is_a_bug_of_the_caller() {
        let mut reader = BoxReader::new(FramerConfig::new(PAYLOAD_LIMIT));
        reader.finish();
        reader.handle_input(0, b"\0\0\0\x08free");
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR"]
    fn the_conformance_files_read_as_the_boxes_gpac_reports() {
        let mut differences = Vec::new();
        for file in conformance_files()
            .into_iter()
            .chain(compressed_box_conformance_files())
        {
            let reported =
                read_every_way(&file.name, &file.bytes, &FramerConfig::new(PAYLOAD_LIMIT));
            let ours = tally(reported.iter().filter_map(|reported| match reported {
                Reported::Box { .. } | Reported::Gathered { .. } => {
                    reported.frame(&file.bytes).map(|(box_type, _, total)| {
                        (gpac_type(box_type.four_cc().as_bytes()), total)
                    })
                }
                Reported::Error(error) => {
                    differences.push(format!("{}: {error}", file.name));
                    None
                }
                Reported::Other(other) => {
                    differences.push(format!("{}: {other}", file.name));
                    None
                }
            }));
            let theirs = tally(
                gpac_children(file.report.get("IsoMediaFile").unwrap())
                    .into_iter()
                    .map(gpac_frame),
            );
            if ours != theirs {
                differences.push(format!(
                    "{}: read {ours:?}, GPAC reports {theirs:?}",
                    file.name
                ));
            }
        }
        assert_eq!(differences, Vec::<String>::new());
    }
}
