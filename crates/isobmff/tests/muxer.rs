//! Movie files written around samples, read back by this crate's demuxer and checked against ffprobe on the conformance files

#[cfg(test)]
mod tests {
    use isobmff::demuxer::Event;
    use isobmff::framing::media_data_header;
    use isobmff::muxer::{Error, FileStructure, MovieDescription, TrackDescription};
    use isobmff::{Demuxer, DemuxerConfig, Muxer, MuxerConfig, SampleLocation, SampleProperties};
    use isobmff_boxes::{FileTypeBox, MovieBox, SampleFlags};
    use isobmff_core::{BoxEncode, FourCc};
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

    /// The file a muxer writes of what `source` holds, its samples in the order read, each written where the muxer places it
    fn remux(source: &Read) -> Result<Vec<u8>, Error> {
        let structure = FileStructure::NonFragmented {
            file_type: source.file_type.clone(),
        };
        let mut muxer = Muxer::new(
            MovieDescription::from(&source.movie),
            MuxerConfig::default(),
            structure,
        )?;
        for trak in &source.movie.trak {
            muxer.add_track(TrackDescription::from(trak))?;
        }
        let mut written = Vec::new();
        for (properties, bytes) in &source.samples {
            let location = muxer.handle_sample(*properties, u32::try_from(bytes.len()).unwrap())?;
            muxer.poll_output(&mut written);
            written.resize(usize::try_from(location.offset).unwrap(), 0);
            written.extend_from_slice(bytes);
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

    /// A muxer of a movie of one track, `track_ID` 1, of one sample entry
    fn muxer_of_one_track() -> Muxer {
        let mut muxer = Muxer::new(
            MovieDescription::new(1000),
            MuxerConfig::default(),
            FileStructure::NonFragmented {
                file_type: mp4_file_type(),
            },
        )
        .unwrap();
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
            let written = read(&remux(&source).unwrap()).unwrap();
            assert_eq!(
                (written.samples, written.errors),
                (source.samples, Vec::new())
            );
        }
    }

    #[test]
    fn the_file_opens_with_the_ftyp_and_an_mdat_header_the_one_finish_returns_replaces() {
        let ftyp = mp4_file_type();
        let mut opening = vec![0; usize::try_from(ftyp.encoded_len()).unwrap()];
        ftyp.encode(&mut opening).unwrap();
        let ftyp_length = u64::try_from(opening.len()).unwrap();
        opening.extend_from_slice(&media_data_header(0).unwrap());

        let mut muxer = muxer_of_one_track();
        let mut written = Vec::new();
        muxer.poll_output(&mut written);
        assert_eq!(written, opening);

        let location = muxer.handle_sample(sample(0), 5).unwrap();
        muxer.advance_position(3).unwrap();
        let header = muxer.finish().unwrap();
        assert_eq!(
            (location.offset, header.offset, Some(header.bytes)),
            (ftyp_length + 16, ftyp_length, media_data_header(5 + 3))
        );
    }

    #[test]
    fn decoding_times_count_from_the_first_sample_of_the_track() {
        let mut muxer = muxer_of_one_track();
        let taken = [
            muxer.handle_sample(sample(1000), 1),
            muxer.handle_sample(sample(1010), 1),
            muxer.handle_sample(sample(990), 1),
        ];
        let start = taken[0].as_ref().unwrap().offset;
        assert_eq!(
            taken,
            [
                Ok(SampleLocation {
                    properties: sample(0),
                    offset: start,
                    size: 1,
                }),
                Ok(SampleLocation {
                    properties: sample(10),
                    offset: start + 1,
                    size: 1,
                }),
                Err(Error::StructureData(
                    isobmff::structure_data::Error::DecodingTimeGap {
                        track_id: 1,
                        expected: 1020,
                        found: 990,
                    }
                )),
            ]
        );
    }

    #[test]
    fn a_track_is_refused_with_a_track_id_of_0_or_one_taken_a_data_reference_elsewhere_or_after_the_media_data()
     {
        let mut muxer = muxer_of_one_track();
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
        muxer.advance_position(1).unwrap();
        assert_eq!(
            muxer.add_track(one_sample_entry_track(2)),
            Err(Error::TrackAfterMediaData { track_id: 2 })
        );
    }

    #[test]
    fn a_sample_refused_leaves_the_media_data_as_it_was() {
        let mut muxer = muxer_of_one_track();
        let mut of_track_2 = sample(0);
        of_track_2.track_id = 2;
        let mut described_by_2 = sample(0);
        described_by_2.sample_description_index = 2;
        assert_eq!(
            [
                muxer.handle_sample(of_track_2, 1).map(|_| ()),
                muxer.handle_sample(described_by_2, 1).map(|_| ()),
            ],
            [
                Err(Error::UnknownTrack { track_id: 2 }),
                Err(Error::StructureData(
                    isobmff::structure_data::Error::SampleDescriptionIndexOutOfRange {
                        sample_description_index: 2,
                    }
                )),
            ]
        );
        let first = muxer.handle_sample(sample(0), 1).unwrap();
        let mut fresh = muxer_of_one_track();
        assert_eq!(first, fresh.handle_sample(sample(0), 1).unwrap());
    }

    #[test]
    fn the_media_data_ends_at_the_last_offset_a_file_can_have_and_not_past_it() {
        let mut muxer = muxer_of_one_track();
        let start = muxer.handle_sample(sample(0), 0).unwrap().offset;
        muxer
            .advance_position(u64::MAX.checked_sub(start + 1).unwrap())
            .unwrap();
        assert_eq!(
            [
                muxer.handle_sample(sample(10), 2).map(|_| ()),
                muxer.advance_position(2),
            ],
            [Err(Error::Overflow), Err(Error::Overflow)]
        );
        assert_eq!(
            [
                muxer
                    .handle_sample(sample(10), 1)
                    .map(|location| location.offset),
                muxer
                    .handle_sample(sample(20), 0)
                    .map(|location| location.offset),
            ],
            [Ok(u64::MAX - 1), Ok(u64::MAX)]
        );
        assert_eq!(muxer.advance_position(1), Err(Error::Overflow));
    }

    #[test]
    fn a_finished_muxer_takes_nothing_more() {
        let mut muxer = muxer_of_one_track();
        muxer.finish().unwrap();
        assert_eq!(
            [
                muxer.add_track(one_sample_entry_track(2)),
                muxer.handle_sample(sample(0), 1).map(|_| ()),
                muxer.advance_position(1),
                muxer.finish().map(|_| ()),
            ],
            [
                Err(Error::Finished),
                Err(Error::Finished),
                Err(Error::Finished),
                Err(Error::Finished),
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
            let written = match remux(&source) {
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
