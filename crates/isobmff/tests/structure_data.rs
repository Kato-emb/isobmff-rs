//! Samples the structure-data of a file declares, checked on the files of MPEG's `FileFormatConformance` against the packets `ffprobe` reads
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests, and `ISOBMFF_FFPROBE`
//! names `ffprobe` where it is not on the path.

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use isobmff::boxes::{MovieBox, MovieFragmentBox};
    use isobmff::structure_data::movie_fragment_random_access_start;
    use isobmff::{BoxDefinition, BoxType, Boxes, DemuxError, DemuxEvent, Demuxer, SampleLocation};
    use isobmff_test_support::{ConformanceFile, conformance_files, ffprobe_packets};

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

    /// Reads `file` with a [`Demuxer`] from wherever it wants, skipping each `mdat`
    fn resolve(file: &[u8]) -> Result<Resolved, DemuxError> {
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

    /// Reads `file` into `resolved` from wherever its demuxer wants, skipping each `mdat`
    fn resolve_from(resolved: &mut Resolved, file: &[u8]) -> Result<(), DemuxError> {
        let demuxer = &mut resolved.demuxer;
        while let Some(wanted) = demuxer.wanted_input() {
            resolved.wanted.push(wanted.offset());
            let input = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            let events = match input {
                [] => demuxer.finish()?,
                input => demuxer.handle_input(wanted.offset(), input)?,
            };
            if let Some(event) = events {
                match event {
                    DemuxEvent::Movie { samples, .. }
                    | DemuxEvent::MovieFragment { samples, .. } => {
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
                        demuxer.skip()?;
                    }
                    DemuxEvent::FileType { .. }
                    | DemuxEvent::SegmentIndex { .. }
                    | DemuxEvent::MovieFragmentRandomAccess { .. }
                    | DemuxEvent::MediaDataPayload { .. }
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

    /// Files whose media data is in other files, and the error the movie is refused with
    const EXTERNAL_DATA: [(&str, u32, u16); 3] = [
        ("02_dref_edts_img.mp4", 201, 1),
        ("04_bifs_video.mp4", 3, 1),
        ("a6_tone_multifile.mp4", 1, 2),
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
            if let Some(&(_, track_id, data_reference_index)) = EXTERNAL_DATA
                .iter()
                .find(|(external, ..)| *external == name)
            {
                let refused = resolve(&bytes).err();
                assert!(
                    matches!(
                        refused,
                        Some(DemuxError::ExternalDataReference {
                            track_id: refused_track,
                            data_reference_index: refused_index,
                            ..
                        }) if refused_track == track_id && refused_index == data_reference_index
                    ),
                    "{name}, whose media data is in another file: {refused:?}"
                );
                continue;
            }
            let resolved = resolve(&bytes).expect(&name);
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
            let mut samples = resolved
                .locations
                .iter()
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
            matches!(refused, Err(DemuxError::MissingDecodingTime { .. })),
            "{refused:?}"
        );
        assert_eq!(held(&resolved.demuxer), read_whole);
    }
}
