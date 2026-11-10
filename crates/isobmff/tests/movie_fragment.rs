//! The samples movie fragments declare, resolved to where their bytes lie, checked against FFmpeg on the fixtures and the conformance files

#[cfg(test)]
#[path = "support/movie.rs"]
#[expect(
    dead_code,
    reason = "the moov alone is for the tests of the sample tables"
)]
mod support;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::structure_data::{Error, StructureDataReader};
    use isobmff::{SampleLocation, SampleProperties};
    use isobmff_boxes::{
        MovieBox, MovieExtendsBox, MovieFragmentBox, SampleFlags, TrackExtendsBox,
        TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackRunBox, TrackRunSample,
        TrackRunSamples,
    };
    use isobmff_core::{BoxDecode, BoxDefinition};
    use isobmff_test_support::conformance_files;
    use isobmff_test_support::fixtures::{movie, track};

    use crate::support::{SAMPLE_COUNT_LIMIT, by_track, ffprobe_by_track};

    /// The `moov` and each `moof` of `file` with its file offset, read with the framing reader
    fn structure_data_of(file: &[u8]) -> (Option<MovieBox>, Vec<(u64, MovieFragmentBox)>) {
        let mut reader = BoxReader::new(FramerConfig::new(64 * 1024 * 1024));
        let (mut movie, mut fragments) = (None, Vec::new());
        loop {
            while let Some(event) = reader.poll_event() {
                if let Event::Gathered {
                    offset,
                    box_type,
                    payload,
                    ..
                } = event
                {
                    if box_type == MovieBox::BOX_TYPE {
                        movie = Some(MovieBox::decode_payload(payload).unwrap());
                    } else if box_type == MovieFragmentBox::BOX_TYPE {
                        fragments
                            .push((offset, MovieFragmentBox::decode_payload(payload).unwrap()));
                    }
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
        (movie, fragments)
    }

    /// The locations of the samples of `movie` and then of each of `fragments`
    fn locations(
        movie: &MovieBox,
        fragments: &[(u64, MovieFragmentBox)],
    ) -> Result<Vec<SampleLocation>, Error> {
        let mut reader = StructureDataReader::new(SAMPLE_COUNT_LIMIT);
        let mut found: Vec<_> = reader.handle_movie(movie)?.collect();
        for (offset, moof) in fragments {
            found.extend(reader.handle_movie_fragment(moof, *offset)?);
        }
        Ok(found)
    }

    const FIXTURES: [&str; 2] = ["fragmented.mp4", "segment.mp4"];

    fn fixture(name: &str) -> (Vec<u8>, std::path::PathBuf) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        (std::fs::read(&path).unwrap(), path)
    }

    #[test]
    #[ignore = "needs ffprobe"]
    fn the_fixtures_resolve_to_the_packets_ffmpeg_reads() {
        for name in FIXTURES {
            let (file, path) = fixture(name);
            let (movie, fragments) = structure_data_of(&file);
            let ours = by_track(&locations(&movie.unwrap(), &fragments).unwrap());
            assert_eq!(Some(ours), ffprobe_by_track(&path), "{name}");
        }
    }

    #[test]
    fn concatenated_segments_go_on_in_decoding_time_without_a_tfdt() {
        let (file, _) = fixture("segment.mp4");
        let (movie, fragments) = structure_data_of(&file);
        let movie = movie.unwrap();
        let without_tfdt = fragments
            .iter()
            .map(|(offset, moof)| {
                let mut moof = moof.clone();
                for traf in &mut moof.traf {
                    traf.tfdt = None;
                }
                (*offset, moof)
            })
            .collect::<Vec<_>>();
        let with = locations(&movie, &fragments).unwrap();
        assert_eq!(fragments.len(), 3);
        assert_eq!(locations(&movie, &without_tfdt).unwrap(), with);
    }

    /// A movie of track 1, of samples of 2 and 3 bytes at 100 lasting 10, whose fragments take the defaults of a `trex`: samples of 4 bytes lasting 20, sample entry 1
    fn movie_with_samples_and_fragments() -> MovieBox {
        let mut movie = movie([track(1, 10, &[2, 3], &[(1, 2)], &[100])]);
        let mut trex = TrackExtendsBox::default();
        trex.track_id = 1;
        trex.default_sample_description_index = 1;
        trex.default_sample_duration = 20;
        trex.default_sample_size = 4;
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = vec![trex];
        movie.mvex = Some(mvex);
        movie
    }

    /// A track fragment of track `track_id` of one run of `sample_count` samples that take every default, at `data_offset` from its base
    fn traf(track_id: u32, sample_count: u32, data_offset: Option<i32>) -> TrackFragmentBox {
        let mut traf = TrackFragmentBox::default();
        traf.tfhd.track_id = track_id;
        let mut trun = TrackRunBox::default();
        trun.data_offset = data_offset;
        trun.samples = TrackRunSamples::EmptyRows { sample_count };
        traf.trun = vec![trun];
        traf
    }

    fn moof(trafs: impl IntoIterator<Item = TrackFragmentBox>) -> MovieFragmentBox {
        let mut moof = MovieFragmentBox::default();
        moof.traf = trafs.into_iter().collect();
        moof
    }

    /// A sample of track 1 at `offset`, of `size` bytes, at `decoding_time` lasting `sample_duration`, a sync sample the first sample entry describes
    fn at(offset: u64, size: u32, decoding_time: u64, sample_duration: u32) -> SampleLocation {
        SampleLocation {
            properties: SampleProperties {
                track_id: 1,
                decoding_time,
                sample_duration,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            offset,
            size,
        }
    }

    #[test]
    fn the_samples_of_a_movie_and_of_the_fragments_that_follow_it_are_both_resolved() {
        let fragments = [
            (1000, moof([traf(1, 2, Some(16))])),
            (2000, moof([traf(1, 1, None)])),
        ];
        let found = locations(&movie_with_samples_and_fragments(), &fragments).unwrap();
        assert_eq!(
            found,
            [
                at(100, 2, 0, 10),
                at(102, 3, 10, 10),
                at(1016, 4, 20, 20),
                at(1020, 4, 40, 20),
                at(2000, 4, 60, 20)
            ]
        );
    }

    #[test]
    fn a_track_fragment_with_no_base_follows_the_data_of_the_one_before_it_unless_based_on_the_moof()
     {
        let mut on_moof = traf(1, 1, Some(8));
        on_moof.tfhd.default_base_is_moof = true;
        let mut on_file = traf(1, 1, Some(8));
        on_file.tfhd.base_data_offset = Some(5000);
        let fragments = [(
            1000,
            moof([traf(1, 2, Some(16)), traf(1, 1, None), on_moof, on_file]),
        )];
        let found = locations(&movie_with_samples_and_fragments(), &fragments).unwrap();
        assert_eq!(
            found.get(2..).unwrap(),
            [
                at(1008, 4, 80, 20),
                at(1016, 4, 20, 20),
                at(1020, 4, 40, 20),
                at(1024, 4, 60, 20),
                at(5008, 4, 100, 20)
            ]
        );
    }

    #[test]
    fn the_values_of_a_run_take_the_place_of_those_of_the_tfhd_and_the_trex() {
        let mut fragment = traf(1, 0, None);
        fragment.tfhd.default_sample_duration = Some(7);
        fragment.tfhd.default_sample_size = Some(9);
        fragment.tfdt = Some({
            let mut tfdt = TrackFragmentBaseMediaDecodeTimeBox::default();
            tfdt.base_media_decode_time = 500;
            tfdt
        });
        let mut non_sync = SampleFlags::default();
        non_sync.sample_is_non_sync_sample = true;
        let trun = fragment.trun.first_mut().unwrap();
        trun.first_sample_flags = Some(SampleFlags::default());
        fragment.tfhd.default_sample_flags = Some(non_sync);
        let mut long = TrackRunSample::default();
        long.sample_duration = Some(30);
        long.sample_composition_time_offset = Some(-3);
        trun.samples = TrackRunSamples::Rows {
            samples: vec![long, TrackRunSample::default()],
        };
        let found = locations(
            &movie_with_samples_and_fragments(),
            &[(1000, moof([fragment]))],
        )
        .unwrap();
        let mut first = at(1000, 9, 500, 30);
        first.properties.sample_composition_time_offset = -3;
        let mut second = at(1009, 9, 530, 7);
        second.properties.sample_flags = non_sync;
        assert_eq!(found.get(2..).unwrap(), [first, second]);
    }

    #[test]
    fn a_fragment_refused_leaves_the_decoding_times_of_its_tracks_as_they_were() {
        let mut reader = StructureDataReader::new(SAMPLE_COUNT_LIMIT);
        assert_eq!(
            reader
                .handle_movie_fragment(&moof([traf(1, 1, None)]), 0)
                .map(|_| ()),
            Err(Error::MovieFragmentWithoutMovie)
        );
        let _ = reader
            .handle_movie(&movie_with_samples_and_fragments())
            .unwrap()
            .count();
        assert_eq!(
            [
                (moof([traf(1, 1, None), traf(2, 1, None)]), 0),
                (moof([traf(1, 1, Some(-1))]), 0),
                (moof([traf(1, 1, None)]), u64::MAX - 3),
                (moof([traf(1, u32::MAX, None)]), 0),
            ]
            .map(|(moof, offset)| reader.handle_movie_fragment(&moof, offset).map(|_| ())),
            [
                Err(Error::UnknownTrack { track_id: 2 }),
                Err(Error::Overflow { track_id: 1 }),
                Err(Error::Overflow { track_id: 1 }),
                Err(Error::SampleCountOverLimit {
                    sample_count: u64::from(u32::MAX),
                    limit: SAMPLE_COUNT_LIMIT,
                }),
            ]
        );
        let next: Vec<_> = reader
            .handle_movie_fragment(&moof([traf(1, 1, None)]), 3000)
            .unwrap()
            .collect();
        assert_eq!(next, [at(3000, 4, 20, 20)]);
    }

    #[test]
    fn a_sample_that_takes_a_default_nothing_gives_is_refused() {
        let mut reader = StructureDataReader::new(SAMPLE_COUNT_LIMIT);
        let mut no_trex = movie_with_samples_and_fragments();
        no_trex.mvex = None;
        let _ = reader.handle_movie(&no_trex).unwrap().count();
        assert_eq!(
            reader
                .handle_movie_fragment(&moof([traf(1, 1, None)]), 0)
                .map(|_| ()),
            Err(Error::MissingDefault { track_id: 1 })
        );
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_fragments_of_the_conformance_files_resolve_to_the_packets_ffmpeg_reads() {
        let mut differences = Vec::new();
        let mut compared = 0;
        for file in conformance_files() {
            let (Some(movie), fragments) = structure_data_of(&file.bytes) else {
                continue;
            };
            if fragments.is_empty() {
                continue;
            }
            let ours = match locations(&movie, &fragments) {
                Ok(found) => by_track(&found),
                Err(error) => {
                    differences.push(format!("{}: {error}", file.name));
                    continue;
                }
            };
            // Why compare these with this crate's demuxer alone: ffprobe
            // refuses to read them.
            let Some(theirs) = ffprobe_by_track(&file.path) else {
                continue;
            };
            compared += 1;
            if ours != theirs {
                differences.push(format!(
                    "{}: {:?} samples, ffprobe {:?}; first differing {:?}",
                    file.name,
                    ours.values().map(Vec::len).collect::<Vec<_>>(),
                    theirs.values().map(Vec::len).collect::<Vec<_>>(),
                    ours.iter()
                        .zip(&theirs)
                        .find(|(ours, theirs)| ours != theirs)
                        .map(|((track, ours), (_, theirs))| (
                            track,
                            ours.iter()
                                .zip(theirs)
                                .find(|(ours, theirs)| ours != theirs)
                        ))
                ));
            }
        }
        assert_eq!(differences, Vec::<String>::new());
        assert!(compared > 0);
    }
}
