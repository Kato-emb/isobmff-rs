//! Movie fragment random access tables read against GPAC's report of the conformance files, and written by the fragmented muxer, each entry checked against the sync sample it points at

#[cfg(test)]
#[path = "support/muxer.rs"]
mod support;

#[cfg(test)]
mod tests {
    use isobmff::demux::Event;
    use isobmff::mux::{MovieParameters, TrackParameters};
    use isobmff::{Demuxer, DemuxerConfig, FragmentedMovieFileMuxer, MuxerConfig};
    use isobmff_boxes::{MovieFragmentBox, MovieFragmentRandomAccessBox, TrackRunSamples};
    use isobmff_core::{BoxDecode, BoxHeader, BoxType};
    use isobmff_test_support::serde_json::Value;
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets, gpac_list};

    use crate::support::{
        Read, has_tracks_in_other_files, mp4_file_type, one_sample_entry_track, packet_read, read,
        sample,
    };

    /// An entry of a `tfra`: its `track_ID`, `time`, `moof_offset`, `traf_number`, `trun_number` and `sample_delta`
    type Entry = (u32, u64, u64, u32, u32, u32);

    /// The `mfra` the demuxer reports of `file`
    fn random_access(file: &[u8]) -> Option<MovieFragmentRandomAccessBox> {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let mut mfra = None;
        loop {
            while let Some(event) = demuxer.poll_event() {
                if let Event::MovieFragmentRandomAccess(found) = event {
                    mfra = Some(found);
                }
            }
            let Some(required) = demuxer.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
                _ => demuxer.finish(),
            }
        }
        mfra
    }

    fn entries(mfra: &MovieFragmentRandomAccessBox) -> Vec<Entry> {
        mfra.tfra
            .iter()
            .flat_map(|tfra| {
                tfra.entries.iter().map(|entry| {
                    (
                        tfra.track_id,
                        entry.time,
                        entry.moof_offset,
                        entry.traf_number,
                        entry.trun_number,
                        entry.sample_delta,
                    )
                })
            })
            .collect()
    }

    /// The file offset of each `moof` at file level of `file`
    fn fragment_offsets(file: &[u8]) -> Vec<u64> {
        let mut offsets = Vec::new();
        let mut position: usize = 0;
        while let Some((header, _)) = file
            .get(position..)
            .and_then(|rest| BoxHeader::decode(rest).ok())
        {
            if header.box_type() == BoxType::compact(*b"moof") {
                offsets.push(u64::try_from(position).unwrap());
            }
            let length = usize::try_from(header.size().total_bytes().unwrap()).unwrap();
            position = position.checked_add(length).unwrap();
        }
        offsets
    }

    /// What each entry of the `mfra` of `file` does not point at: a sync sample of its track, in the `moof` at its offset, through its `traf`, `trun` and sample numbers, composed at its time
    fn differences(file: &[u8], read: &Read) -> Vec<String> {
        let mfra = random_access(file).unwrap();
        let fragments = fragment_offsets(file);
        let starts: Vec<usize> = read.fragment_starts.iter().copied().collect();
        let mut differences = Vec::new();
        for entry @ (track_id, time, moof_offset, traf_number, trun_number, sample_delta) in
            entries(&mfra)
        {
            let Some(fragment) = fragments.iter().position(|&offset| offset == moof_offset) else {
                differences.push(format!("{entry:?}: no moof at its offset"));
                continue;
            };
            let at = usize::try_from(moof_offset).unwrap();
            let (moof, _) = MovieFragmentBox::decode(file.get(at..).unwrap()).unwrap();
            let traf_index = usize::try_from(traf_number)
                .unwrap()
                .checked_sub(1)
                .unwrap();
            let trun_index = usize::try_from(trun_number)
                .unwrap()
                .checked_sub(1)
                .unwrap();
            if moof.traf.get(traf_index).map(|traf| traf.tfhd.track_id) != Some(track_id) {
                differences.push(format!("{entry:?}: its traf is of another track"));
                continue;
            }
            let count = |samples: &TrackRunSamples| match samples {
                TrackRunSamples::EmptyRows { sample_count } => {
                    usize::try_from(*sample_count).unwrap()
                }
                TrackRunSamples::Rows { samples } => samples.len(),
                _ => 0,
            };
            let before_traf: usize = moof
                .traf
                .iter()
                .take(traf_index)
                .filter(|traf| traf.tfhd.track_id == track_id)
                .flat_map(|traf| traf.trun.iter().map(|trun| count(&trun.samples)))
                .sum();
            let before_trun: usize = moof
                .traf
                .get(traf_index)
                .unwrap()
                .trun
                .iter()
                .take(trun_index)
                .map(|trun| count(&trun.samples))
                .sum();
            let index = before_traf
                .checked_add(before_trun)
                .unwrap()
                .checked_add(usize::try_from(sample_delta).unwrap())
                .unwrap()
                .checked_sub(1)
                .unwrap();
            let start = *starts.get(fragment).unwrap();
            let end = starts
                .get(fragment.checked_add(1).unwrap())
                .copied()
                .unwrap_or(read.samples.len());
            let pointed = read
                .samples
                .get(start..end)
                .unwrap()
                .iter()
                .map(|(properties, _)| properties)
                .filter(|properties| properties.track_id == track_id)
                .nth(index);
            let found = pointed.map(|properties| {
                (
                    !properties.sample_flags.sample_is_non_sync_sample,
                    i128::from(properties.decoding_time)
                        .checked_add(i128::from(properties.sample_composition_time_offset))
                        .unwrap(),
                )
            });
            if found != Some((true, i128::from(time))) {
                differences.push(format!("{entry:?}: it points at {found:?}"));
            }
        }
        differences
    }

    /// Writes the samples of `source` in a fragmented movie file, a fragment where each of the source starts
    fn remux(source: &Read) -> Vec<u8> {
        let mut muxer = FragmentedMovieFileMuxer::new(
            MovieParameters::from(&source.movie),
            MuxerConfig::default(),
            source.file_type.clone(),
        )
        .unwrap();
        for trak in &source.movie.trak {
            muxer.add_track(TrackParameters::from(trak)).unwrap();
        }
        let mut written = Vec::new();
        for (index, (properties, bytes)) in source.samples.iter().enumerate() {
            if index > 0 && source.fragment_starts.contains(&index) {
                muxer.finish_fragment().unwrap();
            }
            muxer.handle_sample(*properties, bytes).unwrap();
            muxer.poll_output(&mut written);
        }
        muxer.finish().unwrap();
        muxer.poll_output(&mut written);
        written
    }

    /// Whether the last 32 bits of `file` are the length of the `mfra` that ends it, §8.8.11
    fn ends_with_its_random_access_table(file: &[u8]) -> bool {
        let tail = file.len().checked_sub(4).unwrap();
        let length = u32::from_be_bytes(file.get(tail..).unwrap().try_into().unwrap());
        let start = file
            .len()
            .checked_sub(usize::try_from(length).unwrap())
            .unwrap();
        file.get(start.checked_add(4).unwrap()..start.checked_add(8).unwrap())
            == Some(b"mfra".as_slice())
    }

    #[test]
    fn a_fragmented_movie_written_again_lists_the_entries_ffmpeg_listed_at_its_own_fragments() {
        let file: &[u8] = include_bytes!("fixtures/fragmented.mp4");
        let source = read(file).unwrap();
        let written = remux(&source);

        let theirs = entries(&random_access(file).unwrap());
        let ours = entries(&random_access(&written).unwrap());
        // Why compare all but the `traf_number`: FFmpeg writes 1 for the
        // `traf` of every track, where §8.8.10 numbers the `traf` within its
        // `moof`, as GPAC does; `differences` checks this crate's number.
        let at_fragment =
            |entries: &[Entry], offsets: &[u64]| -> Vec<(u32, u64, usize, u32, u32)> {
                entries
                    .iter()
                    .map(|&(track_id, time, moof_offset, _, trun, sample)| {
                        let fragment = offsets
                            .iter()
                            .position(|&offset| offset == moof_offset)
                            .unwrap();
                        (track_id, time, fragment, trun, sample)
                    })
                    .collect()
            };

        assert_eq!(
            at_fragment(&ours, &fragment_offsets(&written)),
            at_fragment(&theirs, &fragment_offsets(file))
        );
        assert_eq!(
            differences(&written, &read(&written).unwrap()),
            Vec::<String>::new()
        );
        assert!(ends_with_its_random_access_table(&written));
    }

    #[test]
    fn a_sync_sample_after_the_first_run_of_its_track_is_found_through_its_trun() {
        let mut muxer = FragmentedMovieFileMuxer::new(
            MovieParameters::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
        )
        .unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();
        muxer.add_track(one_sample_entry_track(2)).unwrap();
        let of_track = |track_id, decoding_time, sync: bool| {
            let mut properties = sample(decoding_time);
            properties.track_id = track_id;
            properties.sample_flags.sample_is_non_sync_sample = !sync;
            properties
        };
        let mut written = Vec::new();
        for (track_id, decoding_time, sync) in
            [(1, 0, false), (2, 0, true), (1, 10, true), (2, 10, false)]
        {
            muxer
                .handle_sample(of_track(track_id, decoding_time, sync), b"ab")
                .unwrap();
        }
        muxer.finish().unwrap();
        muxer.poll_output(&mut written);

        let moof_offset = *fragment_offsets(&written).first().unwrap();
        assert_eq!(
            entries(&random_access(&written).unwrap()),
            [(1, 10, moof_offset, 1, 2, 1), (2, 0, moof_offset, 2, 1, 1)]
        );
        assert_eq!(
            differences(&written, &read(&written).unwrap()),
            Vec::<String>::new()
        );
    }

    /// The entries of each `tfra` GPAC reports in `report`
    fn gpac_entries(report: &Value) -> Vec<Entry> {
        let mut found = Vec::new();
        let mut nodes = vec![report];
        while let Some(node) = nodes.pop() {
            match node {
                Value::Object(fields)
                    if fields.get("@Type").and_then(Value::as_str) == Some("tfra") =>
                {
                    let number = |entry: &Value, field: &str| -> u64 {
                        entry
                            .get(field)
                            .and_then(Value::as_str)
                            .unwrap()
                            .parse()
                            .unwrap()
                    };
                    let track_id = u32::try_from(number(node, "@TrackId")).unwrap();
                    let small =
                        |entry: &Value, field: &str| u32::try_from(number(entry, field)).unwrap();
                    for entry in fields
                        .get("RandomAccessEntry")
                        .map(gpac_list)
                        .unwrap_or_default()
                    {
                        found.push((
                            track_id,
                            number(entry, "@time"),
                            number(entry, "@moof_offset"),
                            small(entry, "@traf"),
                            small(entry, "@trun"),
                            small(entry, "@sample"),
                        ));
                    }
                }
                Value::Object(fields) => nodes.extend(fields.values().rev()),
                Value::Array(values) => nodes.extend(values.iter().rev()),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        found
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR"]
    fn the_random_access_tables_of_the_conformance_files_read_as_gpac_reports_them() {
        let mut compared = Vec::new();
        for file in conformance_files() {
            let Some(mfra) = random_access(&file.bytes) else {
                continue;
            };
            let ours = entries(&mfra);
            let offsets = fragment_offsets(&file.bytes);
            assert_eq!(ours, gpac_entries(&file.report), "{}", file.name);
            assert!(
                ours.iter().all(|entry| offsets.contains(&entry.2)),
                "{}: an entry points at no moof",
                file.name
            );
            compared.push((file.name.clone(), ours.len()));
        }
        println!("{compared:?}");
        assert_eq!(compared.len(), 3);
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_written_in_fragments_list_each_fragment_s_first_sync_samples() {
        let directory =
            std::env::temp_dir().join(format!("isobmff-random-access-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences_found, mut written_files, mut entry_total, mut compared) =
            (Vec::new(), 0, 0, 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            // Why only fragmented sources without errors or gaps: their
            // samples are written in movie fragments as they are, a file the
            // demuxer reports an error in has none to write, the muxers write
            // the samples of one file alone, and the fragmented muxer refuses
            // a gap in decoding times.
            let mut next = std::collections::BTreeMap::new();
            let gap = source.samples.iter().any(|(properties, _)| {
                let end = properties.decoding_time + u64::from(properties.sample_duration);
                next.insert(properties.track_id, end)
                    .is_some_and(|expected| expected != properties.decoding_time)
            });
            if source.movie.mvex.is_none()
                || !source.errors.is_empty()
                || source.samples.is_empty()
                || has_tracks_in_other_files(&source.movie)
                || gap
            {
                continue;
            }
            let written = remux(&source);
            written_files += 1;
            entry_total += entries(&random_access(&written).unwrap()).len();
            let read_back = read(&written).unwrap();
            differences_found.extend(
                differences(&written, &read_back)
                    .into_iter()
                    .map(|difference| format!("{}: {difference}", file.name)),
            );
            if !ends_with_its_random_access_table(&written) {
                differences_found.push(format!("{}: no mfra at the end", file.name));
            }
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let path = directory.join(&file.name);
            std::fs::write(&path, &written).unwrap();
            let packets = |path: &std::path::Path| {
                ffprobe_packets(&[], path)
                    .iter()
                    .map(packet_read)
                    .collect::<Vec<_>>()
            };
            compared += 1;
            if packets(&path) != packets(&file.path) {
                differences_found.push(format!("{}: ffprobe reads other packets", file.name));
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{written_files} files written, {entry_total} entries, {compared} compared with ffprobe"
        );
        assert_eq!(differences_found, Vec::<String>::new());
        assert!(written_files > 0);
    }

    #[test]
    fn an_mfra_over_the_entry_limit_is_reported_and_the_boxes_after_it_are_read() {
        use isobmff::demux::Error;
        use isobmff_boxes::{TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry};
        use isobmff_core::BoxEncode;

        let mut tfra = TrackFragmentRandomAccessBox::default();
        tfra.track_id = 1;
        tfra.entries = vec![TrackFragmentRandomAccessEntry::default(); 2];
        let mut mfra = MovieFragmentRandomAccessBox::default();
        mfra.tfra = vec![tfra];
        mfra.mfro.parent_size = u32::try_from(mfra.encoded_len()).unwrap();
        let mut file = vec![0; usize::try_from(mfra.encoded_len()).unwrap()];
        mfra.encode(&mut file).unwrap();
        file.extend_from_slice(b"\0\0\0\x08free");
        let mut config = DemuxerConfig::default();
        config.random_access_entry_limit = 1;

        let mut demuxer = Demuxer::new(config);
        let (mut errors, mut boxes) = (Vec::new(), Vec::new());
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    Event::Error(error) => errors.push(error),
                    Event::Box { header, .. } => boxes.push(header.box_type()),
                    Event::FileType(_)
                    | Event::SegmentType(_)
                    | Event::Movie(_)
                    | Event::MovieFragment(_)
                    | Event::SegmentIndex { .. }
                    | Event::MovieFragmentRandomAccess(_)
                    | Event::SampleLocation(_)
                    | Event::Sample(_)
                    | _ => {}
                }
            }
            let Some(required) = demuxer.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
                _ => demuxer.finish(),
            }
        }

        assert_eq!(
            (errors, boxes),
            (
                vec![Error::RandomAccessEntryOverLimit {
                    offset: 0,
                    entry_count: 2,
                    limit: 1,
                }],
                vec![BoxType::compact(*b"free")]
            )
        );
    }
}
