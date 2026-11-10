//! How long the samples of a sample table last: the duration of the media, and the end of their composition

#[cfg(test)]
mod tests {
    use isobmff::structure_data::{composition_end, media_duration};
    use isobmff_boxes::{
        CompositionOffsetBox, CompositionOffsetEntry, SampleTableBox, TimeToSampleBox,
        TimeToSampleEntry,
    };

    /// The sample table of the runs `(sample_count, sample_delta)`, whose composition offsets are the runs `(sample_count, sample_offset)` of `offsets`
    fn table(durations: &[(u32, u32)], offsets: Option<&[(u32, i64)]>) -> SampleTableBox {
        let mut stbl = SampleTableBox::default();
        let mut stts = TimeToSampleBox::default();
        stts.entries = durations
            .iter()
            .map(|&(sample_count, sample_delta)| {
                let mut run = TimeToSampleEntry::default();
                (run.sample_count, run.sample_delta) = (sample_count, sample_delta);
                run
            })
            .collect();
        stbl.stts = Some(stts);
        stbl.ctts = offsets.map(|offsets| {
            let mut ctts = CompositionOffsetBox::default();
            ctts.entries = offsets
                .iter()
                .map(|&(sample_count, sample_offset)| {
                    let mut run = CompositionOffsetEntry::default();
                    (run.sample_count, run.sample_offset) = (sample_count, sample_offset);
                    run
                })
                .collect();
            ctts
        });
        stbl
    }

    #[test]
    fn the_media_lasts_as_long_as_its_samples_laid_end_to_end() {
        assert_eq!(
            [
                media_duration(&table(&[(3, 1_000), (1, 500)], None)),
                media_duration(&table(&[], None)),
                media_duration(&SampleTableBox::default()),
                media_duration(&table(&[(u32::MAX, u32::MAX), (u32::MAX, u32::MAX)], None)),
            ],
            [3_500, 0, 0, u64::MAX]
        );
    }

    #[test]
    fn the_composition_ends_where_the_sample_composed_last_ends() {
        // Composed at 1, 3 and 2 until 2, 4 and 3: the composition ends at 4,
        // past the 3 their durations add up to.
        let reordered = table(&[(3, 1)], Some(&[(1, 1), (1, 2), (1, 0)]));
        // Composed at 0 and 1 until 1 and 2, then at 2 until 3: the runs of
        // durations and of offsets cut each other.
        let runs = table(&[(2, 1), (1, 1)], Some(&[(1, 0), (2, 0)]));
        assert_eq!(
            [
                composition_end(&reordered),
                composition_end(&runs),
                media_duration(&reordered),
            ],
            [4, 3, 3]
        );
    }

    #[test]
    fn without_composition_offsets_the_composition_ends_with_the_media() {
        let stbl = table(&[(3, 1_000), (1, 500)], None);
        assert_eq!(composition_end(&stbl), media_duration(&stbl));
    }

    #[test]
    fn a_composition_that_ends_before_0_ends_at_0_and_a_table_of_no_sample_too() {
        assert_eq!(
            [
                composition_end(&table(&[(2, 1)], Some(&[(2, -10)]))),
                composition_end(&SampleTableBox::default()),
            ],
            [0, 0]
        );
    }
}
