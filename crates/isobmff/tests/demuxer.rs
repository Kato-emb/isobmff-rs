//! A file read into its boxes and the samples of its tracks, however its bytes are handed over

#[cfg(test)]
mod tests {
    use isobmff::demuxer::{Error, Event};
    use isobmff::{Demuxer, DemuxerConfig, SampleLocation, structure_data};
    use isobmff_boxes::{SampleToChunkBox, TimeToSampleBox};
    use isobmff_core::BoxDefinition;
    use isobmff_test_support::conformance_files;
    use isobmff_test_support::fixtures::fragments_ahead_of_their_samples;
    use std::collections::BTreeMap;

    /// The fixtures whose samples all lie after the `moov`
    const FIXTURES: [(&str, &[u8]); 3] = [
        (
            "movie_moov_first.mp4",
            include_bytes!("fixtures/movie_moov_first.mp4"),
        ),
        ("fragmented.mp4", include_bytes!("fixtures/fragmented.mp4")),
        ("segment.mp4", include_bytes!("fixtures/segment.mp4")),
    ];

    /// A movie file whose samples all lie before the `moov`
    const MOVIE_LAST: &[u8] = include_bytes!("fixtures/movie_moov_last.mp4");

    /// The default limits, with each `(payload_limit, sample_size_limit)` given that differs
    fn config(payload_limit: Option<u64>, sample_size_limit: Option<u64>) -> DemuxerConfig {
        let mut config = DemuxerConfig::default();
        config.payload_limit = payload_limit.unwrap_or(config.payload_limit);
        config.sample_size_limit = sample_size_limit.unwrap_or(config.sample_size_limit);
        config
    }

    /// An [`Event`] that owns what it lent, the boxes read into values left as their type
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Reported {
        FileType,
        SegmentType,
        Movie(Vec<u32>),
        MovieFragment(u32),
        SampleLocation(SampleLocation),
        Sample(SampleLocation, Vec<u8>),
        Box(String),
        Error(Error),
        Other(String),
    }

    impl Reported {
        /// One line that says what was reported
        fn line(&self) -> String {
            match self {
                Self::Sample(location, bytes) => format!(
                    "sample of track {} at {}, {} bytes",
                    location.properties.track_id,
                    location.offset,
                    bytes.len()
                ),
                Self::Error(error) => {
                    core::iter::successors(core::error::Error::source(error), |cause| {
                        cause.source()
                    })
                    .fold(format!("{error}"), |line, cause| format!("{line}: {cause}"))
                }
                Self::FileType
                | Self::SegmentType
                | Self::Movie(_)
                | Self::MovieFragment(_)
                | Self::SampleLocation(_)
                | Self::Box(_)
                | Self::Other(_) => format!("{self:?}"),
            }
        }
    }

    /// Which tracks a reading subscribes to once it has read the `moov`
    #[derive(Clone, Copy)]
    enum Subscribe<'a> {
        Every,
        Only(&'a [u32]),
    }

    fn drain(demuxer: &mut Demuxer, subscribe: Subscribe<'_>, reported: &mut Vec<Reported>) {
        while let Some(event) = demuxer.poll_event() {
            let next = match event {
                Event::FileType(_) => Reported::FileType,
                Event::SegmentType(_) => Reported::SegmentType,
                Event::MovieFragment(moof) => Reported::MovieFragment(moof.mfhd.sequence_number),
                Event::Movie(movie) => {
                    Reported::Movie(movie.trak.iter().map(|trak| trak.tkhd.track_id).collect())
                }
                Event::SampleLocation(location) => Reported::SampleLocation(location),
                Event::Sample(sample) => Reported::Sample(sample.location(), sample.bytes.to_vec()),
                Event::Box { header, .. } => Reported::Box(header.box_type().to_string()),
                Event::Error(error) => Reported::Error(error),
                other => Reported::Other(format!("{other:?}")),
            };
            if let Reported::Movie(track_ids) = &next {
                let track_ids = match subscribe {
                    Subscribe::Every => track_ids.as_slice(),
                    Subscribe::Only(track_ids) => track_ids,
                };
                for &track_id in track_ids {
                    demuxer.subscribe_track(track_id);
                }
            }
            reported.push(next);
        }
    }

    /// Reads `file` where the demuxer asks, as a caller that seeks does
    fn read_where_required(
        file: &[u8],
        config: DemuxerConfig,
        subscribe: Subscribe<'_>,
    ) -> Vec<Reported> {
        let mut demuxer = Demuxer::new(config);
        let mut reported = Vec::new();
        loop {
            drain(&mut demuxer, subscribe, &mut reported);
            let Some(required) = demuxer.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
                _ => demuxer.finish(),
            }
        }
        reported
    }

    /// Reads `file` handed over from its start in pieces of `piece` bytes, each again until the demuxer wants what lies past it, as a caller that cannot seek does: it ends the input where the demuxer wants bytes before the piece
    fn read_in_pieces(
        file: &[u8],
        config: DemuxerConfig,
        subscribe: Subscribe<'_>,
        piece: usize,
    ) -> Vec<Reported> {
        let mut demuxer = Demuxer::new(config);
        let mut reported = Vec::new();
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
                    drain(&mut demuxer, subscribe, &mut reported);
                    return reported;
                }
                demuxer.handle_input(position, bytes);
                drain(&mut demuxer, subscribe, &mut reported);
            }
        }
        demuxer.finish();
        drain(&mut demuxer, subscribe, &mut reported);
        reported
    }

    /// The events of `file`, the same however it is handed over
    fn read_every_way(
        name: &str,
        file: &[u8],
        config: DemuxerConfig,
        subscribe: Subscribe<'_>,
    ) -> Vec<Reported> {
        let reported = read_where_required(file, config, subscribe);
        for piece in [1, 4096, file.len().max(1)] {
            assert_eq!(
                read_in_pieces(file, config, subscribe, piece),
                reported,
                "{name} in pieces of {piece}"
            );
        }
        reported
    }

    /// The bytes of `location` in `file`
    fn bytes_at<'a>(file: &'a [u8], location: &SampleLocation) -> &'a [u8] {
        let start = usize::try_from(location.offset).unwrap();
        file.get(
            start
                ..start
                    .checked_add(usize::try_from(location.size).unwrap())
                    .unwrap(),
        )
        .unwrap()
    }

    /// The location of each sample `reported` lends
    fn samples_lent(reported: &[Reported]) -> Vec<SampleLocation> {
        reported
            .iter()
            .filter_map(|reported| {
                if let Reported::Sample(location, _) = reported {
                    Some(*location)
                } else {
                    None
                }
            })
            .collect()
    }

    /// The locations `reported` holds
    fn locations(reported: &[Reported]) -> Vec<SampleLocation> {
        reported
            .iter()
            .filter_map(|reported| {
                if let Reported::SampleLocation(location) = reported {
                    Some(*location)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Every sample `reported` lends holds the bytes of its location in `file`
    fn lent_as_held(name: &str, file: &[u8], reported: &[Reported]) {
        for reported in reported {
            if let Reported::Sample(location, bytes) = reported {
                assert_eq!(bytes.as_slice(), bytes_at(file, location), "{name}");
            }
        }
    }

    #[test]
    fn a_file_whose_samples_follow_the_moov_reads_the_same_however_it_is_handed_over() {
        for (name, file) in FIXTURES {
            let reported = read_every_way(name, file, config(None, None), Subscribe::Every);
            lent_as_held(name, file, &reported);
            let lent = samples_lent(&reported);
            assert_eq!(lent.len(), 12 + 58, "{name}");
            assert_eq!(lent, locations(&reported), "{name}");
        }
    }

    #[test]
    fn the_samples_before_the_moov_are_lent_to_a_caller_that_seeks() {
        let reported = read_where_required(MOVIE_LAST, config(None, None), Subscribe::Every);
        lent_as_held("movie_moov_last.mp4", MOVIE_LAST, &reported);
        let lent = samples_lent(&reported);
        assert_eq!(lent.len(), 12 + 58);
        assert_eq!(lent, locations(&reported));
    }

    #[test]
    fn a_caller_that_cannot_seek_ends_the_input_at_the_samples_before_the_bytes_it_holds() {
        // Why every sample, or none: the demuxer asks for the samples in the
        // order of their offsets, so the first decides whether the piece the
        // caller holds when the `moov` is read reaches back to them, which only
        // a piece of the whole file does.
        for (piece, sample_count) in [(1, 0), (4096, 0), (MOVIE_LAST.len(), 12 + 58)] {
            let reported = read_in_pieces(MOVIE_LAST, config(None, None), Subscribe::Every, piece);
            lent_as_held("movie_moov_last.mp4", MOVIE_LAST, &reported);
            let errors = reported
                .iter()
                .filter(|reported| matches!(reported, Reported::Error(_)))
                .count();
            assert_eq!(
                (
                    locations(&reported).len(),
                    samples_lent(&reported).len(),
                    errors
                ),
                (12 + 58, sample_count, 0),
                "in pieces of {piece}"
            );
        }
    }

    #[test]
    fn a_sample_of_no_bytes_at_the_end_of_the_media_data_is_lent_before_what_follows_it() {
        use isobmff_test_support::fixtures::{encoded, movie, track, written};

        let moov_at = |chunk_offset| {
            let mut moov = movie([track(1, 10, &[3, 0], &[(1, 2)], &[chunk_offset])]);
            moov.mvhd.next_track_id = 2;
            encoded(&moov)
        };
        let chunk_offset = u64::try_from(moov_at(0).len() + 8).unwrap();
        let media_data = [moov_at(chunk_offset), written(b"mdat", &[b"abc"])].concat();
        for (name, file, after) in [
            (
                "a file that ends with the media data",
                media_data.clone(),
                None,
            ),
            (
                "media data followed by free space",
                [media_data.clone(), written(b"free", &[])].concat(),
                Some(Reported::Box("free".into())),
            ),
        ] {
            let reported = read_every_way(name, &file, config(None, None), Subscribe::Every);
            lent_as_held(name, &file, &reported);
            let lent = reported
                .iter()
                .filter(|reported| matches!(reported, Reported::Sample(..) | Reported::Box(_)))
                .map(|reported| {
                    if let Reported::Sample(location, _) = reported {
                        Reported::Sample(*location, Vec::new())
                    } else {
                        reported.clone()
                    }
                })
                .collect::<Vec<_>>();
            let mut expected = locations(&reported)
                .into_iter()
                .map(|location| Reported::Sample(location, Vec::new()))
                .collect::<Vec<_>>();
            expected.insert(0, Reported::Box("mdat".into()));
            expected.extend(after);
            assert_eq!((locations(&reported).len(), lent), (2, expected), "{name}");
        }
    }

    #[test]
    fn the_samples_of_a_track_not_subscribed_to_are_located_and_not_lent() {
        let (name, file) = FIXTURES[0];
        let reported = read_every_way(name, file, config(None, None), Subscribe::Only(&[1]));
        assert!(
            samples_lent(&reported)
                .iter()
                .all(|location| location.properties.track_id == 1)
        );
        assert_eq!(samples_lent(&reported).len(), 12);
        assert!(reported.iter().any(|reported| matches!(reported, Reported::SampleLocation(location) if location.properties.track_id == 2)));
    }

    #[test]
    fn a_sample_over_the_limit_is_reported_and_the_samples_after_it_are_lent() {
        let (name, file) = FIXTURES[0];
        let reported = read_every_way(name, file, config(None, Some(1024)), Subscribe::Every);
        let over = reported
            .iter()
            .filter_map(|reported| {
                if let Reported::Error(Error::SampleOverLimit {
                    location,
                    limit: 1024,
                }) = reported
                {
                    Some(location.size)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(!over.is_empty() && over.iter().all(|&size| size > 1024));
        assert_eq!(samples_lent(&reported).len() + over.len(), 12 + 58);
    }

    #[test]
    fn a_moov_over_the_payload_limit_is_reported_and_the_boxes_after_it_are_read() {
        let (name, file) = FIXTURES[0];
        let reported = read_every_way(name, file, config(Some(64), None), Subscribe::Every);
        assert_eq!(
            reported.iter().map(Reported::line).collect::<Vec<_>>(),
            [
                "FileType",
                "a box at file level cannot be framed: the payload of the box `moov` at offset 32 is 1792 bytes, over the limit of 64",
                "Box(\"free\")",
                "Box(\"mdat\")"
            ]
        );
    }

    #[test]
    fn a_second_moov_is_refused_and_the_first_movie_kept() {
        let (name, file) = FIXTURES[0];
        let moov = file.get(32..1832).unwrap();
        assert_eq!(moov.get(4..8), Some(&b"moov"[..]));
        let doubled = [file, moov].concat();
        let reported = read_every_way(name, &doubled, config(None, None), Subscribe::Every);
        let count = |matches: fn(&Reported) -> bool| {
            reported.iter().filter(|reported| matches(reported)).count()
        };
        assert_eq!(
            (
                count(|reported| matches!(reported, Reported::Movie(_))),
                count(|reported| matches!(reported, Reported::SampleLocation(_))),
                count(|reported| matches!(reported, Reported::Sample(..))),
            ),
            (1, 70, 70)
        );
        assert_eq!(
            reported
                .iter()
                .filter_map(|reported| {
                    if let Reported::Error(error) = reported {
                        Some(*error)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>(),
            [Error::DuplicateMovie {
                offset: u64::try_from(file.len()).unwrap()
            }]
        );
    }

    #[test]
    fn the_events_across_a_truncated_file() {
        let (name, file) = FIXTURES[0];
        // Why 23600: inside the last sample of track 1, which starts at 23497
        let truncated = file.get(..23_600).unwrap();
        let reported = read_every_way(name, truncated, config(None, None), Subscribe::Only(&[1]));
        let lines = reported
            .iter()
            .filter(|reported| !matches!(reported, Reported::SampleLocation(_)))
            .map(Reported::line)
            .collect::<Vec<_>>();
        insta::assert_debug_snapshot!(lines);
    }

    #[test]
    fn each_segment_reports_its_styp_and_its_moof_and_their_samples_follow() {
        let (name, file) = FIXTURES[2];
        let reported = read_every_way(name, file, config(None, None), Subscribe::Every);
        let boxes = reported
            .iter()
            .filter(|reported| {
                !matches!(reported, Reported::SampleLocation(_) | Reported::Sample(..))
            })
            .cloned()
            .collect::<Vec<_>>();
        let segment = |sequence_number| {
            [
                Reported::SegmentType,
                Reported::MovieFragment(sequence_number),
                Reported::Box("mdat".to_owned()),
            ]
        };
        assert_eq!(
            boxes,
            [
                vec![Reported::FileType, Reported::Movie(vec![1, 2])],
                segment(1).to_vec(),
                segment(2).to_vec(),
                segment(3).to_vec(),
            ]
            .concat()
        );
    }

    #[test]
    fn a_moof_whose_samples_would_pass_the_locations_held_is_reported_and_its_time_is_not_counted()
    {
        let file = fragments_ahead_of_their_samples(64);
        let mut limited = DemuxerConfig::default();
        limited.sample_count_limit = 8;
        let reported = read_where_required(&file, limited, Subscribe::Every);
        let refused = reported
            .iter()
            .filter(|reported| {
                matches!(
                    reported,
                    Reported::Error(Error::StructureData {
                        source: structure_data::Error::SampleCountOverLimit { limit: 8, .. },
                        ..
                    })
                )
            })
            .count();
        let lent = samples_lent(&reported)
            .iter()
            .map(|location| location.properties.decoding_time)
            .collect::<Vec<_>>();
        assert_eq!(
            (refused, lent),
            (64 - 8, [0, 20, 40, 60, 80, 100, 120, 140, 160].to_vec())
        );
    }

    #[test]
    fn the_events_across_movie_fragments_before_any_movie() {
        let (name, file) = FIXTURES[2];
        // Why 1298: the `moov` of the fixture spans 28 to 1298, after the `ftyp`
        let without_movie = [file.get(..28).unwrap(), file.get(1298..).unwrap()].concat();
        let reported = read_every_way(name, &without_movie, config(None, None), Subscribe::Every);
        let lines = reported.iter().map(Reported::line).collect::<Vec<_>>();
        insta::assert_debug_snapshot!(lines);
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR"]
    fn the_samples_of_the_conformance_files_are_lent_as_the_files_hold_them() {
        let mut differences = Vec::new();
        for file in conformance_files() {
            let seeking = read_where_required(&file.bytes, config(None, None), Subscribe::Every);
            let in_pieces = read_in_pieces(&file.bytes, config(None, None), Subscribe::Every, 4096);
            for reported in seeking.iter().chain(&in_pieces) {
                if let Reported::Sample(location, bytes) = reported
                    && bytes.as_slice() != bytes_at(&file.bytes, location)
                {
                    differences.push(format!(
                        "{}: the sample at {} differs",
                        file.name, location.offset
                    ));
                }
            }
            if samples_lent(&seeking).len() != locations(&seeking).len() {
                differences.push(format!(
                    "{}: {} of {} samples lent to a caller that seeks",
                    file.name,
                    samples_lent(&seeking).len(),
                    locations(&seeking).len()
                ));
            }
        }
        assert_eq!(differences, Vec::<String>::new());
    }

    /// What the demuxer reports of a file whose boxes disagree with themselves, in order
    #[derive(Debug, PartialEq, Eq)]
    enum Read {
        Movie,
        MovieFragment(u32),
        /// A run of locations, as how many of each track it holds
        Locations(BTreeMap<u32, usize>),
        Error(Error),
    }

    /// What the demuxer reports of `file` under `config`, subscribed to no track: each box read into a value, each error, and each run of locations
    fn read_disagreeing(file: &[u8], config: DemuxerConfig) -> Vec<Read> {
        let mut read: Vec<Read> = Vec::new();
        for reported in read_where_required(file, config, Subscribe::Only(&[])) {
            let next = match reported {
                Reported::Movie(_) => Read::Movie,
                Reported::MovieFragment(sequence_number) => Read::MovieFragment(sequence_number),
                Reported::Error(error) => Read::Error(error),
                Reported::SampleLocation(location) => {
                    let track_id = location.properties.track_id;
                    if let Some(Read::Locations(counts)) = read.last_mut() {
                        let count = counts.entry(track_id).or_insert(0);
                        *count = count.saturating_add(1);
                        continue;
                    }
                    Read::Locations(BTreeMap::from([(track_id, 1)]))
                }
                Reported::FileType
                | Reported::SegmentType
                | Reported::Sample(..)
                | Reported::Box(_)
                | Reported::Other(_) => continue,
            };
            read.push(next);
        }
        read
    }

    /// The error of a `moov` or a `moof` at `offset` whose samples the structure-data refuses for `source`
    fn refused(offset: u64, box_type: &[u8; 4], source: structure_data::Error) -> Read {
        Read::Error(Error::StructureData {
            offset,
            box_type: isobmff_core::BoxType::compact(*box_type),
            source,
        })
    }

    /// A run of locations, `(track_ID, count)` for each track it holds
    fn run_of(counts: &[(u32, usize)]) -> Read {
        Read::Locations(counts.iter().copied().collect())
    }

    #[test]
    fn a_box_whose_tables_disagree_is_reported_then_read_as_ffmpeg_reads_it() {
        let sample_count_mismatch = |track_id, box_type, sample_count, table_sample_count| {
            structure_data::Error::SampleCountMismatch {
                track_id,
                box_type,
                sample_count,
                table_sample_count,
            }
        };
        // Why 22595: the `moov` of movie_moov_last.mp4, which the fixtures
        // change in place, starts there.
        const MOOV_OFFSET: u64 = 22595;
        let read = |file| read_disagreeing(file, DemuxerConfig::default());
        assert_eq!(
            [
                read(include_bytes!("fixtures/mismatch_tables.mp4")),
                read(include_bytes!("fixtures/mismatch_chunks.mp4")),
                read(include_bytes!("fixtures/mismatch_tfdt.mp4")),
            ],
            [
                vec![
                    refused(
                        MOOV_OFFSET,
                        b"moov",
                        sample_count_mismatch(1, TimeToSampleBox::BOX_TYPE, 12, 10)
                    ),
                    Read::Movie,
                    run_of(&[(1, 12), (2, 55)]),
                ],
                vec![
                    refused(
                        MOOV_OFFSET,
                        b"moov",
                        sample_count_mismatch(1, SampleToChunkBox::BOX_TYPE, 12, 24)
                    ),
                    Read::Movie,
                    run_of(&[(1, 12), (2, 58)]),
                ],
                vec![
                    Read::Movie,
                    Read::MovieFragment(1),
                    run_of(&[(1, 4), (2, 20)]),
                    Read::MovieFragment(2),
                    run_of(&[(1, 4), (2, 19)]),
                    refused(
                        16904,
                        b"moof",
                        structure_data::Error::DecodingTimeMismatch {
                            track_id: 1,
                            expected: 8511,
                            found: 8411
                        }
                    ),
                    Read::MovieFragment(3),
                    run_of(&[(1, 4), (2, 19)]),
                ],
            ]
        );
    }

    #[test]
    fn a_movie_fragment_refused_either_way_is_left_out_and_reading_goes_on() {
        use isobmff_boxes::{MovieFragmentBox, TrackFragmentBox, TrackRunBox, TrackRunSamples};
        use isobmff_test_support::fixtures::encoded;

        // Why split the fixture at 16904: its third `moof` starts there, after
        // the second and that one's `mdat`.
        let fixture: &[u8] = include_bytes!("fixtures/mismatch_tfdt.mp4");
        let (before, after) = fixture.split_at(16904);
        let moof = |track_id, sample_count| {
            let mut trun = TrackRunBox::default();
            trun.samples = TrackRunSamples::EmptyRows { sample_count };
            let mut traf = TrackFragmentBox::default();
            traf.tfhd.track_id = track_id;
            traf.trun = vec![trun];
            let mut moof = MovieFragmentBox::default();
            moof.traf = vec![traf];
            encoded(&moof)
        };
        let (over_limit, unknown_track) = (moof(1, 64), moof(9, 1));
        let file = [before, &over_limit, &unknown_track, after].concat();
        let offset = |length: usize| u64::try_from(length).unwrap();
        let mut config = DemuxerConfig::default();
        config.sample_count_limit = 32;
        assert_eq!(
            read_disagreeing(&file, config).get(3..),
            Some(
                [
                    Read::MovieFragment(2),
                    run_of(&[(1, 4), (2, 19)]),
                    refused(
                        offset(before.len()),
                        b"moof",
                        structure_data::Error::SampleCountOverLimit {
                            sample_count: 64,
                            limit: 32
                        }
                    ),
                    refused(
                        offset(before.len() + over_limit.len()),
                        b"moof",
                        structure_data::Error::UnknownTrack { track_id: 9 }
                    ),
                    refused(
                        offset(before.len() + over_limit.len() + unknown_track.len()),
                        b"moof",
                        structure_data::Error::DecodingTimeMismatch {
                            track_id: 1,
                            expected: 8511,
                            found: 8411
                        }
                    ),
                    Read::MovieFragment(3),
                    run_of(&[(1, 4), (2, 19)]),
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn the_movie_fragments_after_a_movie_read_as_ffmpeg_reads_it_take_its_tracks() {
        use isobmff_boxes::{
            MovieExtendsBox, MovieFragmentBox, TrackExtendsBox, TrackFragmentBox, TrackRunBox,
            TrackRunSamples,
        };
        use isobmff_test_support::fixtures::{encoded, movie, track};

        let mut trak = track(1, 20, &[], &[], &[]);
        let stts = trak.mdia.minf.stbl.stts.as_mut().unwrap();
        stts.entries.first_mut().unwrap().sample_count = 3;
        let mut moov = movie([trak]);
        let mut trex = TrackExtendsBox::default();
        (trex.track_id, trex.default_sample_description_index) = (1, 1);
        (trex.default_sample_duration, trex.default_sample_size) = (20, 4);
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = vec![trex];
        moov.mvex = Some(mvex);
        moov.mvhd.next_track_id = 2;
        let mut trun = TrackRunBox::default();
        trun.samples = TrackRunSamples::EmptyRows { sample_count: 1 };
        let mut traf = TrackFragmentBox::default();
        traf.tfhd.track_id = 1;
        traf.trun = vec![trun];
        let mut moof = MovieFragmentBox::default();
        moof.mfhd.sequence_number = 1;
        moof.traf = vec![traf];
        let file = [encoded(&moov), encoded(&moof)].concat();
        assert_eq!(
            read_disagreeing(&file, DemuxerConfig::default()),
            [
                refused(
                    0,
                    b"moov",
                    structure_data::Error::SampleCountMismatch {
                        track_id: 1,
                        box_type: TimeToSampleBox::BOX_TYPE,
                        sample_count: 0,
                        table_sample_count: 3,
                    }
                ),
                Read::Movie,
                Read::MovieFragment(1),
                run_of(&[(1, 1)]),
            ]
        );
    }
}
