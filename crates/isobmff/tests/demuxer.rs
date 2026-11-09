//! A file read into its boxes and the samples of its tracks, however its bytes are handed over

#[cfg(test)]
mod tests {
    use isobmff::demuxer::{Error, Event};
    use isobmff::{Demuxer, DemuxerConfig, SampleLocation};
    use isobmff_test_support::conformance_files;

    /// The fixtures whose samples all lie after the `moov`
    const FIXTURES: [(&str, &[u8]); 2] = [
        (
            "movie_moov_first.mp4",
            include_bytes!("fixtures/movie_moov_first.mp4"),
        ),
        ("fragmented.mp4", include_bytes!("fixtures/fragmented.mp4")),
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
        Movie(Vec<u32>),
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
                Self::Error(error) => format!("{error}"),
                Self::FileType
                | Self::Movie(_)
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
        for ((name, file), sample_count) in FIXTURES.into_iter().zip([12 + 58, 0]) {
            let reported = read_every_way(name, file, config(None, None), Subscribe::Every);
            lent_as_held(name, file, &reported);
            let lent = samples_lent(&reported);
            assert_eq!(lent.len(), sample_count, "{name}");
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
                "a box at file level cannot be framed",
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
}
