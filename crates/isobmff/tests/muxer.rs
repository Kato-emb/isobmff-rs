//! Movie files written around samples, read back by this crate's demuxer and checked against ffprobe on the conformance files

#[cfg(test)]
mod tests {
    use isobmff::demuxer::Event;
    use isobmff::framing::media_data_header;
    use isobmff::muxer::{Error, MovieDescription, TrackDescription};
    use isobmff::{Demuxer, DemuxerConfig, MovieFileMuxer, MuxerConfig, SampleProperties};
    use isobmff_boxes::{EditBox, FileTypeBox, MovieBox, SampleFlags};
    use isobmff_core::{BoxDecode, BoxEncode, FourCc, OtherBoxes};
    use isobmff_test_support::fixtures::track;
    use isobmff_test_support::{FfprobePacket, conformance_files, ffprobe, ffprobe_packets};

    /// What a demuxer reads of a file: its `ftyp`, its `moov`, and every sample of its tracks with its bytes, in file order
    struct Read {
        file_type: FileTypeBox,
        movie: MovieBox,
        samples: Vec<(SampleProperties, Vec<u8>)>,
        errors: Vec<isobmff::demuxer::Error>,
    }

    fn read(file: &[u8]) -> Option<Read> {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let (mut file_type, mut movie) = (None, None);
        let (mut samples, mut errors) = (Vec::new(), Vec::new());
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    Event::FileType(ftyp) => file_type = Some(ftyp),
                    Event::Movie(moov) => {
                        for trak in &moov.trak {
                            demuxer.subscribe_track(trak.tkhd.track_id);
                        }
                        movie = Some(*moov);
                    }
                    Event::Sample(sample) => {
                        samples.push((sample.properties, sample.bytes.to_vec()));
                    }
                    Event::Error(error) => errors.push(error),
                    Event::SampleLocation(_) | Event::Box { .. } | _ => {}
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
        Some(Read {
            file_type: file_type?,
            movie: movie?,
            samples,
            errors,
        })
    }

    /// The file a muxer writes of what `source` holds, its samples in the order read
    fn remux(source: &Read, config: MuxerConfig) -> Result<Vec<u8>, Error> {
        let mut muxer = MovieFileMuxer::new(
            MovieDescription::from(&source.movie),
            config,
            source.file_type.clone(),
        )?;
        for trak in &source.movie.trak {
            muxer.add_track(TrackDescription::from(trak))?;
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

    /// The file type of an MP4 file, ISO/IEC 14496-14
    fn mp4_file_type() -> FileTypeBox {
        let mut ftyp = FileTypeBox::default();
        ftyp.major_brand = FourCc::new(*b"isom");
        ftyp.compatible_brands = vec![FourCc::new(*b"isom")];
        ftyp
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
            MovieFileMuxer::new(MovieDescription::new(1000), config, mp4_file_type()).unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();
        muxer
    }

    fn one_sample_entry_track(track_id: u32) -> TrackDescription {
        let stsd = track(track_id, 1, &[], &[], &[]).mdia.minf.stbl.stsd;
        TrackDescription::new(track_id, 1000, FourCc::new(*b"vide"), stsd)
    }

    /// A sync sample of track 1 at `decoding_time`, lasting 10
    fn sample(decoding_time: u64) -> SampleProperties {
        SampleProperties {
            track_id: 1,
            decoding_time,
            sample_duration: 10,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::default(),
            sample_description_index: 1,
        }
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
                isobmff::structure_data::Error::DecodingTimeGap {
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
            elsewhere.sample_entries.sample_descriptions.first_mut()
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
                    isobmff::structure_data::Error::SampleDescriptionIndexOutOfRange {
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
            MovieFileMuxer::new(MovieDescription::new(1000), config, mp4_file_type()).map(|_| ())
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

    /// What ffprobe reads of a packet, apart from where it lies
    fn packet_read(packet: &FfprobePacket) -> (u32, u64, Option<i64>, Option<i64>, bool, bool) {
        (
            packet.track_id,
            packet.size,
            packet.dts,
            packet.pts,
            packet.discard,
            packet.key,
        )
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_written_again_read_as_the_same_packets_and_samples() {
        let directory = std::env::temp_dir().join(format!("isobmff-muxer-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut written_files) = (Vec::new(), 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            // Why skip these: a fragmented file holds samples the `moov` does
            // not declare, and a file the demuxer reports an error in, such
            // as one whose samples lie in another file, has none to write.
            if source.movie.mvex.is_some() || !source.errors.is_empty() || source.samples.is_empty()
            {
                continue;
            }
            let written = match remux(&source, MuxerConfig::default()) {
                Ok(written) => written,
                Err(error) => {
                    differences.push(format!("{}: {error}", file.name));
                    continue;
                }
            };
            written_files += 1;
            let read_back = read(&written).unwrap();
            if (&read_back.samples, &read_back.errors) != (&source.samples, &Vec::new()) {
                differences.push(format!("{}: the samples read back differ", file.name));
            }
            // Why compare these with this crate's demuxer alone: ffprobe
            // refuses to read the source.
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let path = directory.join(&file.name);
            std::fs::write(&path, &written).unwrap();
            let theirs = ffprobe_packets(&file.path)
                .iter()
                .map(packet_read)
                .collect::<Vec<_>>();
            let ours = ffprobe_packets(&path)
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
        assert_eq!(differences, Vec::<String>::new());
        assert!(written_files > 0);
    }
}
