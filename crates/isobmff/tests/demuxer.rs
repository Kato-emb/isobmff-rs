//! A file read into its boxes and the location of each sample of its movie, however its bytes are handed over

#[cfg(test)]
mod tests {
    use isobmff::boxes::{DataEntry, DataEntryImdaBox, FileTypeBox, MovieBox, SampleToChunkEntry};
    use isobmff::demux::{Error, Event};
    use isobmff::structure_data::data_layout::DataOrigins;
    use isobmff::structure_data::{self, MovieReader};
    use isobmff::{
        BoxDecode, BoxDefinition, BoxHeader, Boxes, Demuxer, DemuxerConfig, SampleLocation,
        file_level,
    };
    use isobmff_test_support::fixtures::{encoded, movie, track, written};
    use isobmff_test_support::{conformance_files, movie_box};

    /// A movie file whose samples all lie after the `moov`
    const MOVIE_FIRST: &[u8] = include_bytes!("fixtures/movie_moov_first.mp4");

    /// A movie file whose samples all lie before the `moov`
    const MOVIE_LAST: &[u8] = include_bytes!("fixtures/movie_moov_last.mp4");

    /// The default limits, with each `(payload_limit, sample_count_limit)` given that differs
    fn config(payload_limit: Option<u64>, sample_count_limit: Option<u64>) -> DemuxerConfig {
        let mut config = DemuxerConfig::default();
        config.payload_limit = payload_limit.unwrap_or(config.payload_limit);
        config.sample_count_limit = sample_count_limit.unwrap_or(config.sample_count_limit);
        config
    }

    fn drain(demuxer: &mut Demuxer, events: &mut Vec<Event>) {
        while let Some(event) = demuxer.poll_event() {
            events.push(event);
        }
    }

    /// Hands `demuxer` the bytes of `file` from `position` on, or ends the input where none is left
    fn hand_over(demuxer: &mut Demuxer, file: &[u8], position: u64) {
        match file.get(usize::try_from(position).unwrap()..) {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(position, bytes),
            _ => demuxer.finish(),
        }
    }

    /// Reads `file` where the demuxer asks, as a caller that seeks does
    fn read_where_required(file: &[u8], config: DemuxerConfig) -> Vec<Event> {
        let mut demuxer = Demuxer::new(config);
        let mut events = Vec::new();
        loop {
            drain(&mut demuxer, &mut events);
            let Some(required) = demuxer.required_input() else {
                break;
            };
            hand_over(&mut demuxer, file, required.position);
        }
        events
    }

    /// Reads `file` handed over from its start in pieces of `piece` bytes, each again until the demuxer wants what lies past it, as a caller that cannot seek does: it ends the input where the demuxer wants bytes before the piece
    fn read_in_pieces(file: &[u8], config: DemuxerConfig, piece: usize) -> Vec<Event> {
        let mut demuxer = Demuxer::new(config);
        let mut events = Vec::new();
        for (index, bytes) in file.chunks(piece).enumerate() {
            let position = u64::try_from(index.checked_mul(piece).unwrap()).unwrap();
            let end = position
                .checked_add(u64::try_from(bytes.len()).unwrap())
                .unwrap();
            while let Some(required) = demuxer.required_input()
                && required.position < end
            {
                if required.position < position {
                    demuxer.finish();
                    drain(&mut demuxer, &mut events);
                    return events;
                }
                demuxer.handle_input(position, bytes);
                drain(&mut demuxer, &mut events);
            }
        }
        demuxer.finish();
        drain(&mut demuxer, &mut events);
        events
    }

    /// The events of `file`, the same however it is handed over
    fn read_every_way(name: &str, file: &[u8], config: DemuxerConfig) -> Vec<Event> {
        let events = read_where_required(file, config);
        for piece in [1, 4096, file.len().max(1)] {
            assert_eq!(
                read_in_pieces(file, config, piece),
                events,
                "{name} in pieces of {piece}"
            );
        }
        events
    }

    /// One line that says what `event` reports: the type of a box read into a value or not, and an error with its causes
    fn line(event: &Event) -> String {
        match event {
            Event::FileType(_) => String::from("FileType"),
            Event::Movie(movie) => format!(
                "Movie({:?})",
                movie
                    .trak
                    .iter()
                    .map(|trak| trak.tkhd.track_id)
                    .collect::<Vec<_>>()
            ),
            Event::Box { header, .. } => format!("Box({:?})", header.box_type().to_string()),
            Event::Error(error) => {
                core::iter::successors(core::error::Error::source(error), |cause| cause.source())
                    .fold(format!("{error}"), |line, cause| format!("{line}: {cause}"))
            }
            Event::SampleLocation(_) | _ => format!("{event:?}"),
        }
    }

    /// The locations `events` reports
    fn locations(events: &[Event]) -> Vec<SampleLocation> {
        events
            .iter()
            .filter_map(|event| {
                if let Event::SampleLocation(location) = event {
                    Some(*location)
                } else {
                    None
                }
            })
            .collect()
    }

    /// The location of each sample of `movie` at its offset in its container, as the structure-data reads them, in decoding order track after track
    fn placed(movie: &MovieBox) -> Result<Vec<SampleLocation>, structure_data::Error> {
        let mut data_origins = DataOrigins::new();
        MovieReader::new()
            .handle_movie(movie)?
            .map(|location| data_origins.handle_location(location))
            .collect()
    }

    /// The locations [`placed`] returns, in the order of their offsets
    fn located(movie: &MovieBox) -> Result<Vec<SampleLocation>, structure_data::Error> {
        let mut locations = placed(movie)?;
        locations.sort_by_key(|location| location.offset);
        Ok(locations)
    }

    /// The file offset, header and payload of each box at file level of `file`
    fn boxes_of(file: &[u8]) -> Vec<(u64, BoxHeader, &[u8])> {
        let mut offset = 0_u64;
        Boxes::new(file)
            .map(|raw_box| {
                let raw_box = raw_box.unwrap();
                let at = offset;
                offset = offset
                    .checked_add(raw_box.header().encoded_len())
                    .and_then(|end| end.checked_add(u64::try_from(raw_box.payload().len()).ok()?))
                    .unwrap();
                (at, raw_box.header(), raw_box.payload())
            })
            .collect()
    }

    /// The events of a movie file whose boxes read: each box at file level in order, the `ftyp` and the `moov` read into their values, and the locations of the samples of the `moov` after it
    fn events_of_boxes(file: &[u8]) -> Vec<Event> {
        boxes_of(file)
            .into_iter()
            .flat_map(|(offset, header, payload)| {
                if header.box_type() == FileTypeBox::BOX_TYPE {
                    vec![Event::FileType(
                        FileTypeBox::decode_payload(payload).unwrap(),
                    )]
                } else if header.box_type() == MovieBox::BOX_TYPE {
                    let movie = MovieBox::decode_payload(payload).unwrap();
                    let located = located(&movie).unwrap();
                    [Event::Movie(Box::new(movie))]
                        .into_iter()
                        .chain(located.into_iter().map(Event::SampleLocation))
                        .collect()
                } else {
                    vec![Event::Box { offset, header }]
                }
            })
            .collect()
    }

    /// `events`, the boxes not read into values moved `by` bytes on in the file
    fn shifted(events: Vec<Event>, by: usize) -> Vec<Event> {
        let by = u64::try_from(by).unwrap();
        events
            .into_iter()
            .map(|mut event| {
                if let Event::Box { offset, .. } = &mut event {
                    *offset = offset.checked_add(by).unwrap();
                }
                event
            })
            .collect()
    }

    /// `events` with `error` in place of the `moov` and the locations of its samples
    fn refused_movie(events: Vec<Event>, error: Error) -> Vec<Event> {
        let mut events = events
            .into_iter()
            .filter(|event| !matches!(event, Event::Movie(_) | Event::SampleLocation(_)))
            .collect::<Vec<_>>();
        events.insert(1, Event::Error(error));
        events
    }

    #[test]
    fn the_events_of_a_movie_file_are_its_boxes_and_the_locations_of_its_samples() {
        for (name, file, boxes) in [
            (
                "movie_moov_first.mp4",
                MOVIE_FIRST,
                ["ftyp", "moov", "free", "mdat"],
            ),
            (
                "movie_moov_last.mp4",
                MOVIE_LAST,
                ["ftyp", "free", "mdat", "moov"],
            ),
        ] {
            let events = read_every_way(name, file, config(None, None));
            assert_eq!(
                boxes_of(file)
                    .iter()
                    .map(|(_, header, _)| header.box_type().to_string())
                    .collect::<Vec<_>>(),
                boxes,
                "{name}"
            );
            assert_eq!(events.len(), 4 + 12 + 58, "{name}");
            assert_eq!(events, events_of_boxes(file), "{name}");
        }
    }

    #[test]
    fn input_handed_over_while_a_location_waits_is_not_taken() {
        let expected = events_of_boxes(MOVIE_FIRST);
        let mut demuxer = Demuxer::new(config(None, None));
        let mut events = Vec::new();
        while !matches!(events.last(), Some(Event::Movie(_))) {
            let required = demuxer.required_input().unwrap();
            hand_over(&mut demuxer, MOVIE_FIRST, required.position);
            events.extend(demuxer.poll_event());
        }
        assert_eq!(events, expected.get(..2).unwrap());

        let waiting = demuxer.required_input();
        hand_over(&mut demuxer, MOVIE_FIRST, waiting.unwrap().position);
        assert_eq!(
            demuxer.required_input(),
            waiting,
            "the bytes after the moov, handed over while its locations wait"
        );
        drain(&mut demuxer, &mut events);
        assert_eq!(events, expected.get(..2 + 12 + 58).unwrap());

        assert_eq!(demuxer.required_input(), waiting);
        hand_over(&mut demuxer, MOVIE_FIRST, waiting.unwrap().position);
        assert_ne!(
            demuxer.required_input(),
            waiting,
            "the same bytes, handed over once every location is polled"
        );
        drain(&mut demuxer, &mut events);
        assert_eq!(events, expected.get(..2 + 12 + 58 + 1).unwrap());
    }

    /// A demuxer of `movie_moov_first.mp4` that has reported its `moov`, whose locations wait
    fn demuxer_with_locations_waiting() -> Demuxer {
        let mut demuxer = Demuxer::new(config(None, None));
        while !matches!(demuxer.poll_event(), Some(Event::Movie(_))) {
            let required = demuxer.required_input().unwrap();
            hand_over(&mut demuxer, MOVIE_FIRST, required.position);
        }
        demuxer
    }

    #[test]
    #[should_panic(expected = "after the offset")]
    fn input_after_the_required_offset_while_a_location_waits_is_a_bug() {
        let mut demuxer = demuxer_with_locations_waiting();
        let required = demuxer.required_input().unwrap();
        demuxer.handle_input(required.position.checked_add(1).unwrap(), MOVIE_FIRST);
    }

    #[test]
    #[should_panic(expected = "after finish")]
    fn input_after_finish_while_a_location_waits_is_a_bug() {
        let mut demuxer = demuxer_with_locations_waiting();
        demuxer.finish();
        demuxer.handle_input(0, MOVIE_FIRST);
    }

    #[test]
    #[should_panic(expected = "after the offset")]
    fn input_after_the_required_offset_while_a_box_waits_is_a_bug() {
        let mut demuxer = Demuxer::new(config(None, None));
        demuxer.handle_input(0, MOVIE_FIRST);
        let required = demuxer.required_input().unwrap();
        demuxer.handle_input(required.position.checked_add(1).unwrap(), MOVIE_FIRST);
    }

    #[test]
    fn the_samples_of_a_movie_are_located_in_the_order_of_their_offsets() {
        for (name, file) in [
            ("movie_moov_first.mp4", MOVIE_FIRST),
            ("movie_moov_last.mp4", MOVIE_LAST),
            (
                "external_movie.mp4",
                &include_bytes!("fixtures/external_movie.mp4")[..],
            ),
            (
                "chunks_out_of_order.mp4",
                include_bytes!("fixtures/chunks_out_of_order.mp4"),
            ),
        ] {
            let movie = movie_box(file, name).unwrap();
            let reported = locations(&read_every_way(name, file, config(None, None)));
            assert_eq!(reported, located(&movie).unwrap(), "{name}");
            assert_eq!(reported.len(), 12 + 58, "{name}");
            if name == "chunks_out_of_order.mp4" {
                assert_ne!(reported, placed(&movie).unwrap(), "{name}");
            }
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_conformance_files_are_located_as_their_sample_tables_declare() {
        let (mut files, mut sample_locations, mut refused) = (0, 0, 0);
        let mut differences = Vec::<String>::new();
        for file in conformance_files() {
            let Some((offset, _, payload)) = boxes_of(&file.bytes)
                .into_iter()
                .find(|(_, header, _)| header.box_type() == MovieBox::BOX_TYPE)
            else {
                continue;
            };
            let movie = MovieBox::decode_payload(payload).unwrap();
            let events = read_where_required(&file.bytes, config(None, None));
            let reported = locations(&events);
            match located(&movie) {
                Ok(located) => {
                    if reported != located {
                        differences.push(format!(
                            "{}: {} locations reported, {} located",
                            file.name,
                            reported.len(),
                            located.len()
                        ));
                    }
                    files += 1;
                    sample_locations += located.len();
                }
                Err(source) => {
                    let error = Event::Error(Error::StructureData { offset, source });
                    if !reported.is_empty() || !events.contains(&error) {
                        differences.push(format!(
                            "{}: {} locations reported, and not {error:?}",
                            file.name,
                            reported.len()
                        ));
                    }
                    refused += 1;
                }
            }
        }
        println!(
            "compared the locations of {files} files, {sample_locations} samples; {refused} files refused"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(files > 0 && sample_locations > 0, "no file was compared");
    }

    #[test]
    fn a_moov_of_more_samples_than_the_limit_is_reported_and_the_boxes_after_it_are_read() {
        let name = "movie_moov_first.mp4";
        let read = read_every_way(name, MOVIE_FIRST, config(None, Some(12 + 58)));
        assert_eq!(read, events_of_boxes(MOVIE_FIRST));
        let over = read_every_way(name, MOVIE_FIRST, config(None, Some(12 + 58 - 1)));
        assert_eq!(
            over,
            refused_movie(
                events_of_boxes(MOVIE_FIRST),
                Error::SampleCountOverLimit {
                    offset: 32,
                    location_count: 12 + 58,
                    limit: 12 + 58 - 1,
                }
            )
        );
    }

    #[test]
    fn a_moov_refused_leaves_no_location_and_the_moov_after_it_is_read() {
        let mut sample_entry_out_of_range = track(1, 10, &[4, 4], &[(1, 2)], &[100]);
        let stsc = sample_entry_out_of_range
            .mdia
            .minf
            .stbl
            .stsc
            .as_mut()
            .unwrap();
        let mut run = SampleToChunkEntry::default();
        run.first_chunk = 1;
        run.samples_per_chunk = 2;
        run.sample_description_index = 2;
        stsc.entries = vec![run];
        let mut in_an_imda = track(1, 10, &[4, 4], &[(1, 2)], &[100]);
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = 3;
        in_an_imda.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Imda(imdt)];
        let over_the_limit = track(1, 10, &[4; 12 + 58 + 1], &[(1, 71)], &[100]);
        for (refused, sample_count_limit) in [
            (movie([sample_entry_out_of_range]), None),
            (movie([in_an_imda]), None),
            (movie([over_the_limit]), Some(12 + 58)),
        ] {
            let mut refused = refused;
            refused.mvhd.next_track_id = 2;
            let refused_bytes = encoded(&refused);
            let (ftyp, rest) = MOVIE_FIRST.split_at(32);
            let file = [ftyp, &refused_bytes, rest].concat();
            let events = read_every_way(
                "a refused moov first",
                &file,
                config(None, sample_count_limit),
            );
            let expected_error = match located(&refused) {
                Err(source) => Error::StructureData { offset: 32, source },
                Ok(_) => Error::SampleCountOverLimit {
                    offset: 32,
                    location_count: 12 + 58 + 1,
                    limit: 12 + 58,
                },
            };
            let mut expected = shifted(events_of_boxes(MOVIE_FIRST), refused_bytes.len());
            expected.insert(1, Event::Error(expected_error));
            assert_eq!(events, expected);
        }
    }

    #[test]
    fn a_moov_over_the_payload_limit_is_reported_and_the_boxes_after_it_are_read() {
        let name = "movie_moov_first.mp4";
        let reported = read_every_way(name, MOVIE_FIRST, config(Some(64), None));
        assert_eq!(
            reported.iter().map(line).collect::<Vec<_>>(),
            [
                "FileType",
                "a box at file level cannot be read: the payload of the box `moov` at offset 32 is 1792 bytes, over the limit of 64",
                "Box(\"free\")",
                "Box(\"mdat\")"
            ]
        );
    }

    #[test]
    fn a_second_moov_is_refused_and_the_first_movie_kept() {
        let name = "movie_moov_first.mp4";
        let moov = MOVIE_FIRST.get(32..1832).unwrap();
        assert_eq!(moov.get(4..8), Some(&b"moov"[..]));
        let doubled = [MOVIE_FIRST, moov].concat();
        let reported = read_every_way(name, &doubled, config(None, None));
        let mut expected = events_of_boxes(MOVIE_FIRST);
        expected.push(Event::Error(Error::DuplicateMovie {
            offset: u64::try_from(MOVIE_FIRST.len()).unwrap(),
        }));
        assert_eq!(reported, expected);
    }

    #[test]
    fn a_box_that_does_not_read_is_reported_and_the_boxes_after_it_are_read() {
        let short = written(b"ftyp", &[b"ab"]);
        let file = [&short[..], MOVIE_FIRST].concat();
        let events = read_every_way("a short ftyp first", &file, config(None, None));
        let mut expected = vec![Event::Error(Error::Decode {
            offset: 0,
            box_type: FileTypeBox::BOX_TYPE,
            source: FileTypeBox::decode_payload(b"ab").unwrap_err(),
        })];
        expected.extend(shifted(events_of_boxes(MOVIE_FIRST), short.len()));
        assert_eq!(events, expected);
    }

    #[test]
    fn a_file_that_ends_inside_the_moov_reports_it_and_no_location() {
        let truncated = MOVIE_FIRST.get(..1000).unwrap();
        let [ftyp, moov, _, _] = <[_; 4]>::try_from(boxes_of(MOVIE_FIRST)).unwrap();
        assert_eq!(
            read_every_way(
                "movie_moov_first.mp4 cut at 1000",
                truncated,
                config(None, None)
            ),
            [
                Event::FileType(FileTypeBox::decode_payload(ftyp.2).unwrap()),
                Event::Error(Error::FileLevel(file_level::Error::TruncatedBox {
                    offset: 32,
                    header: moov.1
                })),
            ]
        );
    }

    #[test]
    fn a_movie_whose_samples_lie_in_an_imda_not_read_is_refused() {
        let file = include_bytes!("fixtures/imda_movie_moov_first.mp4");
        let [ftyp, _, free, media_data] = <[_; 4]>::try_from(boxes_of(file)).unwrap();
        assert_eq!(
            read_every_way("imda_movie_moov_first.mp4", file, config(None, None)),
            [
                Event::FileType(FileTypeBox::decode_payload(ftyp.2).unwrap()),
                Event::Error(Error::StructureData {
                    offset: 32,
                    source: structure_data::Error::IdentifiedMediaDataNotRead {
                        imda_identifier: 1
                    },
                }),
                Event::Box {
                    offset: free.0,
                    header: free.1
                },
                Event::Box {
                    offset: media_data.0,
                    header: media_data.1
                },
            ]
        );
    }
}
