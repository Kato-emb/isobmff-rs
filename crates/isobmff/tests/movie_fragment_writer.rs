//! Movie fragments written from where their samples lie, resolved back by this crate's reader; the files a muxer writes of them are checked against ffprobe in the tests of the muxers

#[cfg(test)]
mod tests {
    use isobmff::structure_data::{Error, MovieFragmentWriter, StructureDataReader};
    use isobmff::{SampleLocation, SampleProperties};
    use isobmff_boxes::{
        MovieBox, MovieFragmentBox, SampleFlags, TrackFragmentBaseMediaDecodeTimeBox,
        TrackFragmentBox, TrackRunBox, TrackRunSamples,
    };
    use isobmff_core::BoxDecode;
    use isobmff_test_support::fixtures::{in_another_file, movie, track};

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

    /// Writes the fragment of `samples` after `file`, and their bytes after it, each sample as many bytes of its index as its size; returns where each sample lies
    fn write_fragment(
        writer: &mut MovieFragmentWriter,
        samples: &[(SampleProperties, u32)],
        file: &mut Vec<u8>,
    ) -> Vec<SampleLocation> {
        for &(properties, size) in samples {
            writer.handle_sample(properties, size).unwrap();
        }
        writer.write_movie_fragment(file).unwrap();
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
        let mut writer = MovieFragmentWriter::new(&moov, 16);
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
    fn what_every_sample_of_a_track_fragment_shares_is_declared_once_in_its_header() {
        let moov = three_tracks();
        let mut writer = MovieFragmentWriter::new(&moov, 16);
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
        let mut writer = MovieFragmentWriter::new(&moov, 2);
        let mut described_by_2 = sample(1, 0, 10);
        described_by_2.sample_description_index = 2;
        let mut far_composed = sample(1, 10, 10);
        far_composed.sample_composition_time_offset = i64::from(i32::MAX) + 1;
        let mut flags_too_wide = sample(1, 10, 10);
        flags_too_wide.sample_flags.is_leading = 4;
        let results = [
            writer.handle_sample(sample(4, 0, 10), 1),
            writer.handle_sample(described_by_2, 1),
            writer.handle_sample(sample(3, 0, 10), 1),
            writer.handle_sample(sample(1, 0, 10), 1),
            writer.handle_sample(sample(1, 5, 10), 1),
            writer.handle_sample(far_composed, 1),
            writer.handle_sample(flags_too_wide, 1),
            writer.handle_sample(sample(1, 10, 10), u32::MAX),
            writer.handle_sample(sample(1, 10, 10), 1),
            writer.handle_sample(sample(1, 20, 10), 1),
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

        let mut written = Vec::new();
        writer.write_movie_fragment(&mut written).unwrap();
        let mut taken_alone = MovieFragmentWriter::new(&moov, 2);
        taken_alone.handle_sample(sample(1, 0, 10), 1).unwrap();
        taken_alone.handle_sample(sample(1, 10, 10), 1).unwrap();
        let mut expected = Vec::new();
        taken_alone.write_movie_fragment(&mut expected).unwrap();
        assert_eq!(written, expected);
    }

    #[test]
    fn each_fragment_takes_the_next_sequence_number_and_one_of_no_sample_is_not_written() {
        let moov = three_tracks();
        let mut writer = MovieFragmentWriter::new(&moov, 16);
        let mut file = Vec::new();
        writer.write_movie_fragment(&mut file).unwrap();
        assert_eq!(file, Vec::<u8>::new());
        let mut sequence_numbers = Vec::new();
        for decoding_time in [0, 10] {
            let offset = u64::try_from(file.len()).unwrap();
            write_fragment(&mut writer, &[(sample(1, decoding_time, 10), 1)], &mut file);
            sequence_numbers.push(movie_fragment_at(&file, offset).mfhd.sequence_number);
        }
        assert_eq!(sequence_numbers, [1, 2]);
    }
}
