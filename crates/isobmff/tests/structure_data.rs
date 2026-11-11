//! The samples a `moov` declares, resolved to where their bytes lie, checked against FFmpeg on the conformance files

#[cfg(test)]
#[path = "support/movie.rs"]
mod support;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use isobmff::SampleLocation;
    use isobmff::structure_data::{Error, Leniency, StructureDataReader};
    use std::num::NonZeroU32;

    use crate::support::{by_track, ffprobe_by_track, locations, movie_of};

    use isobmff_boxes::{
        CompositionOffsetBox, CompositionOffsetEntry, SampleDependencyTypeBox,
        SampleDependencyTypeEntry, SampleSizeBox, SampleSizeVariant, SampleSizes, SampleToChunkBox,
        SyncSampleBox, TimeToSampleBox,
    };
    use isobmff_core::BoxDefinition;
    use isobmff_test_support::conformance_files;
    use isobmff_test_support::fixtures::{encoded, in_another_file, movie, track};
    use isobmff_test_support::serde_json::Value;

    #[test]
    fn the_samples_of_a_chunk_lie_one_after_another_from_its_offset() {
        let movie = movie([track(1, 10, &[2, 3, 4], &[(1, 2), (2, 1)], &[100, 200])]);
        let found = locations(&movie).unwrap();
        assert_eq!(
            found
                .iter()
                .map(|location| (
                    location.offset,
                    location.size,
                    location.properties.decoding_time
                ))
                .collect::<Vec<_>>(),
            [(100, 2, 0), (102, 3, 10), (200, 4, 20)]
        );
        assert!(
            found
                .iter()
                .all(|location| location.properties.sample_description_index == 1
                    && location.properties.sample_duration == 10
                    && !location.properties.sample_flags.sample_is_non_sync_sample)
        );
    }

    #[test]
    fn the_samples_of_several_tracks_come_in_the_order_of_their_offsets() {
        let movie = movie([
            track(1, 1, &[10, 10], &[(1, 1)], &[0, 30]),
            track(2, 1, &[5, 5], &[(1, 2)], &[10]),
        ]);
        let order = locations(&movie)
            .unwrap()
            .iter()
            .map(|location| (location.properties.track_id, location.offset))
            .collect::<Vec<_>>();
        assert_eq!(order, [(1, 0), (2, 10), (2, 15), (1, 30)]);
    }

    #[test]
    fn the_sync_samples_composition_offsets_and_dependencies_reach_each_sample() {
        let mut trak = track(1, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let stbl = &mut trak.mdia.minf.stbl;
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = vec![1, 3];
        stbl.stss = Some(stss);
        let mut ctts = CompositionOffsetBox::default();
        let mut run = CompositionOffsetEntry::default();
        run.sample_count = 3;
        run.sample_offset = 20;
        ctts.entries = vec![run];
        stbl.ctts = Some(ctts);
        let mut sdtp = SampleDependencyTypeBox::default();
        sdtp.entries = [2, 1, 2]
            .into_iter()
            .map(|sample_depends_on| {
                let mut entry = SampleDependencyTypeEntry::default();
                entry.sample_depends_on = sample_depends_on;
                entry
            })
            .collect();
        stbl.sdtp = Some(sdtp);
        let found = locations(&movie([trak])).unwrap();
        assert_eq!(
            found
                .iter()
                .map(|location| (
                    location.properties.sample_flags.sample_is_non_sync_sample,
                    location.properties.sample_flags.sample_depends_on,
                    location.properties.sample_composition_time_offset,
                ))
                .collect::<Vec<_>>(),
            [(false, 2, 20), (true, 1, 20), (false, 2, 20)]
        );
    }

    #[test]
    fn a_small_moov_that_declares_many_samples_is_refused_before_anything_is_allocated() {
        let mut trak = track(1, 1, &[], &[], &[]);
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Constant {
            sample_size: NonZeroU32::MIN,
            sample_count: u32::MAX,
        };
        trak.mdia.minf.stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
        let mut movie = movie([trak]);
        movie.mvhd.next_track_id = 2;
        assert!(encoded(&movie).len() < 1024);
        assert_eq!(
            StructureDataReader::new(1024, 16)
                .handle_movie(&movie)
                .map(|_| ()),
            Err(Error::SampleCountOverLimit {
                sample_count: u64::from(u32::MAX),
                limit: 1024,
            })
        );
    }

    /// `(offset, size, decoding_time, sample_duration)` of each sample, and what the reader read past
    type ReadPast = (Vec<(u64, u32, u64, u32)>, Vec<Leniency>);

    /// `(offset, size, decoding_time, sample_duration)` of each sample `movie` declares, and what the reader read past
    fn read_past(movie: &isobmff_boxes::MovieBox) -> Result<ReadPast, Error> {
        let mut reader = StructureDataReader::new(1024, 16);
        let samples = reader
            .handle_movie(movie)?
            .map(|location| {
                (
                    location.offset,
                    location.size,
                    location.properties.decoding_time,
                    location.properties.sample_duration,
                )
            })
            .collect();
        Ok((samples, reader.leniencies().copied().collect()))
    }

    /// The `sample_count` of the only run of the time-to-sample table of `trak` set to `sample_count`
    fn durations_counted(
        mut trak: isobmff_boxes::TrackBox,
        sample_count: u32,
    ) -> isobmff_boxes::TrackBox {
        let stts = trak.mdia.minf.stbl.stts.as_mut().unwrap();
        stts.entries.first_mut().unwrap().sample_count = sample_count;
        trak
    }

    fn mismatch(
        box_type: isobmff_core::BoxType,
        sample_count: u64,
        table_sample_count: u64,
    ) -> Leniency {
        Leniency::SampleCountMismatch {
            track_id: 7,
            box_type,
            sample_count,
            table_sample_count,
        }
    }

    #[test]
    fn samples_past_the_end_of_the_time_to_sample_table_last_as_long_as_its_last() {
        let trak = durations_counted(track(7, 10, &[1, 2, 3], &[(1, 3)], &[0]), 2);
        assert_eq!(
            read_past(&movie([trak])),
            Ok((
                vec![(0, 1, 0, 10), (1, 2, 10, 10), (3, 3, 20, 10)],
                vec![mismatch(TimeToSampleBox::BOX_TYPE, 3, 2)]
            ))
        );
    }

    #[test]
    fn durations_past_the_samples_are_not_read() {
        let trak = durations_counted(track(7, 10, &[1, 2], &[(1, 2)], &[0]), 5);
        assert_eq!(
            read_past(&movie([trak])),
            Ok((
                vec![(0, 1, 0, 10), (1, 2, 10, 10)],
                vec![mismatch(TimeToSampleBox::BOX_TYPE, 2, 5)]
            ))
        );
    }

    #[test]
    fn the_samples_are_those_both_the_sample_size_table_and_the_chunks_place() {
        let more_in_chunks = track(7, 10, &[1, 2], &[(1, 3)], &[0]);
        let fewer_in_chunks = track(7, 10, &[1, 2, 3], &[(1, 2)], &[0]);
        let more_in_chunks = durations_counted(more_in_chunks, 2);
        let fewer_in_chunks = durations_counted(fewer_in_chunks, 3);
        assert_eq!(
            [
                read_past(&movie([more_in_chunks])),
                read_past(&movie([fewer_in_chunks]))
            ],
            [
                Ok((
                    vec![(0, 1, 0, 10), (1, 2, 10, 10)],
                    vec![mismatch(SampleToChunkBox::BOX_TYPE, 2, 3)]
                )),
                Ok((
                    vec![(0, 1, 0, 10), (1, 2, 10, 10)],
                    vec![mismatch(SampleToChunkBox::BOX_TYPE, 3, 2)]
                )),
            ]
        );
    }

    #[test]
    fn samples_past_the_composition_offsets_and_the_dependencies_have_none() {
        let mut trak = track(7, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let stbl = &mut trak.mdia.minf.stbl;
        let mut ctts = CompositionOffsetBox::default();
        let mut run = CompositionOffsetEntry::default();
        (run.sample_count, run.sample_offset) = (1, 20);
        ctts.entries = vec![run];
        stbl.ctts = Some(ctts);
        let mut sdtp = SampleDependencyTypeBox::default();
        let mut entry = SampleDependencyTypeEntry::default();
        entry.sample_depends_on = 2;
        sdtp.entries = vec![entry; 4];
        stbl.sdtp = Some(sdtp);
        let mut reader = StructureDataReader::new(1024, 16);
        let found: Vec<_> = reader
            .handle_movie(&movie([trak]))
            .unwrap()
            .map(|location| {
                (
                    location.properties.sample_composition_time_offset,
                    location.properties.sample_flags.sample_depends_on,
                )
            })
            .collect();
        assert_eq!(
            (found, reader.leniencies().copied().collect::<Vec<_>>()),
            (
                vec![(20, 2), (0, 2), (0, 2)],
                vec![
                    mismatch(CompositionOffsetBox::BOX_TYPE, 3, 1),
                    mismatch(SampleDependencyTypeBox::BOX_TYPE, 3, 4),
                ]
            )
        );
    }

    #[test]
    fn a_sync_sample_number_past_the_samples_or_not_after_the_one_before_is_not_read() {
        let mut trak = track(7, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = vec![1, 1, 3, 9];
        trak.mdia.minf.stbl.stss = Some(stss);
        let mut reader = StructureDataReader::new(1024, 16);
        let sync: Vec<_> = reader
            .handle_movie(&movie([trak]))
            .unwrap()
            .map(|location| !location.properties.sample_flags.sample_is_non_sync_sample)
            .collect();
        let ignored = |entry| Leniency::EntryIgnored {
            track_id: 7,
            box_type: SyncSampleBox::BOX_TYPE,
            entry,
        };
        assert_eq!(
            (sync, reader.leniencies().copied().collect::<Vec<_>>()),
            (vec![true, false, true], vec![ignored(1), ignored(3)])
        );
    }

    #[test]
    fn a_sample_table_under_which_no_sample_can_be_placed_is_refused() {
        let no_durations = durations_counted(track(7, 10, &[1, 2], &[(1, 2)], &[0]), 0);
        let no_chunks = track(7, 10, &[1, 2], &[], &[]);
        assert_eq!(
            [
                read_past(&movie([no_durations])),
                read_past(&movie([no_chunks]))
            ],
            [
                Err(Error::NoSamplePlaced {
                    track_id: 7,
                    box_type: TimeToSampleBox::BOX_TYPE,
                }),
                Err(Error::NoSamplePlaced {
                    track_id: 7,
                    box_type: SampleToChunkBox::BOX_TYPE,
                }),
            ]
        );
    }

    #[test]
    fn a_movie_refused_adds_nothing_to_what_was_read_past() {
        let read_past = durations_counted(track(7, 10, &[1, 2], &[(1, 2)], &[0]), 3);
        let refused = in_another_file(track(8, 1, &[1], &[(1, 1)], &[0]));
        let mut reader = StructureDataReader::new(1024, 16);
        assert!(reader.handle_movie(&movie([read_past, refused])).is_err());
        assert_eq!(reader.leniencies().len(), 0);
    }

    #[test]
    fn a_sample_to_chunk_table_that_does_not_start_at_chunk_1_is_refused() {
        let trak = track(7, 1, &[1, 1], &[(2, 1)], &[0, 1]);
        assert_eq!(
            locations(&movie([trak])),
            Err(Error::InvalidEntry {
                track_id: 7,
                box_type: SampleToChunkBox::BOX_TYPE,
                entry: 0,
            })
        );
    }

    #[test]
    fn samples_in_another_file_are_refused() {
        let trak = in_another_file(track(7, 1, &[1], &[(1, 1)], &[0]));
        assert_eq!(
            locations(&movie([trak])),
            Err(Error::MediaDataElsewhere {
                track_id: 7,
                data_reference_index: 1,
            })
        );
    }

    #[test]
    fn a_movie_file_resolves_to_the_same_samples_whether_its_moov_is_first_or_last() {
        let first =
            locations(&movie_of(include_bytes!("fixtures/movie_moov_first.mp4")).unwrap()).unwrap();
        let last =
            locations(&movie_of(include_bytes!("fixtures/movie_moov_last.mp4")).unwrap()).unwrap();
        assert_eq!(first.len(), 12 + 58);
        let shifted = |locations: &[SampleLocation], by: u64| {
            locations
                .iter()
                .map(|location| (location.properties, location.offset - by, location.size))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shifted(&first, first.first().unwrap().offset),
            shifted(&last, last.first().unwrap().offset)
        );
    }

    /// The conformance files whose samples lie in another file, which the reader refuses
    const MEDIA_DATA_ELSEWHERE: [&str; 3] = [
        "02_dref_edts_img.mp4",
        "04_bifs_video.mp4",
        "a6_tone_multifile.mp4",
    ];

    /// Number of samples GPAC reports the sample size table of each track of the `moov` to count
    #[expect(
        clippy::indexing_slicing,
        reason = "indexing a JSON value yields null for a key it lacks, and never panics"
    )]
    fn gpac_sample_counts(report: &Value) -> BTreeMap<u32, u64> {
        let movie = &report["IsoMediaFile"]["MovieBox"];
        let tracks = match (&movie["TrackBox"], movie["TrackBox"].as_array()) {
            (_, Some(tracks)) => tracks.iter().collect(),
            (track, None) if track.is_null() => Vec::new(),
            (track, None) => vec![track],
        };
        tracks
            .into_iter()
            .map(|trak| {
                let track_id = trak["TrackHeaderBox"]["@TrackID"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                let stbl = &trak["MediaBox"]["MediaInformationBox"]["SampleTableBox"];
                let sizes = if stbl["SampleSizeBox"].is_null() {
                    &stbl["CompactSampleSizeBox"]
                } else {
                    &stbl["SampleSizeBox"]
                };
                (
                    track_id,
                    sizes["@SampleCount"].as_str().unwrap().parse().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_resolve_to_the_packets_ffmpeg_reads() {
        let mut differences = Vec::new();
        for file in conformance_files() {
            let Some(movie) = movie_of(&file.bytes) else {
                continue;
            };
            let ours = match locations(&movie) {
                Ok(locations) => by_track(&locations),
                Err(Error::MediaDataElsewhere { .. })
                    if MEDIA_DATA_ELSEWHERE.contains(&file.name.as_str()) =>
                {
                    continue;
                }
                Err(error) => {
                    differences.push(format!("{}: {error}", file.name));
                    continue;
                }
            };
            let counts = ours
                .iter()
                .map(|(&track_id, samples)| (track_id, u64::try_from(samples.len()).unwrap()))
                .collect::<BTreeMap<_, _>>();
            let gpac_counts = gpac_sample_counts(&file.report)
                .into_iter()
                .filter(|&(_, count)| count > 0)
                .collect::<BTreeMap<_, _>>();
            if counts != gpac_counts {
                differences.push(format!(
                    "{}: {counts:?} samples, GPAC reports {gpac_counts:?}",
                    file.name
                ));
            }
            // Why compare these with GPAC alone: ffprobe refuses to read them.
            let Some(theirs) = ffprobe_by_track(&file.path) else {
                continue;
            };
            let fragmented = movie.mvex.is_some();
            for (track_id, samples) in &ours {
                let Some(packets) = theirs.get(track_id) else {
                    differences.push(format!("{}: ffprobe reads no track {track_id}", file.name));
                    continue;
                };
                // Why compare a prefix for a fragmented file: ffprobe reads the
                // samples of the movie fragments after those of the `moov`.
                let packets = if fragmented {
                    packets.get(..samples.len()).unwrap_or(packets)
                } else {
                    packets
                };
                if samples != packets {
                    differences.push(format!(
                        "{} track {track_id}: {} samples, ffprobe {} packets; first differing {:?}",
                        file.name,
                        samples.len(),
                        packets.len(),
                        samples
                            .iter()
                            .zip(packets)
                            .find(|(ours, theirs)| ours != theirs)
                    ));
                }
            }
        }
        assert_eq!(differences, Vec::<String>::new());
    }
}
