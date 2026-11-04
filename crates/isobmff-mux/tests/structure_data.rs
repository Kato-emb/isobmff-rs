//! Samples the structure-data of a file declares, checked on the files of MPEG's `FileFormatConformance` against the packets `ffprobe` reads
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests, and `ISOBMFF_FFPROBE`
//! names `ffprobe` where it is not on the path.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::ops::Range;

    use isobmff::boxes::{DataEntry, MovieBox, MovieFragmentBox};
    use isobmff::structure_data::{DataReference, movie_fragment_random_access_start};
    use isobmff::timeline::Timeline;
    use isobmff::{BoxDefinition, BoxType, Boxes, ReadError, SampleLocation};
    use isobmff_mux::{DemuxEvent, Demuxer};
    use isobmff_test_support::{
        ConformanceFile, conformance_files, ffprobe_packets, gpac_children,
    };

    /// What a [`Demuxer`] read of a file
    struct Resolved {
        /// Locations of the samples the `moov` and the `moof`s declare, in the order the boxes lie
        locations: Vec<SampleLocation>,
        /// Whether a `styp` was reported
        segment_type: bool,
        /// File offsets the demuxer wanted input from
        wanted: Vec<u64>,
        /// File offsets of the payload of each `mdat`
        media_data: Vec<Range<u64>>,
        /// Demuxer the file was read with, which keeps the movie, the segment indexes and the `mfra`
        demuxer: Demuxer,
    }

    /// Reads `file` with a [`Demuxer`] from wherever it wants, discarding every track once the `moov` is read
    fn resolve(file: &[u8]) -> Result<Resolved, ReadError> {
        let mut resolved = Resolved {
            locations: Vec::new(),
            segment_type: false,
            wanted: Vec::new(),
            media_data: Vec::new(),
            demuxer: Demuxer::new(),
        };
        resolve_from(&mut resolved, file)?;
        Ok(resolved)
    }

    /// Reads `file` into `resolved` from wherever its demuxer wants, discarding every track once the `moov` is read
    fn resolve_from(resolved: &mut Resolved, file: &[u8]) -> Result<(), ReadError> {
        let demuxer = &mut resolved.demuxer;
        loop {
            let events = match demuxer.wanted_input() {
                Some(wanted) => {
                    resolved.wanted.push(wanted.offset());
                    let input = file
                        .get(usize::try_from(wanted.offset()).unwrap()..)
                        .unwrap_or_default();
                    match input {
                        [] => demuxer.finish()?,
                        input => demuxer.handle_input(wanted.offset(), input)?,
                    }
                }
                // Nothing is wanted past a box that runs to the end of the file: it ends with the file.
                None => match demuxer.finish()? {
                    None => break,
                    event => event,
                },
            };
            if let Some(event) = events {
                match event {
                    DemuxEvent::Movie { samples, .. } => {
                        resolved.locations.extend(samples);
                        let track_ids: Vec<_> = demuxer
                            .movie()
                            .iter()
                            .flat_map(|movie| &movie.trak)
                            .map(|trak| trak.tkhd.track_id)
                            .collect();
                        for track_id in track_ids {
                            demuxer.discard_track(track_id);
                        }
                    }
                    DemuxEvent::MovieFragment { samples, .. } => {
                        resolved.locations.extend(samples);
                    }
                    DemuxEvent::SegmentType { .. } => resolved.segment_type = true,
                    DemuxEvent::MediaData {
                        header, byte_range, ..
                    } => {
                        let start = byte_range.offset().saturating_add(header.encoded_len());
                        resolved.media_data.push(
                            start..byte_range.length().map_or(u64::MAX, |length| {
                                byte_range.offset().saturating_add(length)
                            }),
                        );
                    }
                    DemuxEvent::FileType { .. }
                    | DemuxEvent::SegmentIndex { .. }
                    | DemuxEvent::MovieFragmentRandomAccess { .. }
                    | DemuxEvent::Other { .. }
                    | _ => {}
                }
            }
        }

        Ok(())
    }

    /// Returns the type and file offset of each top-level box of `file`
    fn top_level(file: &[u8]) -> Vec<(BoxType, u64)> {
        let mut offset = 0_u64;
        Boxes::new(file)
            .map(|raw_box| {
                let raw_box = raw_box.unwrap();
                let at = offset;
                offset = offset
                    .saturating_add(raw_box.header().encoded_len())
                    .saturating_add(u64::try_from(raw_box.payload().len()).unwrap());
                (raw_box.header().box_type(), at)
            })
            .collect()
    }

    /// Track, data reference entry, and the location the entry names, of samples whose media data is in another file
    type ElsewhereEntry = (u32, u16, &'static str);

    /// Files whose media data is partly in other files, with each track and data reference entry that names another file
    const EXTERNAL_DATA: [(&str, &[ElsewhereEntry]); 3] = [
        (
            "02_dref_edts_img.mp4",
            &[
                (101, 1, "media/count_english.aac"),
                (201, 1, "media/logo.jpg"),
            ],
        ),
        ("04_bifs_video.mp4", &[(3, 1, "media/count_video.cmp")]),
        ("a6_tone_multifile.mp4", &[(1, 2, "./myData.dat")]),
    ];

    /// Files `ffprobe` 9.0.1 does not read: it fails with "error reading header"
    const UNREAD_BY_FFPROBE: [&str; 3] = [
        "12_metas_v2.mp4",
        "16_vtt.mp4",
        "FX-VY-9436R.3_qhd-variant.mp4",
    ];

    /// Files whose edit list leaves samples out of the presentation, which `ffprobe` does not report as packets, and renumbers the dts of the rest
    const EDITED_OUT: [&str; 1] = ["a8-foreman_QCIF_edit.mp4"];

    /// Track, file offset, length and dts of a sample or a packet
    type Placed = (u32, u64, u64, i64);

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe on them"]
    fn the_samples_of_a_file_are_the_packets_ffprobe_reads() {
        for ConformanceFile {
            name, bytes, path, ..
        } in conformance_files()
        {
            let resolved = resolve(&bytes).expect(&name);
            let named = |entry: &DataEntry| {
                if let DataEntry::Url(url) = entry {
                    url.location
                        .as_ref()
                        .and_then(|location| location.as_str())
                        .unwrap_or_default()
                        .to_owned()
                } else {
                    format!("{entry:?}")
                }
            };
            let elsewhere = |data_reference: DataReference<'_>| {
                (!data_reference.entry().is_in_this_file()).then(|| {
                    (
                        data_reference.track_id(),
                        data_reference.index(),
                        named(data_reference.entry()),
                    )
                })
            };
            let demuxer = &resolved.demuxer;
            let named_by_locations: Vec<_> = resolved
                .locations
                .iter()
                .map(|location| demuxer.data_reference(location.data_reference()))
                .collect();
            assert!(
                named_by_locations.iter().zip(&resolved.locations).all(
                    |(data_reference, location)| data_reference
                        .is_some_and(|data_reference| data_reference.track_id()
                            == location.properties().track_id)
                ),
                "{name}: the key of each location names an entry of its own track"
            );
            let listed: BTreeSet<_> = demuxer.data_references().filter_map(elsewhere).collect();
            let external: BTreeSet<_> = named_by_locations
                .into_iter()
                .flatten()
                .filter_map(elsewhere)
                .collect();
            let expected_external: BTreeSet<_> = EXTERNAL_DATA
                .iter()
                .filter(|(external, _)| *external == name)
                .flat_map(|(_, entries)| entries.iter())
                .map(|&(track_id, data_reference_index, named)| {
                    (track_id, data_reference_index, named.to_owned())
                })
                .collect();
            assert_eq!(
                (&listed, &external),
                (&expected_external, &expected_external),
                "{name}: data reference entries that name another file, and the samples located in one"
            );
            let read_past: Vec<_> = resolved.demuxer.leniencies().collect();
            assert!(read_past.is_empty(), "{name} is read past: {read_past:?}");
            assert_indexes_point_at_boxes(&name, &resolved.demuxer, &top_level(&bytes));
            assert_eq!(
                resolved.segment_type,
                name == "21_segment.mp4",
                "{name}: styp"
            );
            let wanted_media_data: Vec<_> = resolved
                .wanted
                .iter()
                .filter(|offset| {
                    resolved
                        .media_data
                        .iter()
                        .any(|payload| payload.contains(offset))
                })
                .collect();
            assert!(
                wanted_media_data.is_empty(),
                "{name}: the payload of a mdat is wanted at {wanted_media_data:?}"
            );
            if UNREAD_BY_FFPROBE.contains(&name.as_str()) {
                continue;
            }
            let movie = resolved.demuxer.movie().expect(&name);
            // Why not compare the tracks in other files: ffprobe lists only some of their packets, which it does not read
            let in_file =
                |track_id: &u32| external.iter().all(|(external, ..)| external != track_id);
            let mut samples = resolved
                .locations
                .iter()
                .filter(|location| in_file(&location.properties().track_id))
                .map(|location| {
                    let properties = location.properties();
                    let byte_range = location.byte_range();
                    (
                        properties.track_id,
                        byte_range.start,
                        byte_range.end - byte_range.start,
                        i64::try_from(properties.decoding_time)
                            .unwrap()
                            .checked_sub(dts_shift(movie, properties.track_id))
                            .unwrap(),
                    )
                })
                .collect::<Vec<Placed>>();
            let mut packets = ffprobe_packets(&path)
                .into_iter()
                .filter(|packet| in_file(&packet.track_id))
                .map(|packet| {
                    let dts = packet.dts.expect("ffprobe reports the dts of every packet");
                    (packet.track_id, packet.pos, packet.size, dts)
                })
                .collect::<Vec<Placed>>();
            samples.sort_unstable();
            packets.sort_unstable();

            if EDITED_OUT.contains(&name.as_str()) {
                let placed = |placed: &[Placed]| {
                    placed
                        .iter()
                        .map(|&(track, offset, length, _)| (track, offset, length))
                        .collect::<Vec<_>>()
                };
                let mut kept = placed(&samples);
                kept.retain(|sample| placed(&packets).contains(sample));
                assert_eq!(kept, placed(&packets), "{name}: the packets are samples");
                continue;
            }
            assert_eq!(samples, packets, "{name}: the samples are the packets");
        }
    }

    /// Returns how far `ffprobe` moves the dts of track `track_id` back from its decoding time
    ///
    /// It takes off the media time the first edit of the track starts at, and
    /// as much again as the most negative composition offset of the `ctts`,
    /// so that no sample is composed before the presentation starts.
    fn dts_shift(movie: &MovieBox, track_id: u32) -> i64 {
        let trak = movie
            .trak
            .iter()
            .find(|trak| trak.tkhd.track_id == track_id)
            .unwrap();
        let media_time = trak
            .edts
            .iter()
            .flat_map(|edts| &edts.elst)
            .flat_map(|elst| &elst.entries)
            .map(|entry| entry.media_time)
            .find(|media_time| *media_time >= 0)
            .unwrap_or_default();
        let earliest_composition_offset = trak
            .mdia
            .minf
            .stbl
            .ctts
            .iter()
            .flat_map(|ctts| &ctts.entries)
            .map(|entry| entry.sample_offset)
            .min()
            .unwrap_or_default();

        media_time
            .checked_sub(earliest_composition_offset.min(0))
            .unwrap()
    }

    /// Asserts that each media subsegment of the segment indexes, and each `moof` the `mfra` lists, starts at one of the top-level boxes `top_level`
    fn assert_indexes_point_at_boxes(name: &str, demuxer: &Demuxer, top_level: &[(BoxType, u64)]) {
        let box_at = |offset: u64| {
            top_level
                .iter()
                .find(|(_, box_offset)| *box_offset == offset)
                .map(|(box_type, _)| *box_type)
        };
        for subsegment in demuxer
            .segment_indexes()
            .iter()
            .flat_map(|index| index.subsegments())
        {
            assert!(
                box_at(subsegment.byte_range().start).is_some(),
                "{name}: subsegment {subsegment:?} starts at no top-level box"
            );
        }
        for entry in demuxer
            .movie_fragment_random_access()
            .iter()
            .flat_map(|mfra| &mfra.tfra)
            .flat_map(|tfra| &tfra.entries)
        {
            assert_eq!(
                box_at(entry.moof_offset),
                Some(MovieFragmentBox::BOX_TYPE),
                "{name}: tfra entry {entry:?} points at no moof"
            );
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn a_fragment_resumed_at_from_the_mfra_resolves_only_from_its_tfdt() {
        let ConformanceFile { bytes, .. } = conformance_files()
            .into_iter()
            .find(|file| file.name == "fragment_random_access-2.mp4")
            .unwrap();
        let mut resolved = resolve(&bytes).unwrap();
        let held = |demuxer: &Demuxer| {
            (
                demuxer.movie().cloned(),
                demuxer.segment_indexes().to_vec(),
                demuxer.movie_fragment_random_access().cloned(),
            )
        };
        let read_whole = held(&resolved.demuxer);

        let file_length = u64::try_from(bytes.len()).unwrap();
        let mfra = movie_fragment_random_access_start(file_length, bytes.last_chunk().unwrap());
        resolved.demuxer.resume_at(mfra.unwrap()).unwrap();
        resolve_from(&mut resolved, &bytes).unwrap();
        let last_moof = resolved
            .demuxer
            .movie_fragment_random_access()
            .iter()
            .flat_map(|mfra| &mfra.tfra)
            .flat_map(|tfra| &tfra.entries)
            .map(|entry| entry.moof_offset)
            .max()
            .unwrap();
        resolved.demuxer.resume_at(last_moof).unwrap();

        let refused = resolve_from(&mut resolved, &bytes);
        assert!(
            matches!(refused, Err(ReadError::MissingDecodingTime { .. })),
            "{refused:?}"
        );
        assert_eq!(held(&resolved.demuxer), read_whole);
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_padding_of_each_sample_is_what_gpac_reports_of_the_padb() {
        let mut checked = 0;
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let padded: Vec<_> = report
                .pointer("/IsoMediaFile/MovieBox")
                .map(gpac_children)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|trak| {
                    let padb = trak
                        .pointer("/MediaBox/MediaInformationBox/SampleTableBox/PaddingBitsBox")?;
                    Some((trak, padb))
                })
                .collect();
            if padded.is_empty() {
                continue;
            }
            let locations = resolve(&bytes).expect(&name).locations;
            for (trak, padb) in padded {
                let track_id: u32 = trak
                    .pointer("/TrackHeaderBox/@TrackID")
                    .and_then(|track_id| track_id.as_str()?.parse().ok())
                    .unwrap();
                let reported: Vec<u8> = padb["PaddingBitsEntry"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|entry| entry["@PaddingBits"].as_str().unwrap().parse().unwrap())
                    .collect();
                let mut samples: Vec<_> = locations
                    .iter()
                    .map(SampleLocation::properties)
                    .filter(|properties| properties.track_id == track_id)
                    .map(|properties| {
                        (
                            properties.decoding_time,
                            properties.sample_flags.sample_padding_value,
                        )
                    })
                    .collect();
                samples.sort_unstable();
                let paddings: Vec<u8> = samples.into_iter().map(|(_, padding)| padding).collect();

                assert_eq!(paddings, reported, "{name}: track {track_id}");
                checked += 1;
            }
        }
        assert_ne!(checked, 0, "no file has a padb");
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe on them"]
    fn the_samples_ffprobe_presents_from_their_start_are_presented_at_its_presentation_timestamps()
    {
        let expected = BTreeMap::from([
            (("02_dref_edts_img.mp4", 101), BTreeSet::from([0])),
            (("17_negative_ctso.mp4", 1), BTreeSet::from([0])),
            (("a8-foreman_QCIF_edit.mp4", 1), BTreeSet::from([15])),
            (("a9-aac-samplegroups-edit.mp4", 1), BTreeSet::from([0])),
            (("restricted.mp4", 1), BTreeSet::from([0])),
        ]);
        let mut origins = BTreeMap::<(&str, u32), BTreeSet<i128>>::new();
        for file in conformance_files() {
            let Some(&(name, _)) = expected.keys().find(|(name, _)| *name == file.name) else {
                continue;
            };
            let resolved = resolve(&file.bytes).unwrap();
            let movie = resolved.demuxer.movie().unwrap();
            for packet in ffprobe_packets(&file.path) {
                if !expected.contains_key(&(name, packet.track_id)) {
                    continue;
                }
                let location = resolved
                    .locations
                    .iter()
                    .find(|location| {
                        location.properties().track_id == packet.track_id
                            && location.byte_range().start == packet.pos
                    })
                    .unwrap();
                let properties = location.properties();
                let composition_time = i128::from(properties.decoding_time)
                    + i128::from(properties.sample_composition_time_offset);
                let first = Timeline::new(movie, packet.track_id)
                    .unwrap()
                    .present(properties)
                    .next();
                let entered =
                    first.map(|presented| i128::from(presented.media_time()) - composition_time);
                if packet.discard {
                    assert!(
                        entered.is_none_or(|entered| entered > 0),
                        "{name}: a packet ffprobe discards is presented from its start: {packet:?}"
                    );
                    continue;
                }
                let (first, entered) = first.zip(entered).unwrap();
                let mdhd = &movie
                    .trak
                    .iter()
                    .find(|trak| trak.tkhd.track_id == packet.track_id)
                    .unwrap()
                    .mdia
                    .mdhd;
                let (numerator, denominator) = packet.time_base;
                let pts = i128::from(packet.pts.unwrap())
                    * i128::from(numerator)
                    * i128::from(mdhd.timescale)
                    / i128::from(denominator);
                origins
                    .entry((name, packet.track_id))
                    .or_default()
                    .insert(i128::from(first.presentation_time()) - entered - pts);
            }
        }

        assert_eq!(origins, expected);
    }
}
