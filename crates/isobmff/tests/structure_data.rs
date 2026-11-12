//! The samples a `moov` declares, resolved to where their bytes lie, checked against FFmpeg on the conformance files

#[cfg(test)]
#[path = "support/movie.rs"]
mod support;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use isobmff::SampleLocation;
    use isobmff::structure_data::{Error, StructureDataReader};
    use std::num::NonZeroU32;

    use crate::support::{by_track, ffprobe_by_track, locations, movie_of};

    use isobmff_boxes::{
        CompositionOffsetBox, CompositionOffsetEntry, SampleDependencyTypeBox,
        SampleDependencyTypeEntry, SampleSizeBox, SampleSizeVariant, SampleSizes, SampleToChunkBox,
        SyncSampleBox, TimeToSampleBox,
    };
    use isobmff_core::BoxDefinition;
    use isobmff_test_support::fixtures::{encoded, in_another_file, movie, track};
    use isobmff_test_support::serde_json::Value;
    use isobmff_test_support::{conformance_files, gpac_list};

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
            StructureDataReader::new(1024)
                .handle_movie(&movie)
                .map(|_| ()),
            Err(Error::SampleCountOverLimit {
                sample_count: u64::from(u32::MAX),
                limit: 1024,
            })
        );
    }

    /// `(offset, size, decoding_time, sample_duration)` of each sample
    type Samples = Vec<(u64, u32, u64, u32)>;

    /// The samples `movie` declares as the specification reads it, and as it is read lossily
    fn read_both_ways(
        movie: &isobmff_boxes::MovieBox,
    ) -> (Result<Samples, Error>, Result<Samples, Error>) {
        let sample = |location: SampleLocation| {
            (
                location.offset,
                location.size,
                location.properties.decoding_time,
                location.properties.sample_duration,
            )
        };
        let mut reader = StructureDataReader::new(1024);
        let specification = reader
            .handle_movie(movie)
            .map(|locations| locations.map(sample).collect());
        let lossy = reader
            .handle_movie_lossy(movie)
            .map(|locations| locations.map(sample).collect());
        (specification, lossy)
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
    ) -> Error {
        Error::SampleCountMismatch {
            track_id: 7,
            box_type,
            sample_count,
            table_sample_count,
        }
    }

    #[test]
    fn samples_the_time_to_sample_table_gives_no_duration_are_not_placed_when_read_lossily() {
        let trak = durations_counted(track(7, 10, &[1, 2, 3], &[(1, 3)], &[0]), 2);
        assert_eq!(
            read_both_ways(&movie([trak])),
            (
                Err(mismatch(TimeToSampleBox::BOX_TYPE, 3, 2)),
                Ok(vec![(0, 1, 0, 10), (1, 2, 10, 10)])
            )
        );
    }

    #[test]
    fn durations_past_the_samples_are_not_read_when_read_lossily() {
        let trak = durations_counted(track(7, 10, &[1, 2], &[(1, 2)], &[0]), 5);
        assert_eq!(
            read_both_ways(&movie([trak])),
            (
                Err(mismatch(TimeToSampleBox::BOX_TYPE, 2, 5)),
                Ok(vec![(0, 1, 0, 10), (1, 2, 10, 10)])
            )
        );
    }

    #[test]
    fn the_samples_are_those_both_the_sample_size_table_and_the_chunks_place_when_read_lossily() {
        let more_in_chunks = durations_counted(track(7, 10, &[1, 2], &[(1, 3)], &[0]), 2);
        let fewer_in_chunks = durations_counted(track(7, 10, &[1, 2, 3], &[(1, 2)], &[0]), 3);
        assert_eq!(
            [
                read_both_ways(&movie([more_in_chunks])),
                read_both_ways(&movie([fewer_in_chunks]))
            ],
            [
                (
                    Err(mismatch(SampleToChunkBox::BOX_TYPE, 2, 3)),
                    Ok(vec![(0, 1, 0, 10), (1, 2, 10, 10)])
                ),
                (
                    Err(mismatch(SampleToChunkBox::BOX_TYPE, 3, 2)),
                    Ok(vec![(0, 1, 0, 10), (1, 2, 10, 10)])
                ),
            ]
        );
    }

    #[test]
    fn samples_past_the_composition_offsets_and_the_dependencies_have_none_when_read_lossily() {
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
        let movie = movie([trak]);
        let mut reader = StructureDataReader::new(1024);
        let refused = reader.handle_movie(&movie).map(|_| ());
        let found: Vec<_> = reader
            .handle_movie_lossy(&movie)
            .unwrap()
            .map(|location| {
                (
                    location.properties.sample_composition_time_offset,
                    location.properties.sample_flags.sample_depends_on,
                )
            })
            .collect();
        assert_eq!(
            (refused, found),
            (
                Err(mismatch(CompositionOffsetBox::BOX_TYPE, 3, 1)),
                vec![(20, 2), (0, 2), (0, 2)]
            )
        );
    }

    #[test]
    fn a_sync_sample_number_past_the_samples_or_not_after_the_one_before_is_not_read_when_read_lossily()
     {
        let mut trak = track(7, 10, &[1, 1, 1], &[(1, 3)], &[0]);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = vec![1, 1, 3, 9];
        trak.mdia.minf.stbl.stss = Some(stss);
        let movie = movie([trak]);
        let mut reader = StructureDataReader::new(1024);
        let refused = reader.handle_movie(&movie).map(|_| ());
        let sync: Vec<_> = reader
            .handle_movie_lossy(&movie)
            .unwrap()
            .map(|location| !location.properties.sample_flags.sample_is_non_sync_sample)
            .collect();
        assert_eq!(
            (refused, sync),
            (
                Err(Error::InvalidEntry {
                    track_id: 7,
                    box_type: SyncSampleBox::BOX_TYPE,
                    entry: 1,
                }),
                vec![true, false, true]
            )
        );
    }

    #[test]
    fn a_sample_table_under_which_no_sample_can_be_placed_is_refused_either_way() {
        let no_durations = durations_counted(track(7, 10, &[1, 2], &[(1, 2)], &[0]), 0);
        let no_chunks = track(7, 10, &[1, 2], &[], &[]);
        let no_durations_counted = mismatch(TimeToSampleBox::BOX_TYPE, 2, 0);
        let no_chunk_counted = mismatch(SampleToChunkBox::BOX_TYPE, 2, 0);
        assert_eq!(
            [
                read_both_ways(&movie([no_durations])),
                read_both_ways(&movie([no_chunks]))
            ],
            [
                (Err(no_durations_counted), Err(no_durations_counted)),
                (Err(no_chunk_counted), Err(no_chunk_counted)),
            ]
        );
    }

    #[test]
    fn a_sample_to_chunk_run_no_sample_placed_uses_is_not_read_when_read_lossily() {
        let mut trak = track(7, 10, &[1], &[(1, 1), (2, 1)], &[100, 200]);
        let stsc = trak.mdia.minf.stbl.stsc.as_mut().unwrap();
        stsc.entries.last_mut().unwrap().sample_description_index = 2;
        assert_eq!(
            read_both_ways(&movie([trak])),
            (
                Err(mismatch(SampleToChunkBox::BOX_TYPE, 1, 2)),
                Ok(vec![(100, 1, 0, 10)])
            )
        );
    }

    #[test]
    fn a_movie_refused_gives_no_track_to_the_movie_fragments_after_it() {
        let mismatched = durations_counted(track(7, 10, &[1, 2], &[(1, 2)], &[0]), 3);
        let mut reader = StructureDataReader::new(1024);
        assert_eq!(
            (
                reader.handle_movie(&movie([mismatched])).map(|_| ()),
                reader
                    .handle_movie_fragment(&isobmff_boxes::MovieFragmentBox::default(), 0)
                    .map(|_| ())
            ),
            (
                Err(mismatch(TimeToSampleBox::BOX_TYPE, 2, 3)),
                Err(Error::MovieFragmentWithoutMovie)
            )
        );
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

    /// File offset of each sample of each track of the `moov` GPAC reports, in the order of their sample numbers, by `track_ID`
    ///
    /// Sample numbers count the samples in decoding order, ISO/IEC 14496-12
    /// §8.6.1.2; the offsets are those its sample-to-chunk, chunk offset and
    /// sample size tables give, §8.7.4, §8.7.5, §8.7.3.
    #[expect(
        clippy::indexing_slicing,
        reason = "indexing a JSON value yields null for a key it lacks, and never panics"
    )]
    fn gpac_offsets_by_sample_number(report: &Value) -> BTreeMap<u32, Vec<u64>> {
        let number = |value: &Value| -> Option<u64> { value.as_str()?.parse().ok() };
        let mut tracks = BTreeMap::new();
        for trak in gpac_list(&report["IsoMediaFile"]["MovieBox"]["TrackBox"]) {
            let stbl = &trak["MediaBox"]["MediaInformationBox"]["SampleTableBox"];
            let sizes_box = if stbl["SampleSizeBox"].is_null() {
                &stbl["CompactSampleSizeBox"]
            } else {
                &stbl["SampleSizeBox"]
            };
            let sizes = gpac_list(&sizes_box["SampleSizeEntry"]);
            let size_of = |sample: usize| -> Option<u64> {
                number(&sizes_box["@ConstantSampleSize"])
                    .or_else(|| number(&sizes.get(sample)?["@Size"]))
            };
            let (Some(sample_count), Some(track_id)) = (
                number(&sizes_box["@SampleCount"]),
                number(&trak["TrackHeaderBox"]["@TrackID"]),
            ) else {
                continue;
            };
            let chunks = gpac_list(&stbl["ChunkOffsetBox"]["ChunkEntry"])
                .into_iter()
                .filter_map(|entry| number(&entry["@offset"]));
            let runs: Vec<(u64, u64)> = gpac_list(&stbl["SampleToChunkBox"]["SampleToChunkEntry"])
                .into_iter()
                .filter_map(|entry| {
                    Some((
                        number(&entry["@FirstChunk"])?,
                        number(&entry["@SamplesPerChunk"])?,
                    ))
                })
                .collect();
            let mut offsets = Vec::new();
            'chunks: for (chunk, chunk_offset) in (1_u64..).zip(chunks) {
                let per_chunk = runs
                    .iter()
                    .rev()
                    .find(|&&(first_chunk, _)| first_chunk <= chunk)
                    .map_or(0, |&(_, per_chunk)| per_chunk);
                let mut offset = chunk_offset;
                for _ in 0..per_chunk {
                    let sample = offsets.len();
                    if u64::try_from(sample).unwrap() == sample_count {
                        break 'chunks;
                    }
                    offsets.push(offset);
                    offset = offset.checked_add(size_of(sample).unwrap()).unwrap();
                }
            }
            if !offsets.is_empty() {
                tracks.insert(u32::try_from(track_id).unwrap(), offsets);
            }
        }
        tracks
    }

    #[test]
    #[ignore = "needs the conformance files"]
    fn the_samples_of_each_track_in_decoding_order_lie_where_gpac_reports_them_by_sample_number() {
        let mut differences = Vec::new();
        for file in conformance_files() {
            let Some(movie) = movie_of(&file.bytes) else {
                continue;
            };
            let Ok(mut found) = locations(&movie) else {
                continue;
            };
            found.sort_by_key(|location| location.properties.decoding_time);
            let mut ours: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
            for location in &found {
                ours.entry(location.properties.track_id)
                    .or_default()
                    .push(location.offset);
            }
            let gpac = gpac_offsets_by_sample_number(&file.report);
            if ours != gpac {
                differences.push(file.name);
            }
        }
        assert_eq!(differences, Vec::<String>::new());
    }

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
