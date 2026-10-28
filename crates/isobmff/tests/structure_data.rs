//! Samples the structure-data of a file declares, checked on the files of MPEG's `FileFormatConformance` against the packets `ffprobe` reads
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests, and `ISOBMFF_FFPROBE`
//! names `ffprobe` where it is not on the path.

#[cfg(test)]
mod tests {
    use isobmff::boxes::{
        MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentIndexBox,
    };
    use isobmff::framing::{BoxEvent, BoxReader};
    use isobmff::structure_data::StructureDataReader;
    use isobmff::{BoxDecode, BoxDefinition, BoxType, DemuxError, Error, SampleLocation};
    use isobmff_test_support::{ConformanceFile, conformance_files, ffprobe_packets};

    /// Why the samples of a file do not resolve
    #[derive(Debug, PartialEq)]
    enum Failure {
        /// The reader refuses the file
        Demux(DemuxError),
        /// A `moov` or a `moof` does not decode
        Decode(Error),
    }

    impl From<DemuxError> for Failure {
        fn from(error: DemuxError) -> Self {
            Self::Demux(error)
        }
    }

    impl From<Error> for Failure {
        fn from(error: Error) -> Self {
            Self::Decode(error)
        }
    }

    /// What the structure-data of a file resolves to
    struct Resolved {
        /// Locations of the samples the `moov` and the `moof`s declare, in the order the boxes lie
        locations: Vec<SampleLocation>,
        /// Type and file offset of each top-level box
        top_level: Vec<(BoxType, u64)>,
        /// Reader the structure-data was read with, which keeps the segment indexes and the `mfra`
        structure_data: StructureDataReader,
    }

    /// Types of the top-level boxes of structure-data that [`resolve`] reads
    const STRUCTURE_DATA: [BoxType; 4] = [
        MovieBox::BOX_TYPE,
        MovieFragmentBox::BOX_TYPE,
        SegmentIndexBox::BOX_TYPE,
        MovieFragmentRandomAccessBox::BOX_TYPE,
    ];

    /// Resolves the `moov`, the `moof`s, the `sidx`es and the `mfra` of `file`
    fn resolve(file: &[u8]) -> Result<Resolved, Failure> {
        let mut reader = BoxReader::new();
        let mut resolved = Resolved {
            locations: Vec::new(),
            top_level: Vec::new(),
            structure_data: StructureDataReader::new(u64::MAX),
        };
        while let Some(wanted) = reader.wanted_input() {
            let input = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            if input.is_empty() {
                break;
            }
            for event in reader.handle_input(wanted.offset(), input)? {
                match event {
                    BoxEvent::Header(header) => {
                        let byte_range = reader.open_box().unwrap();
                        resolved
                            .top_level
                            .push((header.box_type(), byte_range.offset()));
                        if STRUCTURE_DATA.contains(&header.box_type()) {
                            reader.gather(u64::MAX)?;
                        } else {
                            reader.skip()?;
                        }
                    }
                    BoxEvent::End => {
                        if let Some(raw_box) = reader.gathered_box() {
                            let &(box_type, offset) = resolved.top_level.last().unwrap();
                            let end = offset
                                .saturating_add(raw_box.header().encoded_len())
                                .saturating_add(u64::try_from(raw_box.payload().len()).unwrap());
                            read_structure_data(
                                &mut resolved,
                                box_type,
                                raw_box.payload(),
                                offset,
                                end,
                            )?;
                        }
                    }
                    BoxEvent::Payload(_) | _ => {}
                }
            }
        }
        let _events = reader.finish()?;

        Ok(resolved)
    }

    /// Reads the `payload` of a box of structure-data of `box_type`, which lies from file offset `offset` to `end`, into `resolved`
    fn read_structure_data(
        resolved: &mut Resolved,
        box_type: BoxType,
        payload: &[u8],
        offset: u64,
        end: u64,
    ) -> Result<(), Failure> {
        let structure_data = &mut resolved.structure_data;
        if box_type == MovieBox::BOX_TYPE {
            let movie = MovieBox::decode_payload(payload)?;
            resolved
                .locations
                .extend(structure_data.handle_movie(movie)?);
        } else if box_type == MovieFragmentBox::BOX_TYPE {
            let movie_fragment = MovieFragmentBox::decode_payload(payload)?;
            resolved
                .locations
                .extend(structure_data.handle_movie_fragment(&movie_fragment, offset)?);
        } else if box_type == SegmentIndexBox::BOX_TYPE {
            structure_data.handle_segment_index(&SegmentIndexBox::decode_payload(payload)?, end)?;
        } else {
            structure_data.handle_movie_fragment_random_access(
                MovieFragmentRandomAccessBox::decode_payload(payload)?,
            );
        }

        Ok(())
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
                        Some(Failure::Demux(DemuxError::ExternalDataReference {
                            track_id: refused_track,
                            data_reference_index: refused_index,
                            ..
                        })) if refused_track == track_id && refused_index == data_reference_index
                    ),
                    "{name}, whose media data is in another file: {refused:?}"
                );
                continue;
            }
            let resolved = resolve(&bytes).expect(&name);
            assert_indexes_point_at_boxes(&name, &resolved);
            if UNREAD_BY_FFPROBE.contains(&name.as_str()) {
                continue;
            }
            let movie = resolved.structure_data.movie().expect(&name);
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

    /// Asserts that each media subsegment of the segment indexes, and each `moof` the `mfra` lists, starts at a top-level box
    fn assert_indexes_point_at_boxes(name: &str, resolved: &Resolved) {
        let structure_data = &resolved.structure_data;
        let box_at = |offset: u64| {
            resolved
                .top_level
                .iter()
                .find(|(_, box_offset)| *box_offset == offset)
                .map(|(box_type, _)| *box_type)
        };
        for subsegment in structure_data
            .segment_indexes()
            .iter()
            .flat_map(|index| index.subsegments())
        {
            assert!(
                box_at(subsegment.byte_range().start).is_some(),
                "{name}: subsegment {subsegment:?} starts at no top-level box"
            );
        }
        for entry in structure_data
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
}
