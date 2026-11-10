//! Sample tables written from where the samples of a track lie, checked against GPAC and this crate's reader on the conformance files

#[cfg(test)]
#[path = "support/movie.rs"]
#[expect(
    dead_code,
    reason = "the comparison with ffprobe is for the tests of reading"
)]
mod support;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU32;

    use isobmff::structure_data::{Error, SampleTableWriter};
    use isobmff::{SampleLocation, SampleProperties};
    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkOffsetVariant, CompositionOffsetBox, CompositionOffsetEntry,
        DataReferenceBox, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleDescriptionBox,
        SampleFlags, SampleSizeBox, SampleSizeVariant, SampleSizes, SampleTableBox,
        SampleToChunkBox, SampleToChunkEntry, SyncSampleBox, TimeToSampleBox, TimeToSampleEntry,
    };
    use isobmff_test_support::fixtures::{in_another_file, movie, track};
    use isobmff_test_support::serde_json::Value;
    use isobmff_test_support::{conformance_files, gpac_list as list};

    use crate::support::{SAMPLE_COUNT_LIMIT, locations, movie_of};

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

    /// The data references of a track whose media data is in the file itself
    fn in_the_file() -> DataReferenceBox {
        track(1, 1, &[], &[], &[]).mdia.minf.dinf.dref
    }

    /// The `stbl` a writer writes from `locations`, each at the decoding time the `sample_duration`s before it add up to, with `stsd`
    fn written(
        stsd: &SampleDescriptionBox,
        locations: impl IntoIterator<Item = SampleLocation>,
    ) -> SampleTableBox {
        let mut writer = SampleTableWriter::new(stsd, &in_the_file(), SAMPLE_COUNT_LIMIT);
        let mut decoding_time = 0;
        for mut location in locations {
            location.properties.decoding_time = decoding_time;
            decoding_time = decoding_time
                .checked_add(u64::from(location.properties.sample_duration))
                .unwrap();
            writer.handle_location(location).unwrap();
        }
        writer.write_sample_table()
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
        let at = |offset, decoding_time| {
            let mut sample = location(offset, 1, 1);
            sample.properties.decoding_time = decoding_time;
            sample
        };
        let mut far_composition = at(1, 1);
        far_composition.properties.sample_composition_time_offset = i64::from(i32::MAX) + 1;
        let mut of_track_2 = at(1, 1);
        of_track_2.properties.track_id = 2;
        let mut described_by_2 = at(1, 1);
        described_by_2.properties.sample_description_index = 2;
        let mut elsewhere = SampleTableWriter::new(
            &sample_entries(1),
            &in_another_file(track(1, 1, &[], &[], &[]))
                .mdia
                .minf
                .dinf
                .dref,
            2,
        );
        assert_eq!(
            (elsewhere.handle_location(at(0, 0)), elsewhere.has_samples()),
            (
                Err(Error::MediaDataElsewhere {
                    track_id: 1,
                    data_reference_index: 1,
                }),
                false
            )
        );
        let mut writer = SampleTableWriter::new(&sample_entries(1), &in_the_file(), 2);
        assert_eq!(
            [
                writer.handle_location(at(0, 0)),
                writer.handle_location(described_by_2),
                writer.handle_location(far_composition),
                writer.handle_location(of_track_2),
                writer.handle_location(at(1, 2)),
                writer.handle_location(at(1, 1)),
                writer.handle_location(at(2, 2)),
            ],
            [
                Ok(()),
                Err(Error::SampleDescriptionIndexOutOfRange {
                    sample_description_index: 2,
                }),
                Err(Error::CompositionOffsetOutOfRange {
                    sample_composition_time_offset: i64::from(i32::MAX) + 1,
                }),
                Err(Error::OtherTrack {
                    track_id: 1,
                    found: 2,
                }),
                Err(Error::DecodingTimeGap {
                    track_id: 1,
                    expected: 1,
                    found: 2,
                }),
                Ok(()),
                Err(Error::SampleCountOverLimit {
                    sample_count: 3,
                    limit: 2,
                }),
            ]
        );
        assert_eq!(
            (
                writer.has_samples(),
                writer.write_sample_table(),
                writer.has_samples()
            ),
            (
                true,
                written(&sample_entries(1), [location(0, 1, 1), location(1, 1, 1)]),
                false
            )
        );
    }

    #[test]
    fn a_sample_ending_past_the_last_file_offset_is_refused_and_one_ending_at_it_is_read_back() {
        let mut writer =
            SampleTableWriter::new(&sample_entries(1), &in_the_file(), SAMPLE_COUNT_LIMIT);
        assert_eq!(
            writer.handle_location(location(u64::MAX, 1, 1)),
            Err(Error::Overflow { track_id: 1 })
        );
        let last = location(u64::MAX - 1, 1, 1);
        writer.handle_location(last).unwrap();
        let mut trak = track(1, 1, &[], &[], &[]);
        trak.mdia.minf.stbl = writer.write_sample_table();
        assert_eq!(locations(&movie([trak])), Ok(vec![last]));
    }

    #[test]
    fn the_first_sample_of_a_table_is_at_decoding_time_0() {
        let mut later = location(0, 1, 1);
        later.properties.decoding_time = 1;
        assert_eq!(
            SampleTableWriter::new(&sample_entries(1), &in_the_file(), SAMPLE_COUNT_LIMIT)
                .handle_location(later),
            Err(Error::DecodingTimeGap {
                track_id: 1,
                expected: 0,
                found: 1,
            })
        );
    }

    #[test]
    fn a_writer_that_has_written_a_table_takes_the_samples_of_another_track() {
        let stsd = sample_entries(1);
        let mut of_track_2 = location(7, 9, 2);
        of_track_2.properties.track_id = 2;
        let mut writer = SampleTableWriter::new(&stsd, &in_the_file(), SAMPLE_COUNT_LIMIT);
        writer.handle_location(location(0, 7, 1)).unwrap();
        let first = writer.write_sample_table();
        writer.handle_location(of_track_2).unwrap();
        assert_eq!(
            [first, writer.write_sample_table()],
            [
                written(&stsd, [location(0, 7, 1)]),
                written(&stsd, [of_track_2]),
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
                let mut writer = SampleTableWriter::new(
                    &trak.mdia.minf.stbl.stsd,
                    &trak.mdia.minf.dinf.dref,
                    SAMPLE_COUNT_LIMIT,
                );
                let result = track
                    .into_iter()
                    .try_for_each(|&location| writer.handle_location(location))
                    .map(|()| writer.write_sample_table());
                match result {
                    Ok(stbl) => trak.mdia.minf.stbl = stbl,
                    Err(error) => {
                        differences.push(format!("{} track {track_id}: {error}", file.name));
                        continue;
                    }
                }
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
