//! Movie fragments written from where their samples lie, resolved back by this crate's reader; the files a muxer writes of them are checked against ffprobe in the tests of the muxers

#[cfg(test)]
mod tests {
    use isobmff::structure_data::{
        Error, MovieFragmentWriter, StructureDataReader, TrackSampleDescription,
    };
    use isobmff::{SampleLocation, SampleProperties};
    use isobmff_boxes::{
        MovieBox, MovieFragmentBox, SampleFlags, TrackFragmentBaseMediaDecodeTimeBox,
        TrackFragmentBox, TrackRunBox, TrackRunSamples,
    };
    use isobmff_core::{BoxDecode, BoxEncode};
    use isobmff_test_support::fixtures::{in_another_file, movie, track};

    /// The header of an `mdat` of `payload_length` bytes, 16 bytes with a `largesize`
    fn media_data_header(payload_length: u64) -> Option<[u8; 16]> {
        let size = isobmff_core::ExtendedSize::new(payload_length.checked_add(16)?)?;
        let header = isobmff_core::BoxHeader::new(
            isobmff_core::BoxType::compact(*b"mdat"),
            isobmff_core::BoxSize::Extended(size),
        )?;
        let mut bytes = [0; 16];
        header.encode(&mut bytes).ok()?;
        Some(bytes)
    }

    /// A movie of track 1, of one sample entry, track 2, of two, and track 3, whose media data is in another file
    fn three_tracks() -> MovieBox {
        let mut two_entries = track(2, 1, &[], &[], &[]);
        let stsd = &mut two_entries.mdia.minf.stbl.stsd.sample_descriptions;
        stsd.push(stsd.first().unwrap().clone());
        movie([
            track(1, 1, &[], &[], &[]),
            two_entries,
            in_another_file(track(3, 1, &[], &[], &[])),
        ])
    }

    fn non_sync() -> SampleFlags {
        let mut flags = SampleFlags::default();
        flags.sample_is_non_sync_sample = true;
        flags.sample_depends_on = 1;
        flags
    }

    fn sample(track_id: u32, decoding_time: u64, sample_duration: u32) -> SampleProperties {
        SampleProperties {
            track_id,
            decoding_time,
            sample_duration,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::default(),
            sample_description_index: 1,
        }
    }

    /// The sample `properties` declares, at `offset` from the first byte of the media data of its fragment, of `size` bytes
    fn at(properties: SampleProperties, offset: u64, size: u32) -> SampleLocation {
        SampleLocation {
            properties,
            offset,
            size,
        }
    }

    /// A writer of fragments of the tracks of `moov`, each of at most `sample_count_limit` samples
    fn writer_of(moov: &MovieBox, sample_count_limit: u64) -> MovieFragmentWriter {
        let tracks: Vec<_> = moov
            .trak
            .iter()
            .map(|trak| TrackSampleDescription {
                track_id: trak.tkhd.track_id,
                stsd: &trak.mdia.minf.stbl.stsd,
                dref: &trak.mdia.minf.dinf.dref,
            })
            .collect();
        MovieFragmentWriter::new(&tracks, sample_count_limit)
    }

    /// Finishes the fragment of `writer` and appends its `moof` to `file`, with the header of an `mdat` of `media_data_length` bytes
    fn append_fragment(
        writer: &mut MovieFragmentWriter,
        media_data_length: u64,
        file: &mut Vec<u8>,
    ) {
        let moof = writer.finish();
        let start = file.len();
        file.resize(
            start
                .checked_add(usize::try_from(moof.encoded_len()).unwrap())
                .unwrap(),
            0,
        );
        moof.encode(file.get_mut(start..).unwrap()).unwrap();
        file.extend_from_slice(&media_data_header(media_data_length).unwrap());
    }

    /// Writes the fragment of `samples`, their bytes one after another in the order given, after `file`, and their bytes after it, each sample as many bytes of its index as its size; returns where each sample lies in `file`
    fn write_fragment(
        writer: &mut MovieFragmentWriter,
        samples: &[(SampleProperties, u32)],
        file: &mut Vec<u8>,
    ) -> Vec<SampleLocation> {
        let mut offset = 0;
        for &(properties, size) in samples {
            writer
                .handle_location(at(properties, offset, size))
                .unwrap();
            offset = offset.checked_add(u64::from(size)).unwrap();
        }
        append_fragment(writer, offset, file);
        let mut locations = Vec::new();
        for (index, &(properties, size)) in samples.iter().enumerate() {
            locations.push(SampleLocation {
                properties,
                offset: u64::try_from(file.len()).unwrap(),
                size,
            });
            let byte = u8::try_from(index).unwrap();
            file.extend(std::iter::repeat_n(byte, usize::try_from(size).unwrap()));
        }
        locations
    }

    /// The `moof` that starts at `offset` in `file`
    fn movie_fragment_at(file: &[u8], offset: u64) -> MovieFragmentBox {
        let bytes = file.get(usize::try_from(offset).unwrap()..).unwrap();
        MovieFragmentBox::decode(bytes).unwrap().0
    }

    #[test]
    fn the_fragments_declare_their_samples_where_their_bytes_follow_one_another_across_tracks() {
        let moov = three_tracks();
        let mut writer = writer_of(&moov, 16);
        let mut reordered = sample(2, 100, 20);
        reordered.sample_composition_time_offset = -5;
        let mut described_by_2 = sample(2, 120, 30);
        described_by_2.sample_description_index = 2;
        let fragments = [
            vec![
                (sample(1, 0, 10), 3),
                (
                    SampleProperties {
                        sample_flags: non_sync(),
                        ..sample(1, 10, 10)
                    },
                    4,
                ),
                (reordered, 5),
                (
                    SampleProperties {
                        sample_flags: non_sync(),
                        ..sample(1, 20, 12)
                    },
                    6,
                ),
                (described_by_2, 7),
            ],
            vec![
                (
                    SampleProperties {
                        decoding_time: 150,
                        ..described_by_2
                    },
                    2,
                ),
                (sample(1, 32, 10), 0),
            ],
        ];

        let mut reader = StructureDataReader::new(16);
        assert_eq!(reader.handle_movie(&moov).unwrap().count(), 0);
        let mut file = Vec::new();
        for samples in &fragments {
            let offset = u64::try_from(file.len()).unwrap();
            let mut expected = write_fragment(&mut writer, samples, &mut file);
            let moof = movie_fragment_at(&file, offset);
            let mut resolved: Vec<_> = reader
                .handle_movie_fragment(&moof, offset)
                .unwrap()
                .collect();
            resolved.sort_by_key(|location| location.offset);
            expected.sort_by_key(|location| location.offset);
            assert_eq!(resolved, expected);
        }
    }

    #[test]
    fn the_samples_lie_where_their_offsets_place_them_in_the_media_data_whatever_the_order_handed_over()
     {
        let moov = three_tracks();
        let mut writer = writer_of(&moov, 16);
        let samples = [
            at(sample(1, 0, 10), 10, 1),
            at(sample(2, 0, 10), 0, 10),
            at(sample(1, 10, 10), 11, 2),
            at(sample(1, 20, 10), 20, 1),
        ];
        for location in samples {
            writer.handle_location(location).unwrap();
        }
        let mut file = Vec::new();
        append_fragment(&mut writer, 21, &mut file);
        let moof = movie_fragment_at(&file, 0);
        let media_data = u64::try_from(file.len()).unwrap();
        file.extend([0; 21]);

        let mut reader = StructureDataReader::new(16);
        assert_eq!(reader.handle_movie(&moov).unwrap().count(), 0);
        let mut resolved: Vec<_> = reader.handle_movie_fragment(&moof, 0).unwrap().collect();
        resolved.sort_by_key(|location| location.offset);
        let mut expected = samples.map(|location| SampleLocation {
            offset: media_data + location.offset,
            ..location
        });
        expected.sort_by_key(|location| location.offset);
        assert_eq!(
            (
                resolved,
                moof.traf
                    .iter()
                    .map(|traf| traf.trun.len())
                    .collect::<Vec<_>>(),
                file.get(
                    usize::try_from(media_data).unwrap() - 16..usize::try_from(media_data).unwrap()
                ),
            ),
            (
                expected.to_vec(),
                vec![2, 1],
                media_data_header(21).as_ref().map(<[u8; 16]>::as_slice),
            )
        );
    }

    #[test]
    fn what_every_sample_of_a_track_fragment_shares_is_declared_once_in_its_header() {
        let moov = three_tracks();
        let mut writer = writer_of(&moov, 16);
        let later = SampleProperties {
            sample_flags: non_sync(),
            ..sample(1, 0, 10)
        };
        let samples = [
            (sample(1, 1000, 10), 4),
            (
                SampleProperties {
                    decoding_time: 1010,
                    ..later
                },
                4,
            ),
            (
                SampleProperties {
                    decoding_time: 1020,
                    ..later
                },
                4,
            ),
        ];
        let mut file = Vec::new();
        let locations = write_fragment(&mut writer, &samples, &mut file);
        let moof = movie_fragment_at(&file, 0);

        let mut expected = TrackFragmentBox::default();
        expected.tfhd.default_base_is_moof = true;
        expected.tfhd.track_id = 1;
        expected.tfhd.sample_description_index = Some(1);
        expected.tfhd.default_sample_duration = Some(10);
        expected.tfhd.default_sample_size = Some(4);
        expected.tfhd.default_sample_flags = Some(non_sync());
        let mut tfdt = TrackFragmentBaseMediaDecodeTimeBox::default();
        tfdt.base_media_decode_time = 1000;
        expected.tfdt = Some(tfdt);
        let mut trun = TrackRunBox::default();
        trun.data_offset = Some(i32::try_from(locations.first().unwrap().offset).unwrap());
        trun.first_sample_flags = Some(SampleFlags::default());
        trun.samples = TrackRunSamples::EmptyRows { sample_count: 3 };
        expected.trun = vec![trun];
        assert_eq!(moof.traf, [expected]);
    }

    #[test]
    fn a_sample_refused_leaves_the_fragment_as_it_was() {
        let moov = three_tracks();
        let mut writer = writer_of(&moov, 2);
        let mut described_by_2 = sample(1, 0, 10);
        described_by_2.sample_description_index = 2;
        let mut far_composed = sample(1, 10, 10);
        far_composed.sample_composition_time_offset = i64::from(i32::MAX) + 1;
        let mut flags_too_wide = sample(1, 10, 10);
        flags_too_wide.sample_flags.is_leading = 4;
        let results = [
            writer.handle_location(at(sample(4, 0, 10), 0, 1)),
            writer.handle_location(at(described_by_2, 0, 1)),
            writer.handle_location(at(sample(3, 0, 10), 0, 1)),
            writer.handle_location(at(sample(1, 0, 10), 0, 1)),
            writer.handle_location(at(sample(1, 5, 10), 1, 1)),
            writer.handle_location(at(far_composed, 1, 1)),
            writer.handle_location(at(flags_too_wide, 1, 1)),
            writer.handle_location(at(sample(1, 10, 10), 1, u32::MAX)),
            writer.handle_location(at(sample(1, 10, 10), 1, 1)),
            writer.handle_location(at(sample(1, 20, 10), 2, 1)),
        ];
        assert_eq!(
            results.map(|result| result.map_err(|error| {
                if let Error::Encode(_) = error {
                    None
                } else {
                    Some(error)
                }
            })),
            [
                Err(Some(Error::UnknownTrack { track_id: 4 })),
                Err(Some(Error::SampleDescriptionIndexOutOfRange {
                    sample_description_index: 2
                })),
                Err(Some(Error::MediaDataElsewhere {
                    track_id: 3,
                    data_reference_index: 1
                })),
                Ok(()),
                Err(Some(Error::DecodingTimeGap {
                    track_id: 1,
                    expected: 10,
                    found: 5
                })),
                Err(Some(Error::CompositionOffsetOutOfRange {
                    sample_composition_time_offset: i64::from(i32::MAX) + 1
                })),
                Err(None),
                Err(Some(Error::Overflow { track_id: 1 })),
                Ok(()),
                Err(Some(Error::SampleCountOverLimit {
                    sample_count: 3,
                    limit: 2
                })),
            ]
        );

        let mut taken_alone = writer_of(&moov, 2);
        taken_alone
            .handle_location(at(sample(1, 0, 10), 0, 1))
            .unwrap();
        taken_alone
            .handle_location(at(sample(1, 10, 10), 1, 1))
            .unwrap();
        assert_eq!(writer.finish(), taken_alone.finish());
    }

    #[test]
    fn each_fragment_takes_the_next_sequence_number_and_one_of_no_sample_declares_no_track_fragment()
     {
        let moov = three_tracks();
        let mut writer = writer_of(&moov, 16);
        let empty = writer.finish();
        let mut file = Vec::new();
        let mut sequence_numbers = vec![empty.mfhd.sequence_number];
        for decoding_time in [0, 10] {
            let offset = u64::try_from(file.len()).unwrap();
            write_fragment(&mut writer, &[(sample(1, decoding_time, 10), 1)], &mut file);
            sequence_numbers.push(movie_fragment_at(&file, offset).mfhd.sequence_number);
        }
        assert_eq!(
            (empty.traf.len(), writer.has_samples(), sequence_numbers),
            (0, false, vec![1, 2, 3])
        );
    }
}
