//! The samples a writer laid down as a non-fragmented movie file, read back off that file

#[cfg(test)]
mod tests {
    use isobmff::{
        BoxEvent, BoxType, MovieBox, MovieHeaderBox, Mp4EpochSeconds, NonFragmentedReader,
        NonFragmentedWriter, Sample,
    };
    use isobmff_test_support::{events_of, file_type, track};

    /// Ticks a second the media of the movie is timed in
    const TIMESCALE: u32 = 90_000;

    /// Movie of two tracks declaring no sample yet, the template the writer fills in
    fn movie() -> MovieBox {
        let epoch = Mp4EpochSeconds::from_seconds(0);

        MovieBox::new(
            MovieHeaderBox::new(epoch, epoch, TIMESCALE, 0, 3),
            vec![track(1), track(2)],
            None,
        )
        .unwrap()
    }

    /// The samples the two tracks carry, chunk by chunk, the tracks taking turns
    fn declared_chunks() -> Vec<Vec<Sample>> {
        let video =
            |decode_time, data: &[u8]| Sample::new(1, decode_time, 3_000, 0, 0, 1, data.to_vec());
        let audio =
            |decode_time, data: &[u8]| Sample::new(2, decode_time, 1_024, 0, 0, 1, data.to_vec());

        vec![
            vec![video(0, b"VIDEO_01"), video(3_000, b"VIDEO_02")],
            vec![
                audio(0, b"AUD1"),
                audio(1_024, b"AUD2"),
                audio(2_048, b"AUD3"),
            ],
            vec![video(6_000, b"VIDEO_03")],
            vec![audio(3_072, b"AUD4")],
        ]
    }

    /// The file the chunks make: the brands, one `mdat` per chunk, then the movie
    fn written_file(chunks: Vec<Vec<Sample>>) -> Vec<u8> {
        let mut writer = NonFragmentedWriter::new();
        let mut file = Vec::new();

        writer.handle_file_type(file_type()).unwrap();
        writer.handle_movie(movie()).unwrap();
        for chunk in chunks {
            writer.begin_chunk().unwrap();
            for sample in chunk {
                writer.handle_sample(sample).unwrap();
            }
        }
        writer.finish().unwrap();

        while let Some(written) = writer.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    /// The samples `file` carries, read off it in order and then off the bytes it wants fetched
    fn read_back(file: &[u8]) -> (MovieBox, Vec<Sample>) {
        let mut reader = NonFragmentedReader::new();
        let mut samples = Vec::new();

        reader.handle_input(file).unwrap();
        while let Some(wanted) = reader.wanted_extent() {
            let fetched = file
                .get(usize::try_from(wanted.start).unwrap()..usize::try_from(wanted.end).unwrap())
                .unwrap();
            reader.handle_data(wanted.start, fetched).unwrap();
        }
        reader.finish().unwrap();
        while let Some(sample) = reader.poll_sample() {
            samples.push(sample);
        }

        (reader.movie().cloned().unwrap(), samples)
    }

    #[test]
    fn the_samples_are_read_back_as_they_were_handed_over_chunk_by_chunk() {
        let file = written_file(declared_chunks());

        let (_movie, samples) = read_back(&file);

        assert_eq!(samples, declared_chunks().concat());
    }

    #[test]
    fn each_chunk_is_laid_down_as_its_own_media_data_box_and_the_movie_comes_last() {
        let file = written_file(declared_chunks());

        let top_level: Vec<BoxType> = events_of(&file, file.len())
            .unwrap()
            .into_iter()
            .filter_map(|(_extent, event)| {
                if let BoxEvent::Header(header) = event {
                    Some(header.box_type())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            top_level,
            [b"ftyp", b"mdat", b"mdat", b"mdat", b"mdat", b"moov"]
                .map(|fourcc| BoxType::compact(*fourcc))
        );
    }

    #[test]
    fn the_chunk_offsets_of_each_track_point_at_the_media_data_of_its_own_chunks() {
        let file = written_file(declared_chunks());

        let (movie, _samples) = read_back(&file);
        let chunk_offsets = |track_id: u32| -> Vec<u64> {
            movie
                .trak()
                .iter()
                .find(|trak| trak.tkhd().track_id() == track_id)
                .unwrap()
                .mdia()
                .minf()
                .stbl()
                .stco()
                .entries()
                .iter()
                .map(|entry| u64::from(entry.chunk_offset()))
                .collect()
        };
        let media_data_starting_with = |prefix: &[u8]| -> Vec<u64> {
            events_of(&file, file.len())
                .unwrap()
                .into_iter()
                .filter_map(|(extent, event)| {
                    if let BoxEvent::Payload(payload) = event {
                        payload.starts_with(prefix).then_some(extent.start)
                    } else {
                        None
                    }
                })
                .collect()
        };

        assert_eq!(
            [chunk_offsets(1), chunk_offsets(2)],
            [
                media_data_starting_with(b"VIDEO"),
                media_data_starting_with(b"AUD"),
            ]
        );
    }
}
