//! The location of each sample the sample tables of a movie declare, checked against GPAC and FFmpeg on the conformance files and the fixtures

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error as _;
    use std::fs;
    use std::num::NonZeroU32;
    use std::path::PathBuf;

    use isobmff::boxes::{
        ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox,
        CompositionOffsetEntry, DataEntry, DataEntrySeqNumImdaBox, MovieBox,
        SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleDescription, SampleEntry,
        SampleFlags, SampleSizeBox, SampleSizeVariant, SampleSizes, SampleToChunkBox,
        SampleToChunkEntry, SyncSampleBox, TimeToSampleBox, TrackBox,
    };
    use isobmff::structure_data::data_layout::data_entry;
    use isobmff::structure_data::{Error, MovieReader};
    use isobmff::{
        BoxDefinition, BoxType, DataOrigin, FourCc, MediaDataContainer, SampleLocation,
        SampleProperties,
    };
    use isobmff_test_support::fixtures::{movie, track};
    use isobmff_test_support::serde_json::Value;
    use isobmff_test_support::{
        FfprobePacket, conformance_files, ffprobe, ffprobe_packets, fixture_movie_box, gpac_list,
        gpac_tracks, movie_box,
    };

    /// The location of the sample of track `track_id` decoded at `decoding_time` for `sample_duration`, at data offset `offset` of the file for `size` bytes, of the first sample entry
    fn location(
        track_id: u32,
        (decoding_time, sample_duration): (u64, u32),
        (offset, size): (u64, u32),
    ) -> SampleLocation {
        SampleLocation {
            properties: SampleProperties {
                track_id,
                decoding_time,
                sample_duration,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            container: MediaDataContainer::SameFile,
            data_origin: DataOrigin::Container,
            offset,
            size,
        }
    }

    /// Returns why `movie` is refused, for the track it names
    fn refused(movie: &MovieBox) -> Option<Error> {
        MovieReader::new().handle_movie(movie).err()
    }

    /// The refusal of the tables of track `track_id`, for `source`
    fn of_track(track_id: u32, source: Error) -> Option<Error> {
        Some(Error::Track {
            track_id,
            source: Box::new(source),
        })
    }

    #[test]
    fn the_samples_of_each_track_come_in_decoding_order_track_after_track() {
        let movie = movie([
            track(1, 10, &[3, 4, 5], &[(1, 2), (2, 1)], &[100, 200]),
            track(2, 7, &[], &[], &[]),
            track(3, 20, &[6, 7], &[(1, 1)], &[300, 50]),
        ]);
        let mut locations = MovieReader::new().handle_movie(&movie).unwrap();
        assert_eq!(locations.remaining(), 5);
        let first = locations.next();
        assert_eq!(locations.remaining(), 4);
        assert_eq!(
            first.into_iter().chain(locations).collect::<Vec<_>>(),
            [
                location(1, (0, 10), (100, 3)),
                location(1, (10, 10), (103, 4)),
                location(1, (20, 10), (200, 5)),
                location(3, (0, 20), (300, 6)),
                location(3, (20, 20), (50, 7)),
            ]
        );
    }

    /// A track of 4 samples of 1 byte, lasting 1 each, 2 in each of its 2 chunks
    fn track_of_4() -> TrackBox {
        track(1, 1, &[1; 4], &[(1, 2)], &[0, 100])
    }

    #[test]
    fn a_table_that_covers_another_number_of_samples_than_the_sample_size_table_is_refused() {
        let mismatch = |box_type, table_sample_count| {
            of_track(
                1,
                Error::SampleCountMismatch {
                    box_type,
                    table_sample_count,
                    sample_count: 4,
                },
            )
        };
        let mut trak = track_of_4();
        let stbl = &mut trak.mdia.minf.stbl;
        stbl.stts
            .as_mut()
            .unwrap()
            .entries
            .first_mut()
            .unwrap()
            .sample_count = 5;
        assert_eq!(
            refused(&movie([trak])),
            mismatch(TimeToSampleBox::BOX_TYPE, 5)
        );

        let mut trak = track_of_4();
        let mut ctts = CompositionOffsetBox::default();
        let mut entry = CompositionOffsetEntry::default();
        entry.sample_count = 3;
        ctts.entries = vec![entry];
        trak.mdia.minf.stbl.ctts = Some(ctts);
        assert_eq!(
            refused(&movie([trak])),
            mismatch(CompositionOffsetBox::BOX_TYPE, 3)
        );

        let mut trak = track_of_4();
        let mut sdtp = SampleDependencyTypeBox::default();
        sdtp.entries = vec![SampleDependencyTypeEntry::default(); 6];
        trak.mdia.minf.stbl.sdtp = Some(sdtp);
        assert_eq!(
            refused(&movie([trak])),
            mismatch(SampleDependencyTypeBox::BOX_TYPE, 6)
        );

        let trak = track(1, 1, &[1; 4], &[(1, 3)], &[0, 100]);
        assert_eq!(
            refused(&movie([trak])),
            mismatch(SampleToChunkBox::BOX_TYPE, 6)
        );
    }

    #[test]
    fn a_sample_table_without_a_mandatory_box_is_refused() {
        let without = |remove: fn(&mut TrackBox), box_type| {
            let mut trak = track_of_4();
            remove(&mut trak);
            assert_eq!(
                refused(&movie([trak])),
                of_track(1, Error::MandatoryBoxMissing { box_type })
            );
        };
        without(
            |trak| trak.mdia.minf.stbl.stts = None,
            TimeToSampleBox::BOX_TYPE,
        );
        without(
            |trak| trak.mdia.minf.stbl.stsc = None,
            SampleToChunkBox::BOX_TYPE,
        );
        without(
            |trak| trak.mdia.minf.stbl.chunk_offset = None,
            ChunkOffsetBox::BOX_TYPE,
        );
        without(
            |trak| trak.mdia.minf.stbl.sample_size = None,
            SampleSizeBox::BOX_TYPE,
        );
    }

    #[test]
    fn a_sync_sample_table_that_names_no_sample_is_refused() {
        let mut trak = track_of_4();
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = vec![1, 5];
        trak.mdia.minf.stbl.stss = Some(stss);
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::SyncSampleNumberOutOfRange {
                    entry: 1,
                    sample_number: 5,
                    sample_count: 4
                }
            )
        );
    }

    #[test]
    fn a_data_reference_to_the_sequence_number_of_a_movie_fragment_is_refused_for_a_track_of_samples_alone()
     {
        let sequence_number = DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default());
        let mut no_sample = track(1, 20, &[], &[], &[]);
        no_sample.mdia.minf.dinf.dref.data_entries = vec![sequence_number.clone()];
        let mut some = track(2, 20, &[1], &[(1, 1)], &[0]);
        some.mdia.minf.dinf.dref.data_entries = vec![sequence_number];
        assert_eq!(refused(&movie([no_sample])), None);
        assert_eq!(
            refused(&movie([some])),
            of_track(
                2,
                Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 1
                }
            )
        );
    }

    #[test]
    fn a_run_of_no_sample_may_name_a_sample_entry_the_track_does_not_have() {
        let mut trak = track(1, 1, &[1; 2], &[(1, 1), (2, 0), (3, 1)], &[0, 10, 20]);
        stsc_entry(&mut trak, 1).sample_description_index = 9;
        assert_eq!(refused(&movie([trak.clone()])), None);
        stsc_entry(&mut trak, 2).sample_description_index = 9;
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::SampleDescriptionIndexOutOfRange {
                    sample_description_index: 9,
                    sample_entry_count: 1
                }
            )
        );
    }

    /// The entry `index`, from 0, of the sample-to-chunk table of `trak`
    fn stsc_entry(trak: &mut TrackBox, index: usize) -> &mut SampleToChunkEntry {
        trak.mdia
            .minf
            .stbl
            .stsc
            .as_mut()
            .unwrap()
            .entries
            .get_mut(index)
            .unwrap()
    }

    #[test]
    fn a_data_reference_is_reported_before_a_sample_whose_end_is_past_a_u64() {
        let mut trak = track(1, 1, &[10, 1], &[(1, 1), (2, 1)], &[0, 0]);
        let mut co64 = ChunkLargeOffsetBox::default();
        co64.chunk_offsets = vec![u64::MAX - 5, 0];
        trak.mdia.minf.stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkLargeOffset(co64));
        let descriptions = &mut trak.mdia.minf.stbl.stsd.sample_descriptions;
        descriptions.push(SampleDescription::Typed(SampleEntry::new(
            FourCc::new([0; 4]),
            2,
            Vec::new(),
        )));
        trak.mdia
            .minf
            .dinf
            .dref
            .data_entries
            .push(DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default()));
        stsc_entry(&mut trak, 1).sample_description_index = 2;
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 2
                }
            )
        );
    }

    #[test]
    fn a_sample_whose_end_is_past_what_a_u64_counts_is_refused() {
        let mut trak = track(1, 1, &[1, 10, 1], &[(1, 2), (2, 1)], &[0, 0]);
        let mut co64 = ChunkLargeOffsetBox::default();
        co64.chunk_offsets = vec![u64::MAX - 5, 0];
        trak.mdia.minf.stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkLargeOffset(co64));
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::DataOffsetOverflow {
                    sample_number: 2,
                    offset: u64::MAX - 4,
                    size: 10
                }
            )
        );
    }

    #[test]
    fn a_movie_is_refused_whole_for_the_first_track_refused() {
        let mut second = track(2, 1, &[1], &[(1, 1)], &[0]);
        second.mdia.minf.stbl.stts = None;
        let mut third = track(3, 1, &[1], &[(1, 1)], &[0]);
        third.mdia.minf.stbl.stsc = None;
        let error = refused(&movie([track_of_4(), second, third])).unwrap();
        assert_eq!(
            (error.to_string(), error.source().map(ToString::to_string)),
            (
                String::from("the structure-data of track 2 is refused"),
                Some(String::from(
                    "the sample table holds no `stts` this crate reads, which is mandatory"
                ))
            )
        );
    }

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

    /// A track of `track_id` declaring the most samples a sample size table counts, each of 1 byte and lasting 1, in one chunk, with one entry in each table
    fn track_of_the_most_samples(track_id: u32) -> TrackBox {
        let mut trak = track(track_id, 1, &[], &[(1, u32::MAX)], &[0]);
        let stbl = &mut trak.mdia.minf.stbl;
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Constant {
            sample_size: NonZeroU32::MIN,
            sample_count: u32::MAX,
        };
        stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
        stbl.stts
            .as_mut()
            .unwrap()
            .entries
            .first_mut()
            .unwrap()
            .sample_count = u32::MAX;
        trak
    }

    #[test]
    fn the_tables_are_checked_without_a_step_for_each_sample_they_declare() {
        let movie = movie((1..=4).map(track_of_the_most_samples));
        let locations = MovieReader::new().handle_movie(&movie).unwrap();
        assert_eq!(locations.remaining(), 4 * u64::from(u32::MAX));
    }

    /// The fixtures of the `isobmff` crate, `tests/fixtures`, each with its name
    fn fixtures() -> Vec<(String, PathBuf)> {
        let mut fixtures = fs::read_dir(FIXTURES)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "mp4"))
            .map(|path| (path.file_name().unwrap().to_str().unwrap().to_owned(), path))
            .collect::<Vec<_>>();
        fixtures.sort();
        fixtures
    }

    /// The fixtures whose tables disagree, and the refusal of the first track of each
    const REFUSED_FIXTURES: [(&str, [u8; 4], u64); 2] = [
        ("mismatch_tables.mp4", *b"stts", 10),
        ("mismatch_chunks.mp4", *b"stsc", 24),
    ];

    #[test]
    fn the_fixtures_whose_tables_disagree_are_refused() {
        for (name, box_type, table_sample_count) in REFUSED_FIXTURES {
            let movie = fixture_movie_box(name);
            assert_eq!(
                refused(&movie),
                of_track(
                    1,
                    Error::SampleCountMismatch {
                        box_type: BoxType::compact(box_type),
                        table_sample_count,
                        sample_count: 12
                    }
                ),
                "{name}"
            );
        }
    }

    /// The number of samples GPAC reports the sample size table of each track of `report` declares, by `track_ID`
    fn gpac_sample_counts(report: &Value) -> BTreeMap<u32, usize> {
        gpac_tracks(report)
            .into_iter()
            .map(|(track_id, trak)| {
                let stbl = trak.pointer("/MediaBox/MediaInformationBox/SampleTableBox");
                let sample_count = ["SampleSizeBox", "CompactSampleSizeBox"]
                    .into_iter()
                    .find_map(|size_box| stbl?.get(size_box)?.get("@SampleCount")?.as_str());
                (
                    track_id,
                    sample_count.map_or(0, |count| count.parse().unwrap()),
                )
            })
            .collect()
    }

    /// The container each data reference entry GPAC reports of each track of `report` specifies, in the order of the entries, by `track_ID`; `None` for an entry of a type other than `url ` and `urn `
    fn gpac_containers(report: &Value) -> BTreeMap<u32, Vec<Option<MediaDataContainer>>> {
        gpac_tracks(report)
            .into_iter()
            .map(|(track_id, trak)| {
                let containers = trak
                    .pointer(
                        "/MediaBox/MediaInformationBox/DataInformationBox/DataReferenceBox/URLDataEntryBox",
                    )
                    .map(gpac_list)
                    .unwrap_or_default()
                    .into_iter()
                    .zip(1..)
                    .map(|(entry, data_reference_index)| {
                        match (
                            entry.get("@Type").and_then(Value::as_str),
                            entry.get("@Flags").and_then(Value::as_str),
                        ) {
                            (Some("url "), Some("1")) => Some(MediaDataContainer::SameFile),
                            (Some("url " | "urn "), _) => Some(MediaDataContainer::Resource {
                                data_reference_index,
                            }),
                            _ => None,
                        }
                    })
                    .collect();
                (track_id, containers)
            })
            .collect()
    }

    /// The tracks and samples compared, and the tracks of them in the conformance files
    #[derive(Debug, Default)]
    struct Tally {
        tracks: usize,
        samples: usize,
        conformance_tracks: usize,
    }

    impl Tally {
        fn add(&mut self, is_conformance: bool, samples: usize) {
            self.tracks = self.tracks.saturating_add(1);
            self.samples = self.samples.saturating_add(samples);
            self.conformance_tracks = self
                .conformance_tracks
                .saturating_add(usize::from(is_conformance));
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe"]
    fn the_samples_of_each_track_lie_and_are_timed_where_gpac_and_ffprobe_read_them() {
        let mut differences = Vec::<String>::new();
        let (mut timed, mut located) = (Tally::default(), Tally::default());
        let mut contained = Tally::default();
        let mut times_only = Vec::<(String, u32, usize, usize)>::new();
        let mut left_out = Vec::<(String, Option<u32>, &str)>::new();
        let conformance = conformance_files()
            .into_iter()
            .map(|file| (file.name, file.path, file.bytes, Some(file.report)));
        let fixtures = fixtures().into_iter().map(|(name, path)| {
            let bytes = fs::read(&path).unwrap();
            (name, path, bytes, None)
        });
        for (name, path, bytes, report) in conformance.chain(fixtures) {
            // Why not compare the fixtures whose tables disagree: the
            // specification gives no value to some of their samples, and
            // `the_fixtures_whose_tables_disagree_are_refused` holds them
            // refused.
            if REFUSED_FIXTURES
                .iter()
                .any(|&(refused, ..)| refused == name)
            {
                left_out.push((name, None, "a file whose tables disagree"));
                continue;
            }
            let Some(moov) = movie_box(&bytes, &name) else {
                continue;
            };
            let locations = match MovieReader::new().handle_movie(&moov) {
                Ok(locations) => locations,
                Err(error) => {
                    differences.push(format!("{name}: {error}: {:?}", error.source()));
                    continue;
                }
            };
            let declared = locations.remaining();
            let mut ours = BTreeMap::<u32, Vec<SampleLocation>>::new();
            for location in locations {
                ours.entry(location.properties.track_id)
                    .or_default()
                    .push(location);
            }
            let placed = ours.values().map(Vec::len).sum::<usize>();
            if u64::try_from(placed).unwrap() != declared {
                differences.push(format!(
                    "{name}: {placed} samples returned of the {declared} counted"
                ));
            }
            let gpac = report.as_ref().map(gpac_sample_counts);
            let gpac_containers = report.as_ref().map(gpac_containers);
            let packets = ffprobe(&["-v", "error"], &path).status.success().then(|| {
                let mut packets = BTreeMap::<u32, Vec<FfprobePacket>>::new();
                for packet in ffprobe_packets(&["-ignore_editlist", "1"], &path) {
                    packets.entry(packet.track_id).or_default().push(packet);
                }
                packets
            });
            for trak in &moov.trak {
                let track_id = trak.tkhd.track_id;
                let ours = ours.get(&track_id).map(Vec::as_slice).unwrap_or_default();
                let (stbl, dref) = (&trak.mdia.minf.stbl, &trak.mdia.minf.dinf.dref);
                let declared = match &stbl.sample_size {
                    Some(SampleSizeVariant::SampleSize(stsz)) => match &stsz.sample_sizes {
                        SampleSizes::Constant { sample_count, .. } => {
                            usize::try_from(*sample_count).unwrap()
                        }
                        SampleSizes::Table { entry_sizes } => entry_sizes.len(),
                        _ => 0,
                    },
                    Some(SampleSizeVariant::CompactSampleSize(stz2)) => stz2.entry_sizes.len(),
                    Some(_) | None => 0,
                };
                if ours.len() != declared {
                    differences.push(format!(
                        "{name} track {track_id}: {} samples, the sample size table {declared}",
                        ours.len()
                    ));
                }
                if let Some(containers) = gpac_containers
                    .as_ref()
                    .and_then(|containers| containers.get(&track_id))
                    && !ours.is_empty()
                {
                    let by_sample_entry = stbl
                        .stsd
                        .sample_descriptions
                        .iter()
                        .map(|sample_entry| {
                            let index = usize::from(sample_entry.data_reference_index()?);
                            *containers.get(index.checked_sub(1)?)?
                        })
                        .collect::<Vec<_>>();
                    let theirs = ours
                        .iter()
                        .map(|location| {
                            let index =
                                usize::try_from(location.properties.sample_description_index);
                            *by_sample_entry.get(index.ok()?.checked_sub(1)?)?
                        })
                        .collect::<Vec<_>>();
                    let ours = ours
                        .iter()
                        .map(|location| Some(location.container))
                        .collect::<Vec<_>>();
                    if ours != theirs {
                        differences.push(format!(
                            "{name} track {track_id}: containers {ours:?}, GPAC's data reference entries {theirs:?}"
                        ));
                    }
                    contained.add(report.is_some(), ours.len());
                }
                if let Some(gpac) = &gpac
                    && gpac.get(&track_id) != Some(&ours.len())
                {
                    differences.push(format!(
                        "{name} track {track_id}: {} samples, GPAC {:?}",
                        ours.len(),
                        gpac.get(&track_id)
                    ));
                }
                if declared == 0 {
                    left_out.push((name.clone(), Some(track_id), "of no sample"));
                    continue;
                }
                let entries = stbl.stsc.iter().flat_map(|stsc| &stsc.entries);
                let is_elsewhere = entries.into_iter().any(|entry| {
                    data_entry(&stbl.stsd, dref, entry.sample_description_index)
                        .is_ok_and(|data_entry| !data_entry.is_in_same_file())
                });
                let Some(packets) = &packets else {
                    left_out.push((
                        name.clone(),
                        Some(track_id),
                        "of a file ffprobe does not read",
                    ));
                    continue;
                };
                let theirs = packets
                    .get(&track_id)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                // Why let ffprobe read fewer packets of a track with samples
                // outside this file: it drops those whose bytes it cannot
                // read. Why let it read more in a fragmented file alone: those
                // are the samples of its movie fragments.
                if is_elsewhere && theirs.len() < ours.len() {
                    times_only.push((name.clone(), track_id, ours.len(), theirs.len()));
                } else if theirs.len() < ours.len()
                    || (moov.mvex.is_none() && theirs.len() != ours.len())
                {
                    differences.push(format!(
                        "{name} track {track_id}: {} samples, ffprobe {} packets",
                        ours.len(),
                        theirs.len()
                    ));
                }
                let shift = ours.first().zip(theirs.first()).and_then(|(ours, theirs)| {
                    Some(i128::from(ours.properties.decoding_time) - i128::from(theirs.dts?))
                });
                let differing = ours.iter().zip(theirs).position(|(ours, theirs)| {
                    let decoding_time = i128::from(ours.properties.decoding_time);
                    let composition_time =
                        decoding_time + i128::from(ours.properties.sample_composition_time_offset);
                    // Why not hold the positions of samples outside this
                    // file: ffprobe reads them as if they were in this one.
                    (!is_elsewhere
                        && (ours.offset, u64::from(ours.size)) != (theirs.pos, theirs.size))
                        || theirs.dts.map(|dts| decoding_time - i128::from(dts)) != shift
                        || theirs.pts.map(i128::from) != Some(composition_time)
                });
                if let Some(sample) = differing {
                    differences.push(format!(
                        "{name} track {track_id}: sample {sample} {:?}, ffprobe {:?}",
                        ours.get(sample),
                        theirs.get(sample)
                    ));
                }
                // Why hold the time base against the timescale: a time
                // counted in another unit could agree up to the shift alone.
                let timescale = u64::from(trak.mdia.mdhd.timescale);
                if let Some(&FfprobePacket {
                    time_base: (numerator, denominator),
                    ..
                }) = theirs.first()
                    && timescale.saturating_mul(numerator) != denominator
                {
                    differences.push(format!(
                        "{name} track {track_id}: timescale {timescale}, ffprobe time base {numerator}/{denominator}"
                    ));
                }
                let compared = ours.len().min(theirs.len());
                timed.add(report.is_some(), compared);
                if !is_elsewhere {
                    located.add(report.is_some(), compared);
                }
            }
        }
        let mut tally = BTreeMap::<&str, usize>::new();
        for (name, track_id, bucket) in &left_out {
            println!("left out {name} track {track_id:?}: {bucket}");
            *tally.entry(bucket).or_default() += 1;
        }
        for (name, track_id, samples, packets) in &times_only {
            println!(
                "times alone of {name} track {track_id}: {samples} samples, ffprobe {packets} packets"
            );
        }
        println!(
            "compared the times of {timed:?}, the positions of {located:?}, the containers of {contained:?}; left out {tally:?}"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(
            located.tracks > 0 && located.samples > 0 && contained.samples > 0,
            "no track was compared"
        );
    }
}
