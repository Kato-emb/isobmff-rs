//! The samples a sample table is written from resolve back out of it as the extents they were laid down at

#[cfg(test)]
mod tests {
    use core::iter;

    use isobmff_boxes::{
        DegradationPriorityEntry, HeaderDuration, IsLeading, MovieBox, MovieHeaderBox,
        PaddingBitsEntry, SampleDependencyTypeEntry, SampleDependsOn, SampleDescriptionBox,
        SampleFlags, SampleHasRedundancy, SampleIsDependedOn,
    };
    use isobmff_core::{AnyBox, BoxType, Mp4EpochSeconds};
    use isobmff_sample::sample_table::sample_extents;
    use isobmff_sample::{Sample, SampleExtent, SampleProperties, SampleTableWriter, SampleTables};
    use isobmff_test_support::{
        movie_declaring, self_contained_data_reference, track, track_laid_out,
    };

    /// Sample of `track_id` at `decode_time` lasting `sample_duration` units, carrying `data`
    fn sample(track_id: u32, decode_time: u64, sample_duration: u32, data: &[u8]) -> Sample {
        stating(
            track_id,
            decode_time,
            sample_duration,
            0,
            SampleFlags::ZERO,
            data,
        )
    }

    /// Sample of `track_id` as [`sample`] makes it, stating `sample_composition_time_offset` and `sample_flags`
    fn stating(
        track_id: u32,
        decode_time: u64,
        sample_duration: u32,
        sample_composition_time_offset: i64,
        sample_flags: SampleFlags,
        data: &[u8],
    ) -> Sample {
        Sample::new(
            SampleProperties {
                track_id,
                decode_time,
                sample_duration,
                sample_composition_time_offset,
                sample_flags,
                sample_description_index: 1,
            },
            data.to_vec(),
        )
    }

    /// Lays `chunks` out, each `(chunk_offset, samples)`, and hands back the tables of each track in turn
    fn laid_out(chunks: &[(u64, Vec<Sample>)]) -> Vec<(u32, SampleTables)> {
        let mut writer = SampleTableWriter::new(&movie_declaring(vec![track(1), track(2)]));

        for (chunk_offset, samples) in chunks {
            writer.begin_chunk(*chunk_offset).unwrap();
            for sample in samples {
                writer.handle_sample(sample.clone()).unwrap();
            }
            writer.finish_chunk().unwrap();
        }
        writer.finish().unwrap();

        iter::from_fn(|| writer.poll_sample_tables()).collect()
    }

    /// Movie of one track per entry of `tables`, each laid out by its tables
    fn movie_of(tables: Vec<(u32, SampleTables)>) -> MovieBox {
        MovieBox::new(
            MovieHeaderBox::new(
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                1_000,
                HeaderDuration::ZERO,
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
        .unwrap()
    }

    /// Chunks of two tracks interleaved, the samples of track 1 composed apart from their decode times and flagged
    fn interleaved_chunks() -> Vec<(u64, Vec<Sample>)> {
        vec![
            (
                1_000,
                vec![
                    stating(
                        1,
                        0,
                        100,
                        200,
                        SampleFlags::new(
                            SampleDependencyTypeEntry::new(
                                IsLeading::NotLeading,
                                SampleDependsOn::DoesNotDependOnOthers,
                                SampleIsDependedOn::NotDisposable,
                                SampleHasRedundancy::NoRedundantCoding,
                            ),
                            PaddingBitsEntry::new(5).unwrap(),
                            false,
                            DegradationPriorityEntry::new(3),
                        ),
                        b"AAAAAAAA",
                    ),
                    stating(
                        1,
                        100,
                        100,
                        -100,
                        SampleFlags::new(
                            SampleDependencyTypeEntry::new(
                                IsLeading::Unknown,
                                SampleDependsOn::DependsOnOthers,
                                SampleIsDependedOn::Unknown,
                                SampleHasRedundancy::Unknown,
                            ),
                            PaddingBitsEntry::default(),
                            true,
                            DegradationPriorityEntry::default(),
                        ),
                        b"BBBB",
                    ),
                ],
            ),
            (2_000, vec![sample(2, 0, 1_000, b"CC")]),
            (
                3_000,
                vec![stating(
                    1,
                    200,
                    50,
                    0,
                    SampleFlags::new(
                        SampleDependencyTypeEntry::default(),
                        PaddingBitsEntry::default(),
                        true,
                        DegradationPriorityEntry::default(),
                    ),
                    b"DDDD",
                )],
            ),
            (
                4_000,
                vec![sample(2, 1_000, 1_000, b"EE"), sample(2, 2_000, 500, b"F")],
            ),
        ]
    }

    #[test]
    fn the_samples_of_two_tracks_interleaved_by_chunk_resolve_back_out_of_their_tables() {
        let chunks = interleaved_chunks();

        let mut laid_down = Vec::new();
        for (chunk_offset, samples) in &chunks {
            let mut offset = *chunk_offset;
            for sample in samples {
                let end = offset + sample.data().len() as u64;
                laid_down.push(SampleExtent::new(
                    SampleProperties {
                        sample_description_index: 1,
                        ..*sample.properties()
                    },
                    1,
                    offset..end,
                ));
                offset = end;
            }
        }
        let movie = movie_of(laid_out(&chunks));
        let resolved: Vec<SampleExtent> = sample_extents(&movie, u64::MAX)
            .map(Result::unwrap)
            .collect();

        assert_eq!(resolved, laid_down);
    }

    #[test]
    fn the_samples_resolved_out_of_the_tables_lay_them_out_again() {
        let chunks = interleaved_chunks();
        let tables = laid_out(&chunks);
        let mut resolved = sample_extents(&movie_of(tables.clone()), u64::MAX).map(Result::unwrap);

        let from_the_tables: Vec<(u64, Vec<Sample>)> = chunks
            .iter()
            .map(|(chunk_offset, samples)| {
                let samples = samples
                    .iter()
                    .map(|sample| {
                        let extent = resolved.next().unwrap();
                        Sample::new(*extent.properties(), sample.data().to_vec())
                    })
                    .collect();
                (*chunk_offset, samples)
            })
            .collect();

        assert_eq!(laid_out(&from_the_tables), tables);
    }
}
