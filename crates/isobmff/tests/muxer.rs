//! Movie files written around samples, read back by this crate's demuxer and checked against ffprobe on the conformance files

#[cfg(test)]
#[path = "support/muxer.rs"]
#[expect(
    dead_code,
    reason = "where each fragment starts is for the tests of the fragmented muxers"
)]
mod support;

#[cfg(test)]
mod tests {
    use isobmff::framing::{self, BoxReader, FramerConfig};
    use isobmff::muxer::Error;
    use isobmff::muxer::{MovieParameters, TrackParameters};
    use isobmff::{MovieFileMuxer, MuxerConfig, SampleProperties, structure_data};
    use isobmff_boxes::{EditBox, MovieBox, MovieFragmentBox};
    use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, OtherBoxes};
    use isobmff_test_support::fixtures::{encoded, media_data_header};
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use crate::support::{Read, mp4_file_type, one_sample_entry_track, packet_read, read, sample};

    /// The file a muxer writes of what `source` holds, its samples in the order read
    fn remux(source: &Read, config: MuxerConfig) -> Result<Vec<u8>, Error> {
        let mut muxer = MovieFileMuxer::new(
            MovieParameters::from(&source.movie),
            config,
            source.file_type.clone(),
        )?;
        for trak in &source.movie.trak {
            muxer.add_track(TrackParameters::from(trak))?;
        }
        let mut written = Vec::new();
        for (properties, bytes) in &source.samples {
            muxer.handle_sample(*properties, bytes)?;
            muxer.poll_output(&mut written);
        }
        let header = muxer.finish()?;
        muxer.poll_output(&mut written);
        let offset = usize::try_from(header.offset).unwrap();
        written
            .get_mut(offset..)
            .and_then(|rest| rest.get_mut(..header.bytes.len()))
            .unwrap()
            .copy_from_slice(&header.bytes);
        Ok(written)
    }

    /// The bytes a muxer opens a file of [`mp4_file_type`] with: the `ftyp` and the header of the `mdat`
    fn opening() -> Vec<u8> {
        let ftyp = mp4_file_type();
        let mut bytes = vec![0; usize::try_from(ftyp.encoded_len()).unwrap()];
        ftyp.encode(&mut bytes).unwrap();
        bytes.extend_from_slice(&media_data_header(0).unwrap());
        bytes
    }

    /// A muxer of a movie of one track, `track_ID` 1, of one sample entry
    fn muxer_of_one_track(config: MuxerConfig) -> MovieFileMuxer {
        let mut muxer =
            MovieFileMuxer::new(MovieParameters::new(1000), config, mp4_file_type()).unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();
        muxer
    }

    #[test]
    fn a_movie_file_written_again_holds_the_same_samples_whether_its_moov_was_first_or_last() {
        for file in [
            include_bytes!("fixtures/movie_moov_first.mp4").as_slice(),
            include_bytes!("fixtures/movie_moov_last.mp4"),
        ] {
            let source = read(file).unwrap();
            let written = read(&remux(&source, MuxerConfig::default()).unwrap()).unwrap();
            assert_eq!(
                (written.samples, written.errors),
                (source.samples, Vec::new())
            );
        }
    }

    #[test]
    fn the_file_is_the_ftyp_an_mdat_header_the_samples_and_the_moov_in_one_run() {
        let opening = opening();
        let mut muxer = muxer_of_one_track(MuxerConfig::default());
        let mut written = Vec::new();
        muxer.poll_output(&mut written);
        assert_eq!(written, opening);

        muxer.handle_sample(sample(0), b"abc").unwrap();
        muxer.handle_sample(sample(10), b"de").unwrap();
        muxer.poll_output(&mut written);
        let mut expected = opening.clone();
        expected.extend_from_slice(b"abcde");
        assert_eq!(written, expected);

        let header = muxer.finish().unwrap();
        assert_eq!(
            (header.offset, Some(header.bytes)),
            (
                u64::try_from(opening.len() - 16).unwrap(),
                media_data_header(5)
            )
        );
        muxer.poll_output(&mut written);
        let moov = written.get(expected.len()..).unwrap();
        assert_eq!(MovieBox::decode(moov).map(|(_, rest)| rest.len()), Ok(0));
    }

    #[test]
    fn decoding_times_count_from_the_first_sample_of_the_track() {
        let mut muxer = muxer_of_one_track(MuxerConfig::default());
        muxer.handle_sample(sample(1000), b"a").unwrap();
        muxer.handle_sample(sample(1010), b"b").unwrap();
        assert_eq!(
            muxer.handle_sample(sample(990), b"c"),
            Err(Error::StructureData(
                structure_data::Error::DecodingTimeGap {
                    track_id: 1,
                    expected: 1020,
                    found: 990,
                }
            ))
        );
        let mut written = Vec::new();
        muxer.poll_output(&mut written);
        let header = muxer.finish().unwrap();
        muxer.poll_output(&mut written);
        let offset = usize::try_from(header.offset).unwrap();
        written
            .get_mut(offset..offset + 16)
            .unwrap()
            .copy_from_slice(&header.bytes);
        assert_eq!(
            read(&written).unwrap().samples,
            [(sample(0), b"a".to_vec()), (sample(10), b"b".to_vec())]
        );
    }

    #[test]
    fn a_track_is_refused_with_a_track_id_of_0_or_one_taken_a_data_reference_elsewhere_or_after_a_sample()
     {
        let mut muxer = muxer_of_one_track(MuxerConfig::default());
        let mut elsewhere = one_sample_entry_track(3);
        if let Some(isobmff_boxes::SampleDescription::Typed(entry)) =
            elsewhere.stsd.sample_descriptions.first_mut()
        {
            entry.data_reference_index = 2;
        }
        assert_eq!(
            [
                muxer.add_track(one_sample_entry_track(0)),
                muxer.add_track(one_sample_entry_track(1)),
                muxer.add_track(elsewhere),
            ],
            [
                Err(Error::TrackId { track_id: 0 }),
                Err(Error::TrackId { track_id: 1 }),
                Err(Error::DataReferenceIndex {
                    track_id: 3,
                    data_reference_index: 2,
                }),
            ]
        );
        muxer.handle_sample(sample(0), b"").unwrap();
        assert_eq!(
            muxer.add_track(one_sample_entry_track(2)),
            Err(Error::TrackAfterMediaData { track_id: 2 })
        );
    }

    /// A child of a type no box of the crate reads, under `box_type`, as the containers keep it
    fn unknown_child(box_type: &[u8; 4]) -> OtherBoxes {
        let mut edts = b"\0\0\0\x14edts\0\0\0\x0c".to_vec();
        edts.extend_from_slice(box_type);
        edts.extend_from_slice(&[1, 2, 3, 4]);
        EditBox::decode(&edts).unwrap().0.other_boxes
    }

    #[test]
    fn the_boxes_of_a_type_the_crate_does_not_read_are_written_back_into_their_containers() {
        let mut source = read(include_bytes!("fixtures/movie_moov_first.mp4")).unwrap();
        for trak in &mut source.movie.trak {
            trak.other_boxes = unknown_child(b"trk0");
            trak.edts.get_or_insert_default().other_boxes = unknown_child(b"edt0");
            trak.mdia.other_boxes = unknown_child(b"mdi0");
            trak.mdia.minf.other_boxes = unknown_child(b"mnf0");
            trak.mdia.minf.dinf.other_boxes = unknown_child(b"dnf0");
            trak.mdia.minf.stbl.other_boxes = unknown_child(b"stb0");
        }
        source.movie.other_boxes = unknown_child(b"mov0");
        let containers = |movie: &MovieBox| {
            let mut kept = vec![movie.other_boxes.clone()];
            for trak in &movie.trak {
                kept.extend([
                    trak.other_boxes.clone(),
                    trak.edts.clone().unwrap_or_default().other_boxes,
                    trak.mdia.other_boxes.clone(),
                    trak.mdia.minf.other_boxes.clone(),
                    trak.mdia.minf.dinf.other_boxes.clone(),
                    trak.mdia.minf.stbl.other_boxes.clone(),
                ]);
            }
            kept
        };
        let written = read(&remux(&source, MuxerConfig::default()).unwrap()).unwrap();
        assert_eq!(containers(&written.movie), containers(&source.movie));
    }

    #[test]
    fn a_sample_refused_leaves_what_the_muxer_holds_as_it_was() {
        let opening = opening();
        let mut config = MuxerConfig::default();
        config.buffer_limit = opening.len() + 4;
        let mut muxer = muxer_of_one_track(config);
        let mut of_track_2 = sample(0);
        of_track_2.track_id = 2;
        let mut described_by_2 = sample(0);
        described_by_2.sample_description_index = 2;
        assert_eq!(
            [
                muxer.handle_sample(of_track_2, b"x"),
                muxer.handle_sample(described_by_2, b"x"),
                muxer.handle_sample(sample(0), b"vwxyz"),
                muxer.handle_sample(sample(0), b"wxyz"),
                muxer.handle_sample(sample(10), b""),
                muxer.handle_sample(sample(20), b"z"),
            ],
            [
                Err(Error::UnknownTrack { track_id: 2 }),
                Err(Error::StructureData(
                    structure_data::Error::SampleDescriptionIndexOutOfRange {
                        sample_description_index: 2,
                    }
                )),
                Err(Error::BufferFull {
                    length: opening.len() + 5,
                }),
                Ok(()),
                Ok(()),
                Err(Error::BufferFull {
                    length: opening.len() + 5,
                }),
            ]
        );
        let mut written = Vec::new();
        muxer.poll_output(&mut written);
        let mut expected = opening.clone();
        expected.extend_from_slice(b"wxyz");
        assert_eq!(written, expected);
        assert_eq!(muxer.handle_sample(sample(20), b"z"), Ok(()));
    }

    #[test]
    fn the_bytes_that_open_the_file_are_held_within_the_buffer_limit() {
        let opening = opening();
        let with_limit = |buffer_limit| {
            let mut config = MuxerConfig::default();
            config.buffer_limit = buffer_limit;
            MovieFileMuxer::new(MovieParameters::new(1000), config, mp4_file_type()).map(|_| ())
        };
        assert_eq!(
            [
                with_limit(1),
                with_limit(opening.len() - 1),
                with_limit(opening.len())
            ],
            [
                Err(Error::BufferFull {
                    length: opening.len()
                }),
                Err(Error::BufferFull {
                    length: opening.len()
                }),
                Ok(()),
            ]
        );
    }

    #[test]
    fn a_finished_muxer_takes_nothing_more() {
        let mut muxer = muxer_of_one_track(MuxerConfig::default());
        muxer.finish().unwrap();
        assert_eq!(
            [
                muxer.add_track(one_sample_entry_track(2)),
                muxer.handle_sample(sample(0), b"x"),
                muxer.finish().map(|_| ()),
            ],
            [
                Err(Error::Finished),
                Err(Error::Finished),
                Err(Error::Finished)
            ]
        );
    }

    #[test]
    #[ignore = "needs ffprobe"]
    fn a_fragmented_movie_written_as_a_movie_file_presents_every_sample_its_last_edit_runs_to() {
        let directory =
            std::env::temp_dir().join(format!("isobmff-muxer-edits-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut presented = Vec::new();
        for (name, file) in [
            (
                "fragmented.mp4",
                include_bytes!("fixtures/fragmented.mp4").as_slice(),
            ),
            ("segment.mp4", include_bytes!("fixtures/segment.mp4")),
        ] {
            let (source, written) = (
                directory.join(format!("source-{name}")),
                directory.join(format!("written-{name}")),
            );
            std::fs::write(&source, file).unwrap();
            std::fs::write(
                &written,
                remux(&read(file).unwrap(), MuxerConfig::default()).unwrap(),
            )
            .unwrap();
            presented.push((
                name,
                ffprobe_packets(&[], &written).len(),
                ffprobe_packets(&[], &source).len(),
            ));
        }
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(
            presented,
            [("fragmented.mp4", 70, 70), ("segment.mp4", 70, 70)]
        );
    }

    /// The file offset of each box at file level of `file`, and the offset where it ends
    fn top_level_boxes(file: &[u8]) -> Vec<(BoxType, usize, usize)> {
        let mut config = FramerConfig::new(0);
        config.gathered_box_types.clear();
        let mut reader = BoxReader::new(config);
        let mut starts = Vec::new();
        loop {
            while let Some(event) = reader.poll_event() {
                if let framing::Event::Box { offset, header } = event {
                    starts.push((header.box_type(), usize::try_from(offset).unwrap()));
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
        let ends = starts
            .iter()
            .skip(1)
            .map(|&(_, start)| start)
            .chain([file.len()]);
        starts
            .iter()
            .zip(ends)
            .map(|(&(box_type, start), end)| (box_type, start, end))
            .collect()
    }

    /// `file`, each of its tracks starting a second later: the `tfdt` of each movie fragment, and the `media_time` of each edit that is not empty, a second more in the media of the track
    fn a_second_later(file: &[u8]) -> Vec<u8> {
        let mut written = file.to_vec();
        let mut timescales = std::collections::BTreeMap::new();
        for (box_type, start, end) in top_level_boxes(file) {
            let original = file.get(start..end).unwrap();
            let bytes = if box_type == MovieBox::BOX_TYPE {
                let (mut moov, _) = MovieBox::decode(original).unwrap();
                for trak in &mut moov.trak {
                    let timescale = trak.mdia.mdhd.timescale;
                    timescales.insert(trak.tkhd.track_id, u64::from(timescale));
                    let elst = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut());
                    for entry in elst.into_iter().flat_map(|elst| elst.entries.iter_mut()) {
                        if entry.media_time >= 0 {
                            entry.media_time =
                                entry.media_time.checked_add(i64::from(timescale)).unwrap();
                        }
                    }
                }
                encoded(&moov)
            } else if box_type == MovieFragmentBox::BOX_TYPE {
                let (mut moof, _) = MovieFragmentBox::decode(original).unwrap();
                for traf in &mut moof.traf {
                    let timescale = timescales.get(&traf.tfhd.track_id).copied().unwrap();
                    if let Some(tfdt) = traf.tfdt.as_mut() {
                        tfdt.base_media_decode_time =
                            tfdt.base_media_decode_time.checked_add(timescale).unwrap();
                    }
                }
                encoded(&moof)
            } else {
                continue;
            };
            written.get_mut(start..end).unwrap().copy_from_slice(&bytes);
        }
        written
    }

    #[test]
    #[ignore = "needs ffprobe"]
    fn a_movie_whose_tracks_start_later_than_0_written_as_a_movie_file_presents_the_samples_its_edits_select()
     {
        let directory =
            std::env::temp_dir().join(format!("isobmff-muxer-later-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut presented = Vec::new();
        for (name, file) in [
            (
                "fragmented.mp4",
                include_bytes!("fixtures/fragmented.mp4").as_slice(),
            ),
            ("segment.mp4", include_bytes!("fixtures/segment.mp4")),
        ] {
            let later = a_second_later(file);
            let (source, written) = (
                directory.join(format!("source-{name}")),
                directory.join(format!("written-{name}")),
            );
            std::fs::write(&source, &later).unwrap();
            std::fs::write(
                &written,
                remux(&read(&later).unwrap(), MuxerConfig::default()).unwrap(),
            )
            .unwrap();
            presented.push((
                name,
                ffprobe_packets(&[], &written).len(),
                ffprobe_packets(&[], &source).len(),
            ));
        }
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(
            presented,
            [("fragmented.mp4", 70, 70), ("segment.mp4", 70, 70)]
        );
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_written_again_read_as_the_same_packets_and_samples() {
        let directory = std::env::temp_dir().join(format!("isobmff-muxer-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut written_files, mut fragmented_files, mut refused) =
            (Vec::new(), 0, 0, 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            // Why skip these: a file the demuxer reports an error in, such as
            // one whose samples lie in another file, has none to write.
            if !source.errors.is_empty() || source.samples.is_empty() {
                continue;
            }
            let fragmented = source.movie.mvex.is_some();
            let written = match remux(&source, MuxerConfig::default()) {
                Ok(written) => written,
                // Why accept this refusal: a `tfdt` is the sum of the
                // durations of the samples before it, §8.8.12, and the source
                // jumps.
                Err(Error::StructureData(structure_data::Error::DecodingTimeGap { .. }))
                    if fragmented =>
                {
                    refused += 1;
                    continue;
                }
                Err(error) => {
                    differences.push(format!("{}: {error}", file.name));
                    continue;
                }
            };
            written_files += 1;
            let read_back = read(&written).unwrap();
            let samples = if fragmented {
                fragmented_files += 1;
                counted_from_the_first(&source.samples)
            } else {
                source.samples.clone()
            };
            if (&read_back.samples, &read_back.errors) != (&samples, &Vec::new()) {
                differences.push(format!("{}: the samples read back differ", file.name));
            }
            // Why compare these with this crate's demuxer alone: ffprobe
            // refuses to read the source.
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let path = directory.join(&file.name);
            std::fs::write(&path, &written).unwrap();
            // Why compare a fragmented source apart: its decoding times start
            // where its first `tfdt` says, and a movie file starts them at 0,
            // §8.6.1.2. What edits present is compared by the number of
            // packets, and each packet with the edits ignored.
            if fragmented {
                let presented = |path: &std::path::Path| ffprobe_packets(&[], path).len();
                if presented(&path) != presented(&file.path) {
                    differences.push(format!(
                        "{}: ffprobe presents {} packets through the edit lists, {} of the source",
                        file.name,
                        presented(&path),
                        presented(&file.path)
                    ));
                }
                let read = |path: &std::path::Path| {
                    ffprobe_packets(&["-ignore_editlist", "1"], path)
                        .iter()
                        .map(|packet| {
                            (
                                packet.track_id,
                                packet.size,
                                packet.pts.zip(packet.dts).map(|(pts, dts)| pts - dts),
                                packet.key,
                            )
                        })
                        .collect::<Vec<_>>()
                };
                if read(&path) != read(&file.path) {
                    differences.push(format!(
                        "{}: ffprobe reads other packets with the edits ignored",
                        file.name
                    ));
                }
                continue;
            }
            let theirs = ffprobe_packets(&[], &file.path)
                .iter()
                .map(packet_read)
                .collect::<Vec<_>>();
            let ours = ffprobe_packets(&[], &path)
                .iter()
                .map(packet_read)
                .collect::<Vec<_>>();
            if ours != theirs {
                differences.push(format!(
                    "{}: ffprobe reads {} packets, {} from the source; first differing {:?}",
                    file.name,
                    ours.len(),
                    theirs.len(),
                    ours.iter()
                        .zip(&theirs)
                        .find(|(ours, theirs)| ours != theirs)
                ));
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{written_files} files written, {fragmented_files} of them fragmented, {refused} refused for a gap in decoding times"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(written_files > 0 && fragmented_files > 0);
    }

    /// `samples`, the decoding times of each track counted from that of its first sample, as a sample table declares them
    fn counted_from_the_first(
        samples: &[(SampleProperties, Vec<u8>)],
    ) -> Vec<(SampleProperties, Vec<u8>)> {
        let mut firsts = std::collections::BTreeMap::new();
        samples
            .iter()
            .map(|(properties, bytes)| {
                let first = *firsts
                    .entry(properties.track_id)
                    .or_insert(properties.decoding_time);
                let properties = SampleProperties {
                    decoding_time: properties.decoding_time.saturating_sub(first),
                    ..*properties
                };
                (properties, bytes.clone())
            })
            .collect()
    }
}
