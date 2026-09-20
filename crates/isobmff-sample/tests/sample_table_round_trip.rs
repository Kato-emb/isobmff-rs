//! The samples a sample table is written from resolve back out of it as the extents they were laid down at

#[cfg(test)]
mod tests {
    use isobmff_boxes::{MovieBox, MovieHeaderBox, SampleDescriptionBox};
    use isobmff_core::{AnyBox, BoxType, Mp4EpochSeconds};
    use isobmff_sample::sample_table::sample_extents;
    use isobmff_sample::{Sample, SampleExtent, SampleTableWriter};
    use isobmff_test_support::{self_contained_data_reference, track_laid_out};

    /// Sample of `track_id` at `decode_time` lasting `sample_duration` units, carrying `data`
    fn sample(track_id: u32, decode_time: u64, sample_duration: u32, data: &[u8]) -> Sample {
        Sample::new(
            track_id,
            decode_time,
            sample_duration,
            0,
            0,
            1,
            data.to_vec(),
        )
    }

    #[test]
    fn the_samples_of_two_tracks_interleaved_by_chunk_resolve_back_out_of_their_tables() {
        let chunks = [
            (
                1_000,
                vec![sample(1, 0, 100, b"AAAAAAAA"), sample(1, 100, 100, b"BBBB")],
            ),
            (2_000, vec![sample(2, 0, 1_000, b"CC")]),
            (3_000, vec![sample(1, 200, 50, b"DDDD")]),
            (
                4_000,
                vec![sample(2, 1_000, 1_000, b"EE"), sample(2, 2_000, 500, b"F")],
            ),
        ];
        let mut writer = SampleTableWriter::new();

        let mut laid_down = Vec::new();
        for (chunk_offset, samples) in &chunks {
            writer.begin_chunk(*chunk_offset).unwrap();
            let mut offset = *chunk_offset;
            for sample in samples {
                let data = writer.handle_sample(sample.clone()).unwrap();
                let end = offset + data.len() as u64;
                laid_down.push(SampleExtent::new(
                    sample.track_id(),
                    sample.decode_time(),
                    sample.sample_duration(),
                    0,
                    0,
                    1,
                    1,
                    offset..end,
                ));
                offset = end;
            }
        }
        let tables = writer.finish().unwrap();

        let movie = MovieBox::new(
            MovieHeaderBox::new(
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                1_000,
                0,
                3,
            ),
            tables
                .into_iter()
                .map(|(track_id, track)| {
                    let entry = AnyBox::from_raw_bytes(
                        BoxType::compact(*b"avc1"),
                        vec![0, 0, 0, 0, 0, 0, 0, 1],
                    );
                    track_laid_out(
                        track_id,
                        self_contained_data_reference(),
                        track.into_sample_table(SampleDescriptionBox::new(vec![entry])),
                    )
                })
                .collect(),
            None,
        )
        .unwrap();
        let resolved: Vec<SampleExtent> = sample_extents(&movie).map(Result::unwrap).collect();

        assert_eq!(resolved, laid_down);
    }
}
