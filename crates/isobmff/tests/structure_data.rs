//! The samples a `moov` declares, resolved to where their bytes lie and written back into sample tables, checked against FFmpeg and GPAC on the conformance files

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::structure_data::{Error, SampleTableWriter, StructureDataReader};
    use isobmff::{SampleLocation, SampleProperties};
    use std::num::NonZeroU32;

    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkOffsetVariant, CompositionOffsetBox, CompositionOffsetEntry,
        MovieBox, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleDescriptionBox,
        SampleFlags, SampleSizeBox, SampleSizeVariant, SampleSizes, SampleTableBox,
        SampleToChunkBox, SampleToChunkEntry, SyncSampleBox, TimeToSampleBox, TimeToSampleEntry,
    };
    use isobmff_core::{BoxDecode, BoxDefinition};
    use isobmff_test_support::fixtures::{encoded, in_another_file, movie, track};
    use isobmff_test_support::serde_json::{self, Value};
    use isobmff_test_support::{conformance_files, ffprobe, gpac_list as list};

    const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

    /// `(offset, size, decoding_time)` of the samples of each track, by `track_ID`
    type Samples = BTreeMap<u32, Vec<(u64, u64, u64)>>;

    /// The `moov` of `file`, read with the framing reader
    fn movie_of(file: &[u8]) -> Option<MovieBox> {
        let mut reader = BoxReader::new(FramerConfig::new(64 * 1024 * 1024));
        let mut movie = None;
        loop {
            while let Some(event) = reader.poll_event() {
                if let Event::Gathered {
                    box_type, payload, ..
                } = event
                    && box_type == MovieBox::BOX_TYPE
                {
                    movie = Some(MovieBox::decode_payload(payload).unwrap());
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
        movie
    }

    fn locations(movie: &MovieBox) -> Result<Vec<SampleLocation>, Error> {
        StructureDataReader::new(SAMPLE_COUNT_LIMIT)
            .handle_movie(movie)
            .map(Iterator::collect)
    }

    /// `(offset, size, decoding_time)` of the samples of each track, in decoding order
    fn by_track(locations: &[SampleLocation]) -> Samples {
        let mut tracks = Samples::new();
        for location in locations {
            tracks
                .entry(location.properties.track_id)
                .or_default()
                .push((
                    location.offset,
                    u64::from(location.size),
                    location.properties.decoding_time,
                ));
        }
        for samples in tracks.values_mut() {
            samples.sort_by_key(|&(_, _, decoding_time)| decoding_time);
        }
        tracks
    }

    /// `(pos, size, dts)` of the packets ffprobe reads from each track, ignoring edit lists, or `None` where ffprobe does not read the file
    #[expect(
        clippy::indexing_slicing,
        reason = "indexing a JSON value yields null for a key it lacks, and never panics"
    )]
    fn ffprobe_by_track(path: &Path) -> Option<Samples> {
        let output = ffprobe(
            &[
                "-v",
                "error",
                "-ignore_editlist",
                "1",
                "-show_entries",
                "stream=index,id:packet=stream_index,pos,size,dts",
                "-of",
                "json",
            ],
            path,
        );
        if !output.status.success() {
            return None;
        }
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let ids = report["streams"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stream| {
                let id = stream["id"].as_str().unwrap().trim_start_matches("0x");
                u32::from_str_radix(id, 16).unwrap()
            })
            .collect::<Vec<_>>();
        let mut tracks = BTreeMap::<u32, Vec<_>>::new();
        for packet in report["packets"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let number = |field: &str| {
                let value = &packet[field];
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.parse().ok())
                    .unwrap()
            };
            let index = usize::try_from(packet["stream_index"].as_u64().unwrap()).unwrap();
            tracks.entry(*ids.get(index).unwrap()).or_default().push((
                u64::try_from(number("pos")).unwrap(),
                u64::try_from(number("size")).unwrap(),
                number("dts"),
            ));
        }
        Some(
            tracks
                .into_iter()
                .map(|(track_id, packets)| {
                    let first = packets.first().map_or(0, |&(_, _, dts)| dts);
                    let counted = packets
                        .into_iter()
                        .map(|(pos, size, dts)| {
                            (
                                pos,
                                size,
                                u64::try_from(dts.checked_sub(first).unwrap()).unwrap(),
                            )
                        })
                        .collect();
                    (track_id, counted)
                })
                .collect(),
        )
    }

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

    #[test]
    fn tables_that_count_different_numbers_of_samples_are_refused() {
        let mut trak = track(7, 1, &[1, 1], &[(1, 2)], &[0]);
        trak.mdia
            .minf
            .stbl
            .stts
            .as_mut()
            .unwrap()
            .entries
            .first_mut()
            .unwrap()
            .sample_count = 3;
        assert_eq!(
            locations(&movie([trak])),
            Err(Error::SampleCountMismatch {
                track_id: 7,
                box_type: TimeToSampleBox::BOX_TYPE,
                sample_count: 2,
                table_sample_count: 3,
            })
        );
        let chunks_short = track(7, 1, &[1, 1, 1], &[(1, 2)], &[0]);
        assert_eq!(
            locations(&movie([chunks_short])),
            Err(Error::SampleCountMismatch {
                track_id: 7,
                box_type: SampleToChunkBox::BOX_TYPE,
                sample_count: 3,
                table_sample_count: 2,
            })
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

    /// A sample of track 1 the first sample entry describes, a sync sample of no declared dependency
    fn location(offset: u64, size: u32, sample_duration: u32) -> SampleLocation {
        SampleLocation {
            properties: SampleProperties {
                track_id: 1,
                decoding_time: 0,
                sample_duration,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            offset,
            size,
        }
    }

    /// The sample entries of a track of `count` sample entries alike, whose media data is in the file itself
    fn sample_entries(count: usize) -> SampleDescriptionBox {
        let mut stsd = track(1, 1, &[], &[], &[]).mdia.minf.stbl.stsd;
        let entry = stsd.sample_descriptions.first().unwrap().clone();
        stsd.sample_descriptions = vec![entry; count];
        stsd
    }

    /// The `stbl` a writer writes from `locations`, with `stsd`
    fn written(
        stsd: &SampleDescriptionBox,
        locations: impl IntoIterator<Item = SampleLocation>,
    ) -> SampleTableBox {
        let mut writer = SampleTableWriter::new(SAMPLE_COUNT_LIMIT);
        for location in locations {
            writer.handle_location(location).unwrap();
        }
        let mut out = Vec::new();
        writer.finish(stsd, &mut out).unwrap();
        let (stbl, rest) = SampleTableBox::decode(&out).unwrap();
        assert!(rest.is_empty());
        stbl
    }

    /// The `stbl` of a track of one sample entry whose samples are `sizes`, each lasting `sample_delta`, laid in chunks at `chunk_offsets` by the runs `(first_chunk, samples_per_chunk)`
    fn table(
        sample_delta: u32,
        sizes: &[u32],
        runs: &[(u32, u32)],
        chunk_offsets: &[u64],
    ) -> SampleTableBox {
        track(1, sample_delta, sizes, runs, chunk_offsets)
            .mdia
            .minf
            .stbl
    }

    /// The `stsz` of `sample_count` samples of `sample_size` bytes each
    fn one_size(sample_size: u32, sample_count: u32) -> Option<SampleSizeVariant> {
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Constant {
            sample_size: NonZeroU32::new(sample_size).unwrap(),
            sample_count,
        };
        Some(SampleSizeVariant::SampleSize(stsz))
    }

    #[test]
    fn a_chunk_is_a_run_of_samples_whose_bytes_follow_one_another() {
        let stbl = written(
            &sample_entries(1),
            [
                location(100, 2, 10),
                location(102, 3, 10),
                location(200, 4, 20),
                location(204, 4, 20),
            ],
        );
        let mut expected = table(0, &[2, 3, 4, 4], &[(1, 2)], &[100, 200]);
        let mut stts = TimeToSampleBox::default();
        stts.entries = [(2, 10), (2, 20)]
            .map(|(sample_count, sample_delta)| {
                let mut entry = TimeToSampleEntry::default();
                entry.sample_count = sample_count;
                entry.sample_delta = sample_delta;
                entry
            })
            .to_vec();
        expected.stts = Some(stts);
        assert_eq!(stbl, expected);
    }

    #[test]
    fn samples_of_one_size_that_are_all_sync_samples_take_one_sample_size_and_no_sync_sample_table()
    {
        let stbl = written(
            &sample_entries(1),
            (0..4).map(|index| location(48 + index * 5, 5, 512)),
        );
        let mut expected = table(512, &[5, 5, 5, 5], &[(1, 4)], &[48]);
        expected.sample_size = one_size(5, 4);
        assert_eq!(stbl, expected);
    }

    #[test]
    fn the_composition_offsets_sync_samples_and_dependencies_are_written_where_a_sample_declares_one()
     {
        let samples = [(20, false, 2), (-10, true, 1), (-10, false, 0)]
            .into_iter()
            .zip(0..)
            .map(|((offset, is_non_sync_sample, sample_depends_on), index)| {
                let mut sample = location(index, 1, 10);
                sample.properties.sample_composition_time_offset = offset;
                sample.properties.sample_flags.sample_is_non_sync_sample = is_non_sync_sample;
                sample.properties.sample_flags.sample_depends_on = sample_depends_on;
                sample
            });
        let stbl = written(&sample_entries(1), samples);

        let mut expected = table(10, &[1, 1, 1], &[(1, 3)], &[0]);
        expected.sample_size = one_size(1, 3);
        let mut ctts = CompositionOffsetBox::default();
        ctts.version = 1;
        ctts.entries = [(1, 20), (2, -10)]
            .map(|(sample_count, sample_offset)| {
                let mut entry = CompositionOffsetEntry::default();
                entry.sample_count = sample_count;
                entry.sample_offset = sample_offset;
                entry
            })
            .to_vec();
        expected.ctts = Some(ctts);
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = vec![1, 3];
        expected.stss = Some(stss);
        let mut sdtp = SampleDependencyTypeBox::default();
        sdtp.entries = [2, 1, 0]
            .map(|sample_depends_on| {
                let mut entry = SampleDependencyTypeEntry::default();
                entry.sample_depends_on = sample_depends_on;
                entry
            })
            .to_vec();
        expected.sdtp = Some(sdtp);
        assert_eq!(stbl, expected);
    }

    #[test]
    fn another_sample_entry_starts_a_chunk_and_a_chunk_past_32_bits_takes_large_offsets() {
        let mut described_by_2 = location(4, 4, 1);
        described_by_2.properties.sample_description_index = 2;
        let stsd = sample_entries(2);
        let stbl = written(
            &stsd,
            [location(0, 4, 1), described_by_2, location(1 << 32, 4, 1)],
        );

        let mut expected = table(1, &[4, 4, 4], &[(1, 1)], &[0, 4, 1 << 32]);
        expected.stsd = stsd;
        expected.sample_size = one_size(4, 3);
        let mut stsc = SampleToChunkBox::default();
        stsc.entries = [(1, 1), (2, 2), (3, 1)]
            .map(|(first_chunk, sample_description_index)| {
                let mut entry = SampleToChunkEntry::default();
                entry.first_chunk = first_chunk;
                entry.samples_per_chunk = 1;
                entry.sample_description_index = sample_description_index;
                entry
            })
            .to_vec();
        expected.stsc = Some(stsc);
        let mut co64 = ChunkLargeOffsetBox::default();
        co64.chunk_offsets = vec![0, 4, 1 << 32];
        expected.chunk_offset = Some(ChunkOffsetVariant::ChunkLargeOffset(co64));
        assert_eq!(stbl, expected);
    }

    #[test]
    fn a_sample_the_writer_refuses_is_not_taken() {
        let mut writer = SampleTableWriter::new(2);
        let mut far_composition = location(2, 1, 1);
        far_composition.properties.sample_composition_time_offset = i64::from(i32::MAX) + 1;
        assert_eq!(
            [
                writer.handle_location(location(0, 1, 1)),
                writer.handle_location(far_composition),
                writer.handle_location(location(1, 1, 1)),
                writer.handle_location(location(2, 1, 1)),
            ],
            [
                Ok(()),
                Err(Error::CompositionOffsetOutOfRange {
                    sample_composition_time_offset: i64::from(i32::MAX) + 1,
                }),
                Ok(()),
                Err(Error::SampleCountOverLimit {
                    sample_count: 3,
                    limit: 2,
                }),
            ]
        );
        let mut out = Vec::new();
        writer.finish(&sample_entries(1), &mut out).unwrap();
        assert_eq!(
            SampleTableBox::decode(&out).unwrap().0,
            written(&sample_entries(1), [location(0, 1, 1), location(1, 1, 1)])
        );
    }

    #[test]
    fn a_table_naming_a_sample_entry_the_stsd_lacks_is_not_written_and_the_samples_stay() {
        let mut writer = SampleTableWriter::new(SAMPLE_COUNT_LIMIT);
        let mut described_by_2 = location(0, 1, 1);
        described_by_2.properties.sample_description_index = 2;
        writer.handle_location(described_by_2).unwrap();
        let mut out = b"ftyp".to_vec();
        assert_eq!(
            writer.finish(&sample_entries(1), &mut out),
            Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index: 2,
            })
        );
        assert_eq!(out, b"ftyp");
        writer.finish(&sample_entries(2), &mut out).unwrap();
        assert_eq!(
            SampleTableBox::decode(out.get(4..).unwrap()).unwrap().0,
            written(&sample_entries(2), [described_by_2])
        );
    }

    #[test]
    fn a_writer_that_has_written_a_table_takes_the_samples_of_another_track() {
        let stsd = sample_entries(1);
        let mut writer = SampleTableWriter::new(SAMPLE_COUNT_LIMIT);
        let mut out = Vec::new();
        writer.handle_location(location(0, 7, 1)).unwrap();
        writer.finish(&stsd, &mut out).unwrap();
        let first_length = out.len();
        writer.handle_location(location(7, 9, 2)).unwrap();
        writer.finish(&stsd, &mut out).unwrap();
        assert_eq!(
            [
                SampleTableBox::decode(&out).unwrap().0,
                SampleTableBox::decode(out.get(first_length..).unwrap())
                    .unwrap()
                    .0,
            ],
            [
                written(&stsd, [location(0, 7, 1)]),
                written(&stsd, [location(7, 9, 2)]),
            ]
        );
    }

    /// What a sample table declares of each sample of a track, in decoding order: `(offset, size, sample_duration, sample_composition_time_offset, is_sync_sample, sample_description_index, [is_leading, sample_depends_on, sample_is_depended_on, sample_has_redundancy])`
    type Declared = Vec<(u64, u64, u64, i64, bool, u64, [u8; 4])>;

    fn declared(locations: &[SampleLocation], track_id: u32) -> Declared {
        let mut track = locations
            .iter()
            .filter(|location| location.properties.track_id == track_id)
            .collect::<Vec<_>>();
        track.sort_by_key(|location| location.properties.decoding_time);
        track
            .into_iter()
            .map(|location| {
                let properties = location.properties;
                let flags = properties.sample_flags;
                (
                    location.offset,
                    u64::from(location.size),
                    u64::from(properties.sample_duration),
                    properties.sample_composition_time_offset,
                    !flags.sample_is_non_sync_sample,
                    u64::from(properties.sample_description_index),
                    [
                        flags.is_leading,
                        flags.sample_depends_on,
                        flags.sample_is_depended_on,
                        flags.sample_has_redundancy,
                    ],
                )
            })
            .collect()
    }

    /// What GPAC reports the sample table of each track of the `moov` to declare of each sample, by `track_ID`
    #[expect(
        clippy::indexing_slicing,
        reason = "indexing a JSON value yields null for a key it lacks, and never panics"
    )]
    fn gpac_declared(report: &Value) -> BTreeMap<u32, Declared> {
        fn number<T: std::str::FromStr>(value: &Value) -> T
        where
            T::Err: std::fmt::Debug,
        {
            value.as_str().unwrap().parse().unwrap()
        }
        fn runs(entries: &Value, value: &str) -> Vec<i64> {
            list(entries)
                .into_iter()
                .flat_map(|entry| {
                    std::iter::repeat_n(
                        number::<i64>(&entry[value]),
                        number(&entry["@SampleCount"]),
                    )
                })
                .collect()
        }
        let dependency = |value: &Value| {
            ["unknown", "yes", "no"]
                .iter()
                .position(|&word| Some(word) == value.as_str())
                .map(|position| u8::try_from(position).unwrap())
                .unwrap()
        };

        let movie = &report["IsoMediaFile"]["MovieBox"];
        list(&movie["TrackBox"])
            .into_iter()
            .map(|trak| {
                let stbl = &trak["MediaBox"]["MediaInformationBox"]["SampleTableBox"];
                let sizes: Vec<u64> = if stbl["SampleSizeBox"].is_null() {
                    list(&stbl["CompactSampleSizeBox"]["SampleSizeEntry"])
                        .into_iter()
                        .map(|entry| number(&entry["@Size"]))
                        .collect()
                } else {
                    let stsz = &stbl["SampleSizeBox"];
                    // Why read a missing `@ConstantSampleSize` as 0: GPAC
                    // reports it only where it is not.
                    let constant = &stsz["@ConstantSampleSize"];
                    match if constant.is_null() {
                        0
                    } else {
                        number::<u64>(constant)
                    } {
                        0 => list(&stsz["SampleSizeEntry"])
                            .into_iter()
                            .map(|entry| number(&entry["@Size"]))
                            .collect(),
                        size => vec![size; number(&stsz["@SampleCount"])],
                    }
                };
                let durations = runs(
                    &stbl["TimeToSampleBox"]["TimeToSampleEntry"],
                    "@SampleDelta",
                );
                let composition_offsets = runs(
                    &stbl["CompositionOffsetBox"]["CompositionOffsetEntry"],
                    "@CompositionOffset",
                );
                let sync_samples = (!stbl["SyncSampleBox"].is_null()).then(|| {
                    list(&stbl["SyncSampleBox"]["SyncSampleEntry"])
                        .into_iter()
                        .map(|entry| number::<usize>(&entry["@sampleNumber"]))
                        .collect::<Vec<_>>()
                });
                let dependencies = list(&stbl["SampleDependencyTypeBox"]["SampleDependencyEntry"])
                    .into_iter()
                    .map(|entry| {
                        [
                            "@isLeading",
                            "@dependsOnOther",
                            "@dependedOn",
                            "@hasRedundancy",
                        ]
                        .map(|field| dependency(&entry[field]))
                    })
                    .collect::<Vec<_>>();
                let chunk_offsets = list(&stbl["ChunkOffsetBox"]["ChunkEntry"])
                    .into_iter()
                    .map(|entry| number::<u64>(&entry["@offset"]))
                    .collect::<Vec<_>>();
                let stsc = list(&stbl["SampleToChunkBox"]["SampleToChunkEntry"]);

                let mut samples = Declared::new();
                for (chunk, &chunk_offset) in (1..).zip(&chunk_offsets) {
                    let Some(entry) = stsc
                        .iter()
                        .rev()
                        .find(|entry| number::<u64>(&entry["@FirstChunk"]) <= chunk)
                    else {
                        continue;
                    };
                    let mut offset = chunk_offset;
                    for _ in 0..number::<u64>(&entry["@SamplesPerChunk"]) {
                        let index = samples.len();
                        let Some(&size) = sizes.get(index) else { break };
                        samples.push((
                            offset,
                            size,
                            u64::try_from(durations[index]).unwrap(),
                            composition_offsets.get(index).copied().unwrap_or(0),
                            sync_samples.as_ref().is_none_or(|numbers| {
                                numbers.contains(&index.checked_add(1).unwrap())
                            }),
                            number(&entry["@SampleDescriptionIndex"]),
                            dependencies.get(index).copied().unwrap_or_default(),
                        ));
                        offset = offset.checked_add(size).unwrap();
                    }
                }
                (number(&trak["TrackHeaderBox"]["@TrackID"]), samples)
            })
            .collect()
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR"]
    fn the_sample_tables_written_from_the_conformance_files_declare_the_samples_gpac_reports() {
        let mut differences = Vec::new();
        let mut tracks_written = 0;
        for file in conformance_files() {
            let Some(movie) = movie_of(&file.bytes) else {
                continue;
            };
            let Ok(found) = locations(&movie) else {
                continue;
            };
            let mut rewritten = movie.clone();
            for trak in &mut rewritten.trak {
                let track_id = trak.tkhd.track_id;
                let mut track = found
                    .iter()
                    .filter(|location| location.properties.track_id == track_id)
                    .collect::<Vec<_>>();
                track.sort_by_key(|location| location.properties.decoding_time);
                let mut writer = SampleTableWriter::new(SAMPLE_COUNT_LIMIT);
                let mut out = Vec::new();
                let result = track
                    .into_iter()
                    .try_for_each(|&location| writer.handle_location(location))
                    .and_then(|()| writer.finish(&trak.mdia.minf.stbl.stsd, &mut out));
                if let Err(error) = result {
                    differences.push(format!("{} track {track_id}: {error}", file.name));
                    continue;
                }
                trak.mdia.minf.stbl = SampleTableBox::decode(&out).unwrap().0;
                tracks_written += 1;
            }
            let read_back = match locations(&rewritten) {
                Ok(read_back) => read_back,
                Err(error) => {
                    differences.push(format!(
                        "{}: the written tables are refused: {error}",
                        file.name
                    ));
                    continue;
                }
            };
            if read_back != found {
                differences.push(format!(
                    "{}: the written tables resolve to other samples",
                    file.name
                ));
            }
            for (track_id, theirs) in gpac_declared(&file.report) {
                let ours = declared(&read_back, track_id);
                if ours != theirs {
                    differences.push(format!(
                        "{} track {track_id}: {} samples, GPAC reports {}; first differing {:?}",
                        file.name,
                        ours.len(),
                        theirs.len(),
                        ours.iter()
                            .zip(&theirs)
                            .find(|(ours, theirs)| ours != theirs)
                    ));
                }
            }
        }
        assert_eq!(differences, Vec::<String>::new());
        assert!(tracks_written > 0);
    }
}
