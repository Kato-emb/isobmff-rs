//! Fragmented movie files and segments written around samples, read back by this crate's demuxer and checked against ffprobe on the conformance files

#[cfg(test)]
#[path = "support/muxer.rs"]
mod support;

#[cfg(test)]
mod tests {
    use isobmff::muxer::{Error, MovieDescription, TrackDescription};
    use isobmff::{FragmentedMovieFileMuxer, MuxerConfig, SampleProperties, SegmentMuxer};
    use isobmff_boxes::{FileTypeBox, SegmentTypeBox};
    use isobmff_core::FourCc;
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use crate::support::{Read, mp4_file_type, one_sample_entry_track, packet_read, read, sample};

    /// A muxer that writes movie fragments, of a fragmented movie file or of segments
    enum FragmentMuxer {
        MovieFile(FragmentedMovieFileMuxer),
        Segments(SegmentMuxer),
    }

    impl FragmentMuxer {
        /// A muxer of the movie `movie` describes, in segments of `msdh` where `in_segments`
        fn new(
            in_segments: bool,
            movie: MovieDescription,
            config: MuxerConfig,
            file_type: FileTypeBox,
        ) -> Result<Self, Error> {
            if in_segments {
                let mut styp = SegmentTypeBox::default();
                styp.major_brand = FourCc::new(*b"msdh");
                styp.compatible_brands = vec![FourCc::new(*b"msdh"), FourCc::new(*b"msix")];
                SegmentMuxer::new(movie, config, file_type, styp).map(Self::Segments)
            } else {
                FragmentedMovieFileMuxer::new(movie, config, file_type).map(Self::MovieFile)
            }
        }

        fn add_track(&mut self, track: TrackDescription) -> Result<(), Error> {
            match self {
                Self::MovieFile(muxer) => muxer.add_track(track),
                Self::Segments(muxer) => muxer.add_track(track),
            }
        }

        fn handle_sample(
            &mut self,
            properties: SampleProperties,
            bytes: &[u8],
        ) -> Result<(), Error> {
            match self {
                Self::MovieFile(muxer) => muxer.handle_sample(properties, bytes),
                Self::Segments(muxer) => muxer.handle_sample(properties, bytes),
            }
        }

        /// Ends the fragment, or the segment
        fn end(&mut self) -> Result<(), Error> {
            match self {
                Self::MovieFile(muxer) => muxer.finish_fragment(),
                Self::Segments(muxer) => muxer.finish_segment(),
            }
        }

        fn poll_output(&mut self, out: &mut Vec<u8>) {
            match self {
                Self::MovieFile(muxer) => muxer.poll_output(out),
                Self::Segments(muxer) => muxer.poll_output(out),
            }
        }

        fn finish(&mut self) -> Result<(), Error> {
            match self {
                Self::MovieFile(muxer) => muxer.finish(),
                Self::Segments(muxer) => muxer.finish(),
            }
        }
    }

    /// The fragmented file, or the segments where `in_segments`, a muxer writes of what `source` holds, a fragment ending where one of `source` does and where the next sample would pass a limit
    fn remux_in_fragments(
        source: &Read,
        in_segments: bool,
        config: MuxerConfig,
    ) -> Result<Vec<u8>, Error> {
        let mut muxer = FragmentMuxer::new(
            in_segments,
            MovieDescription::from(&source.movie),
            config,
            source.file_type.clone(),
        )?;
        for trak in &source.movie.trak {
            muxer.add_track(TrackDescription::from(trak))?;
        }
        let mut written = Vec::new();
        for (index, (properties, bytes)) in source.samples.iter().enumerate() {
            if index > 0 && source.fragment_starts.contains(&index) {
                muxer.end()?;
            }
            match muxer.handle_sample(*properties, bytes) {
                Err(
                    Error::BufferFull { .. }
                    | Error::StructureData(isobmff::structure_data::Error::SampleCountOverLimit {
                        ..
                    }),
                ) => {
                    muxer.end()?;
                    muxer.handle_sample(*properties, bytes)?;
                }
                result => result?,
            }
            muxer.poll_output(&mut written);
        }
        muxer.finish()?;
        muxer.poll_output(&mut written);
        Ok(written)
    }

    /// The type of each box at file level of `file`
    fn top_level_box_types(file: &[u8]) -> Vec<String> {
        use isobmff::framing::{BoxReader, Event as BoxEvent, FramerConfig};

        let mut config = FramerConfig::new(0);
        config.gathered_box_types.clear();
        let mut reader = BoxReader::new(config);
        reader.handle_input(0, file);
        let mut types = Vec::new();
        loop {
            while let Some(event) = reader.poll_event() {
                match event {
                    BoxEvent::Box { header, .. } => types.push(header.box_type().to_string()),
                    BoxEvent::Error(error) => types.push(format!("{error}")),
                    BoxEvent::Gathered { .. } | _ => {}
                }
            }
            let Some(required) = reader.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => reader.handle_input(required.position, bytes),
                _ => reader.finish(),
            }
        }
        types
    }

    #[test]
    fn a_movie_written_in_fragments_or_in_segments_holds_the_same_samples() {
        for file in [
            include_bytes!("fixtures/movie_moov_first.mp4").as_slice(),
            include_bytes!("fixtures/movie_moov_last.mp4"),
            include_bytes!("fixtures/fragmented.mp4"),
            include_bytes!("fixtures/segment.mp4"),
        ] {
            let source = read(file).unwrap();
            for in_segments in [false, true] {
                let written = remux_in_fragments(&source, in_segments, MuxerConfig::default());
                let written = read(&written.unwrap()).unwrap();
                assert_eq!(
                    (written.samples, written.fragment_starts, written.errors),
                    (
                        source.samples.clone(),
                        if source.fragment_starts.is_empty() {
                            [0].into()
                        } else {
                            source.fragment_starts.clone()
                        },
                        Vec::new()
                    )
                );
            }
        }
    }

    #[test]
    fn the_moov_follows_the_ftyp_and_each_fragment_or_segment_follows_it_in_one_run() {
        let mut source = read(include_bytes!("fixtures/movie_moov_first.mp4")).unwrap();
        source.fragment_starts = [0, 5, 30].into();
        let in_fragments = remux_in_fragments(&source, false, MuxerConfig::default()).unwrap();
        let in_segments = remux_in_fragments(&source, true, MuxerConfig::default()).unwrap();
        assert_eq!(
            (
                top_level_box_types(&in_fragments),
                top_level_box_types(&in_segments)
            ),
            (
                [
                    "ftyp", "moov", "moof", "mdat", "moof", "mdat", "moof", "mdat"
                ]
                .map(String::from)
                .to_vec(),
                [
                    "ftyp", "moov", "styp", "moof", "mdat", "styp", "moof", "mdat", "styp", "moof",
                    "mdat"
                ]
                .map(String::from)
                .to_vec()
            )
        );
    }

    #[test]
    fn a_fragmented_muxer_holds_the_moov_from_the_first_sample_and_each_fragment_from_its_end() {
        let mut muxer = FragmentedMovieFileMuxer::new(
            MovieDescription::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
        )
        .unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();
        let mut written = Vec::new();
        muxer.poll_output(&mut written);
        assert_eq!(top_level_box_types(&written), ["ftyp"]);
        muxer.finish_fragment().unwrap();
        muxer.handle_sample(sample(0), b"abc").unwrap();
        muxer.poll_output(&mut written);
        assert_eq!(top_level_box_types(&written), ["ftyp", "moov"]);
        let opening = written.len();
        muxer.handle_sample(sample(10), b"de").unwrap();
        muxer.poll_output(&mut written);
        assert_eq!(written.len(), opening);
        muxer.finish_fragment().unwrap();
        muxer.finish_fragment().unwrap();
        muxer.finish().unwrap();
        muxer.poll_output(&mut written);
        assert_eq!(
            (top_level_box_types(&written), written.ends_with(b"abcde")),
            (
                ["ftyp", "moov", "moof", "mdat"].map(String::from).to_vec(),
                true
            )
        );
    }

    #[test]
    fn a_fragmented_muxer_refuses_a_sample_past_the_buffer_limit_and_changes_nothing() {
        let muxer = |buffer_limit| {
            let mut config = MuxerConfig::default();
            config.buffer_limit = buffer_limit;
            let mut muxer =
                FragmentedMovieFileMuxer::new(MovieDescription::new(1000), config, mp4_file_type())
                    .unwrap();
            muxer.add_track(one_sample_entry_track(1)).unwrap();
            muxer
        };
        let mut opening = Vec::new();
        let mut unbounded = muxer(usize::MAX);
        unbounded.handle_sample(sample(0), b"").unwrap();
        unbounded.poll_output(&mut opening);

        let mut muxer = muxer(opening.len() + 4);
        let mut of_track_2 = sample(0);
        of_track_2.track_id = 2;
        assert_eq!(
            [
                muxer.handle_sample(of_track_2, b"x"),
                muxer.handle_sample(sample(0), b"vwxyz"),
                muxer.handle_sample(sample(0), b"wxyz"),
                muxer.add_track(one_sample_entry_track(2)),
                muxer.handle_sample(sample(10), b"z"),
            ],
            [
                Err(Error::UnknownTrack { track_id: 2 }),
                Err(Error::BufferFull {
                    length: opening.len() + 5,
                }),
                Ok(()),
                Err(Error::TrackAfterMediaData { track_id: 2 }),
                Err(Error::BufferFull {
                    length: opening.len() + 5,
                }),
            ]
        );
        let mut written = Vec::new();
        muxer.poll_output(&mut written);
        assert_eq!(written, opening);
        muxer.finish_fragment().unwrap();
        assert_eq!(muxer.handle_sample(sample(10), b"z"), Ok(()));
    }

    #[test]
    fn a_finished_fragmented_muxer_takes_nothing_more() {
        let mut fragments = FragmentMuxer::new(
            false,
            MovieDescription::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
        )
        .unwrap();
        let mut segments = FragmentMuxer::new(
            true,
            MovieDescription::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
        )
        .unwrap();
        for muxer in [&mut fragments, &mut segments] {
            muxer.add_track(one_sample_entry_track(1)).unwrap();
            muxer.finish().unwrap();
            let mut written = Vec::new();
            muxer.poll_output(&mut written);
            assert_eq!(
                (
                    top_level_box_types(&written),
                    [
                        muxer.add_track(one_sample_entry_track(2)),
                        muxer.handle_sample(sample(0), b"x"),
                        muxer.end(),
                        muxer.finish(),
                    ]
                ),
                (
                    ["ftyp", "moov"].map(String::from).to_vec(),
                    [
                        Err(Error::Finished),
                        Err(Error::Finished),
                        Err(Error::Finished),
                        Err(Error::Finished)
                    ]
                )
            );
        }
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_written_in_fragments_or_segments_read_as_the_same_packets_and_samples()
    {
        let directory =
            std::env::temp_dir().join(format!("isobmff-fragment-muxer-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut written_files, mut fragmented_sources) = (Vec::new(), 0, 0);
        let (mut compared, mut refused) = (0, 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            // Why skip these: a file the demuxer reports an error in, such as
            // one whose samples lie in another file, has none to write.
            if !source.errors.is_empty() || source.samples.is_empty() {
                continue;
            }
            let fragmented = source.movie.mvex.is_some();
            fragmented_sources += usize::from(fragmented);
            // Why compare a file of a `moov` alone without its edit lists, by
            // the offset of each composition time from its decoding time:
            // FFmpeg flags no packet of movie fragments to be discarded, and
            // moves no decoding time before 0 to make room for a negative
            // composition offset, as it does where the `moov` declares the
            // samples.
            let arguments: &[&str] = if fragmented {
                &[]
            } else {
                &["-ignore_editlist", "1"]
            };
            let packets_of = |path: &std::path::Path| {
                ffprobe_packets(arguments, path)
                    .iter()
                    .map(|packet| {
                        let read = packet_read(packet);
                        if fragmented {
                            read
                        } else {
                            let offset = packet.pts.zip(packet.dts).map(|(pts, dts)| pts - dts);
                            (read.0, read.1, None, offset, false, read.5)
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let source_readable = ffprobe(&["-v", "error"], &file.path).status.success();
            let theirs = source_readable.then(|| packets_of(&file.path));
            let mut next_decoding_times = std::collections::BTreeMap::new();
            let gap = source.samples.iter().find_map(|(properties, _)| {
                let next = properties.decoding_time + u64::from(properties.sample_duration);
                let expected = next_decoding_times.insert(properties.track_id, next)?;
                (expected != properties.decoding_time).then_some(
                    isobmff::structure_data::Error::DecodingTimeGap {
                        track_id: properties.track_id,
                        expected,
                        found: properties.decoding_time,
                    },
                )
            });
            for in_segments in [false, true] {
                let name = format!(
                    "{}{}",
                    if in_segments { "segments-" } else { "" },
                    file.name
                );
                let written = remux_in_fragments(&source, in_segments, MuxerConfig::default());
                // Why refuse a source whose decoding times jump: a `tfdt` is
                // the sum of the durations of the samples before it, §8.8.12.
                if let Some(gap) = gap {
                    if written != Err(Error::StructureData(gap)) {
                        differences.push(format!("{name}: {written:?}, not the gap {gap:?}"));
                    }
                    refused += 1;
                    continue;
                }
                let written = match written {
                    Ok(written) => written,
                    Err(error) => {
                        differences.push(format!("{name}: {error:?}"));
                        continue;
                    }
                };
                written_files += 1;
                let read_back = read(&written).unwrap();
                if (&read_back.samples, &read_back.errors) != (&source.samples, &Vec::new()) {
                    differences.push(format!(
                        "{name}: the samples read back differ, {} read back of {}, errors {:?}; first differing {:?}",
                        read_back.samples.len(),
                        source.samples.len(),
                        read_back.errors,
                        read_back
                            .samples
                            .iter()
                            .zip(&source.samples)
                            .map(|(ours, theirs)| (ours.0, theirs.0))
                            .find(|(ours, theirs)| ours != theirs)
                    ));
                }
                // Why compare these with this crate's demuxer alone: ffprobe
                // refuses to read the source.
                let Some(theirs) = &theirs else {
                    continue;
                };
                let path = directory.join(&name);
                std::fs::write(&path, &written).unwrap();
                let ours = packets_of(&path);
                compared += 1;
                if &ours != theirs {
                    differences.push(format!(
                        "{name}: ffprobe reads {} packets, {} from the source; first differing {:?}",
                        ours.len(),
                        theirs.len(),
                        ours.iter()
                            .zip(theirs)
                            .find(|(ours, theirs)| ours != theirs)
                    ));
                }
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{written_files} files written, {compared} compared with ffprobe, {refused} refused for a gap in decoding times, of {fragmented_sources} fragmented sources"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(fragmented_sources > 0);
    }
}
