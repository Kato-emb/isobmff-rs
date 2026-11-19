//! Samples assembled from the bytes of their media data, however the bytes are handed over

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use isobmff::boxes::SampleFlags;
    use isobmff::media_data::{Error, Event, MediaDataReader};
    use isobmff::{Demuxer, DemuxerConfig, SampleLocation, SampleProperties, demux};

    /// What a reader reported, owning what it lent
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Reported {
        Sample(SampleLocation, Vec<u8>),
        Error(Error),
        Other(String),
    }

    fn drain(reader: &mut MediaDataReader, reported: &mut Vec<Reported>) {
        while let Some(event) = reader.poll_event() {
            reported.push(match event {
                Event::Sample(sample) => Reported::Sample(sample.location(), sample.bytes.to_vec()),
                Event::Error(error) => Reported::Error(error),
                other => Reported::Other(format!("{other:?}")),
            });
        }
    }

    /// A sample of track `track_id` at `offset` of `size` bytes, its decoding time its offset
    fn at(track_id: u32, offset: u64, size: u32) -> SampleLocation {
        SampleLocation {
            properties: SampleProperties {
                track_id,
                decoding_time: offset,
                sample_duration: 1,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            offset,
            size,
        }
    }

    /// Bytes of a media data where the byte at each offset is that offset, modulo 251
    fn media_data(length: usize) -> Vec<u8> {
        (0..length)
            .map(|offset| u8::try_from(offset % 251).unwrap())
            .collect()
    }

    fn reader_of(locations: &[SampleLocation], sample_size_limit: u64) -> MediaDataReader {
        let mut reader = MediaDataReader::new(1024, sample_size_limit);
        for &location in locations {
            reader.handle_location(location).unwrap();
        }
        reader
    }

    /// The events of `locations` assembled from `file` read where the reader asks, with the ranges it handed over
    fn read_where_required(
        file: &[u8],
        locations: &[SampleLocation],
        sample_size_limit: u64,
    ) -> (Vec<Reported>, Vec<Range<usize>>) {
        let mut reader = reader_of(locations, sample_size_limit);
        let (mut reported, mut handed) = (Vec::new(), Vec::new());
        loop {
            drain(&mut reader, &mut reported);
            let Some(required) = reader.required_input() else {
                break;
            };
            let start = usize::try_from(required.position).unwrap();
            let length = usize::try_from(required.length.unwrap()).unwrap();
            let range = start.min(file.len())..start.saturating_add(length).min(file.len());
            if range.is_empty() {
                reader.finish();
            } else {
                reader.handle_input(required.position, file.get(range.clone()).unwrap());
                handed.push(range);
            }
        }
        reader.finish();
        drain(&mut reader, &mut reported);
        (reported, handed)
    }

    /// The events of `locations` assembled from `file` handed over from its start in pieces of `piece` bytes, ending the input where the reader wants bytes before the piece
    fn read_in_pieces(
        file: &[u8],
        locations: &[SampleLocation],
        sample_size_limit: u64,
        piece: usize,
    ) -> Vec<Reported> {
        let mut reader = reader_of(locations, sample_size_limit);
        let mut reported = Vec::new();
        drain(&mut reader, &mut reported);
        'pieces: for (index, bytes) in file.chunks(piece).enumerate() {
            let position = u64::try_from(index.checked_mul(piece).unwrap()).unwrap();
            let end = position
                .checked_add(u64::try_from(bytes.len()).unwrap())
                .unwrap();
            while let Some(required) = reader.required_input()
                && required.position < end
            {
                if required.position < position {
                    break 'pieces;
                }
                reader.handle_input(position, bytes);
                drain(&mut reader, &mut reported);
            }
        }
        reader.finish();
        drain(&mut reader, &mut reported);
        reported
    }

    /// The events of `locations` from `file`, the same however it is handed over
    fn read_every_way(
        file: &[u8],
        locations: &[SampleLocation],
        sample_size_limit: u64,
    ) -> Vec<Reported> {
        let (reported, _) = read_where_required(file, locations, sample_size_limit);
        for piece in [1, 4096, file.len().max(1)] {
            assert_eq!(
                read_in_pieces(file, locations, sample_size_limit, piece),
                reported,
                "in pieces of {piece}"
            );
        }
        reported
    }

    /// `location` lent with the bytes of `file` it spans
    fn lent(file: &[u8], location: SampleLocation) -> Reported {
        let start = usize::try_from(location.offset).unwrap();
        let end = start
            .checked_add(usize::try_from(location.size).unwrap())
            .unwrap();
        Reported::Sample(location, file.get(start..end).unwrap().to_vec())
    }

    #[test]
    fn the_samples_are_lent_in_the_order_of_their_offsets_whatever_order_they_are_handed_over_in() {
        let file = media_data(10_000);
        let locations = [
            at(2, 9000, 500),
            at(1, 100, 30),
            at(1, 4000, 4000),
            at(2, 130, 70),
        ];
        let reported = read_every_way(&file, &locations, 1 << 20);
        assert_eq!(
            reported,
            [
                lent(&file, at(1, 100, 30)),
                lent(&file, at(2, 130, 70)),
                lent(&file, at(1, 4000, 4000)),
                lent(&file, at(2, 9000, 500)),
            ]
        );
    }

    #[test]
    fn samples_at_the_same_offset_are_lent_in_the_order_they_were_handed_over_in() {
        let file = media_data(100);
        let locations = [at(2, 10, 0), at(1, 10, 4), at(3, 10, 0)];
        assert_eq!(
            read_every_way(&file, &locations, 1 << 20),
            [
                lent(&file, at(2, 10, 0)),
                lent(&file, at(1, 10, 4)),
                lent(&file, at(3, 10, 0)),
            ]
        );
    }

    #[test]
    fn a_sample_of_no_bytes_is_lent_without_input() {
        let mut reader = reader_of(&[at(1, 50, 0)], 1 << 20);
        let mut reported = Vec::new();
        drain(&mut reader, &mut reported);
        assert_eq!(
            (reported, reader.required_input()),
            (vec![Reported::Sample(at(1, 50, 0), Vec::new())], None)
        );
    }

    #[test]
    fn a_sample_over_the_size_limit_is_reported_and_its_bytes_never_requested() {
        let file = media_data(1000);
        let locations = [at(1, 0, 10), at(1, 10, 500), at(1, 510, 20)];
        let (reported, handed) = read_where_required(&file, &locations, 100);
        assert_eq!(
            reported,
            [
                lent(&file, at(1, 0, 10)),
                Reported::Error(Error::SampleOverLimit {
                    location: at(1, 10, 500),
                    limit: 100
                }),
                lent(&file, at(1, 510, 20)),
            ]
        );
        assert!(
            handed
                .iter()
                .all(|range| range.end <= 10 || range.start >= 510),
            "{handed:?}"
        );
        assert_eq!(read_every_way(&file, &locations, 100), reported);
    }

    #[test]
    fn a_sample_past_the_count_limit_is_refused_and_the_reader_left_as_it_was() {
        let file = media_data(100);
        let mut reader = MediaDataReader::new(2, 1 << 20);
        reader.handle_location(at(1, 0, 10)).unwrap();
        reader.handle_location(at(1, 10, 10)).unwrap();
        assert_eq!(
            reader.handle_location(at(1, 20, 10)),
            Err(Error::SampleCountOverLimit {
                sample_count: 3,
                limit: 2
            })
        );
        let mut reported = Vec::new();
        reader.handle_input(0, &file);
        drain(&mut reader, &mut reported);
        reader.handle_location(at(1, 20, 10)).unwrap();
        while let Some(required) = reader.required_input() {
            reader.handle_input(
                required.position,
                file.get(usize::try_from(required.position).unwrap()..)
                    .unwrap(),
            );
            drain(&mut reader, &mut reported);
        }
        assert_eq!(
            reported,
            [
                lent(&file, at(1, 0, 10)),
                lent(&file, at(1, 10, 10)),
                lent(&file, at(1, 20, 10)),
            ]
        );
    }

    /// The input ends inside the second sample, and before any byte of the third
    #[test]
    fn a_sample_the_input_ends_inside_is_reported_truncated_and_one_none_of_whose_bytes_came_is_not()
     {
        let file = media_data(1000);
        let locations = [at(1, 0, 10), at(1, 10, 100), at(1, 500, 10)];
        let truncated = file.get(..50).unwrap();
        assert_eq!(
            read_every_way(truncated, &locations, 1 << 20),
            [
                lent(&file, at(1, 0, 10)),
                Reported::Error(Error::TruncatedSample {
                    location: at(1, 10, 100)
                }),
            ]
        );
    }

    #[test]
    fn the_reader_wants_no_input_once_it_holds_no_sample_or_the_input_ended() {
        let mut empty = MediaDataReader::new(1, 1);
        assert_eq!((empty.has_samples(), empty.required_input()), (false, None));
        let mut reader = reader_of(&[at(1, 0, 10)], 1 << 20);
        assert!(reader.has_samples());
        reader.finish();
        assert_eq!(
            (
                reader.poll_event().is_none(),
                reader.has_samples(),
                reader.required_input()
            ),
            (true, false, None)
        );
        empty.finish();
    }

    /// What the demuxer reads of `file`: the location of each sample of a track whose data references name another file, and the bytes of each sample it lends, every track subscribed to
    fn demuxed(file: &[u8]) -> (Vec<SampleLocation>, Vec<(SampleProperties, Vec<u8>)>) {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let (mut elsewhere, mut located, mut lent) = (Vec::new(), Vec::new(), Vec::new());
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    demux::Event::Movie(movie) => {
                        for trak in &movie.trak {
                            demuxer.subscribe_track(trak.tkhd.track_id);
                            if trak
                                .mdia
                                .minf
                                .dinf
                                .dref
                                .data_entries
                                .iter()
                                .any(|entry| !entry.is_in_this_file())
                            {
                                elsewhere.push(trak.tkhd.track_id);
                            }
                        }
                    }
                    demux::Event::SampleLocation(location)
                        if elsewhere.contains(&location.properties.track_id) =>
                    {
                        located.push(location);
                    }
                    demux::Event::Sample(sample) => {
                        lent.push((sample.properties, sample.bytes.to_vec()));
                    }
                    demux::Event::SampleLocation(_) | demux::Event::Box { .. } | _ => {}
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
        (located, lent)
    }

    /// `(properties, bytes)` of each sample, by track and decoding time
    fn by_track(mut samples: Vec<(SampleProperties, Vec<u8>)>) -> Vec<(SampleProperties, Vec<u8>)> {
        samples.sort_by_key(|(properties, _)| (properties.track_id, properties.decoding_time));
        samples
    }

    /// The samples `reported` lends, or `None` where it reports anything else
    fn assembled(reported: Vec<Reported>) -> Option<Vec<(SampleProperties, Vec<u8>)>> {
        reported
            .into_iter()
            .map(|reported| match reported {
                Reported::Sample(location, bytes) => Some((location.properties, bytes)),
                Reported::Error(_) | Reported::Other(_) => None,
            })
            .collect()
    }

    /// The fixtures of media data in another file, and those of the file itself they were made from
    #[test]
    fn the_samples_of_a_file_a_data_reference_names_assembled_from_its_bytes_are_those_of_the_file_they_came_from()
     {
        for (name, file, media_data, source) in [
            (
                "external_movie.mp4",
                &include_bytes!("fixtures/external_movie.mp4")[..],
                &include_bytes!("fixtures/external_movie.dat")[..],
                &include_bytes!("fixtures/movie_moov_last.mp4")[..],
            ),
            (
                "external_fragments.mp4",
                include_bytes!("fixtures/external_fragments.mp4"),
                include_bytes!("fixtures/external_fragments.dat"),
                include_bytes!("fixtures/fragmented.mp4"),
            ),
        ] {
            let (located, _) = demuxed(file);
            let tracks: Vec<u32> = located
                .iter()
                .map(|location| location.properties.track_id)
                .collect();
            let (_, lent) = demuxed(source);
            let expected: Vec<_> = lent
                .into_iter()
                .filter(|(properties, _)| tracks.contains(&properties.track_id))
                .collect();
            assert!(!expected.is_empty(), "{name}");
            assert_eq!(
                assembled(read_every_way(media_data, &located, 1 << 20)).map(by_track),
                Some(by_track(expected)),
                "{name}"
            );
        }
    }

    /// `a6_tone_multifile.mp4`, whose one track names `./myData.dat` beside it
    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR"]
    fn the_samples_of_the_conformance_file_of_media_data_in_another_file_assemble_from_its_bytes() {
        let file = isobmff_test_support::conformance_files()
            .into_iter()
            .find(|file| file.name == "a6_tone_multifile.mp4")
            .unwrap();
        let media_data = std::fs::read(file.path.with_file_name("myData.dat")).unwrap();
        let (located, lent_by_demuxer) = demuxed(&file.bytes);
        let reported = read_every_way(&media_data, &located, 1 << 20);
        let expected: Vec<_> = located
            .iter()
            .map(|&location| lent(&media_data, location))
            .collect();
        assert_eq!(
            (located.len(), lent_by_demuxer.len(), reported),
            (48, 0, expected)
        );
    }

    #[test]
    #[should_panic(expected = "after the offset")]
    fn input_that_starts_after_the_required_offset_is_a_bug_of_the_caller() {
        let mut reader = reader_of(&[at(1, 10, 10)], 1 << 20);
        reader.handle_input(11, &[0; 10]);
    }

    #[test]
    #[should_panic(expected = "after finish")]
    fn input_after_finish_is_a_bug_of_the_caller() {
        let mut reader = reader_of(&[at(1, 10, 10)], 1 << 20);
        reader.finish();
        reader.handle_input(10, &[0; 10]);
    }

    #[test]
    #[should_panic(expected = "after finish")]
    fn a_location_after_finish_is_a_bug_of_the_caller() {
        let mut reader = MediaDataReader::new(1, 1);
        reader.finish();
        let _ = reader.handle_location(at(1, 0, 1));
    }
}
