//! [`sample_extents`], the samples the sample tables of a movie declare resolved to where they lie, ISO/IEC 14496-12 §8.5.1 and §8.7

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::iter;

use isobmff_boxes::{MovieBox, SampleSizeBox, SampleSizeEntry, SampleSizes, TrackBox};
use isobmff_core::BoxDefinition as _;

use crate::error::SampleError;
use crate::sample::SampleExtent;
use crate::sample_description::SampleDescriptions;

/// Resolves the samples the sample tables of `movie` declare, in the order their bytes lie in the file
///
/// The `stbl` of each track (ISO/IEC 14496-12 §8.5.1) spreads what it declares
/// about a sample over four tables, and a sample is read across them: the
/// `stts` states when it is decoded, as a delta from the sample before it
/// summed from zero (§8.6.1.2); the `stsc` states which chunk it lies in and
/// which `stsd` entry describes it, by runs of chunks holding the same number
/// of samples (§8.7.4); the `stsz` states how many bytes it occupies
/// (§8.7.3.2); and the `stco` states where its chunk starts in the file, the
/// samples of a chunk lying one after another from there (§8.7.5). The
/// `data_reference_index` of each sample is read off the `stsd` entry that
/// describes it (§8.5.2.3), which has to name the file itself.
///
/// A sample comes out a sync sample with a composition time offset of zero,
/// which is what §8.6.2 and §8.6.1.3 have for a track stating no `stss` and
/// no `ctts`: neither table is read yet, so a track stating either comes out
/// as though it did not. A track declaring no sample — one carried in
/// fragments — contributes nothing, and a chunk its `stsc` lays no run over
/// holds none.
///
/// The extents of every track come out together in the order their bytes lie
/// in the file, the samples of one chunk in sample order and the chunks of
/// one track between those of another where the file interleaves them, which
/// is the order a reader fed the file from its start meets them in. They stop
/// at the first failure, which is the last item. They borrow nothing: the
/// movie is the caller's again once the call returns.
///
/// # Errors
///
/// Returned as the last of the extents:
///
/// * [`SampleCountMismatch`](crate::SampleErrorKind::SampleCountMismatch):
///   the tables of a track count different numbers of samples.
/// * [`FirstChunkOutOfRange`](crate::SampleErrorKind::FirstChunkOutOfRange):
///   a run of chunks of a track starts at a chunk outside the range open to it.
/// * [`UnknownSampleDescriptionIndex`](crate::SampleErrorKind::UnknownSampleDescriptionIndex):
///   a run describes its samples by an `stsd` entry its track has none of.
/// * The failures of [`SampleEntry::try_from`](isobmff_boxes::SampleEntry),
///   carried on [`Box`](crate::SampleErrorKind::Box): the `stsd` entry does
///   not read as a sample entry, with `stsd` added to the containers.
/// * [`UnknownDataReferenceIndex`](crate::SampleErrorKind::UnknownDataReferenceIndex):
///   the `stsd` entry names a `dref` entry its track has none of.
/// * [`ExternalDataReference`](crate::SampleErrorKind::ExternalDataReference):
///   the `dref` entry names a resource other than the file itself.
/// * [`UnsupportedBox`](isobmff_core::ErrorKind::UnsupportedBox), carried on
///   [`Box`](crate::SampleErrorKind::Box): the `stsz` states its sizes a way
///   added to [`SampleSizes`] after this resolver, which it does not read.
/// * [`DecodeTimeOverflow`](crate::SampleErrorKind::DecodeTimeOverflow): the
///   decode times of a track run past what 64 bits carry.
/// * [`DataOffsetOverflow`](crate::SampleErrorKind::DataOffsetOverflow): the
///   offsets of a track run past what 64 bits carry.
pub fn sample_extents(
    movie: &MovieBox,
) -> impl Iterator<Item = Result<SampleExtent, SampleError>> + use<> {
    let mut extents = Vec::new();
    let outcome = movie
        .trak()
        .iter()
        .try_for_each(|trak| resolve_track(trak, &mut extents));
    extents.sort_by_key(|extent| extent.extent().start);

    extents.into_iter().map(Ok).chain(outcome.err().map(Err))
}

/// Resolves the samples the sample table of `trak` declares into `extents`, in sample order
fn resolve_track(trak: &TrackBox, extents: &mut Vec<SampleExtent>) -> Result<(), SampleError> {
    let track_id = trak.tkhd().track_id();
    let stbl = trak.mdia().minf().stbl();
    let descriptions = SampleDescriptions::new(trak);
    let mut sizes: Box<dyn Iterator<Item = u32> + '_> = match stbl.stsz().sample_sizes() {
        SampleSizes::Uniform {
            sample_size,
            sample_count,
        } => Box::new(iter::repeat_n(
            sample_size.get(),
            usize::try_from(*sample_count).unwrap_or(usize::MAX),
        )),
        SampleSizes::PerSample(entries) => {
            Box::new(entries.iter().map(SampleSizeEntry::entry_size))
        }
        // Why not unreachable!: the enum is non-exhaustive, so a way of stating
        // the sizes added later lands here, and a failure the caller reads
        // beats a panic.
        _ => {
            return Err(SampleError::from(isobmff_core::Error::unsupported_box(
                SampleSizeBox::BOX_TYPE,
            )));
        }
    };
    let mut deltas = stbl.stts().entries().iter().flat_map(|entry| {
        iter::repeat_n(
            entry.sample_delta(),
            usize::try_from(entry.sample_count()).unwrap_or(usize::MAX),
        )
    });
    let mut runs = stbl.stsc().entries().iter().peekable();
    if let Some(first) = runs.peek().filter(|run| run.first_chunk() != 1) {
        return Err(SampleError::first_chunk_out_of_range(
            track_id,
            first.first_chunk(),
        ));
    }
    let mut active_run = None;
    let mut decode_time = 0_u64;

    for (chunk, offset) in (1_u64..).zip(stbl.stco().entries()) {
        if let Some(run) = runs.next_if(|run| u64::from(run.first_chunk()) == chunk) {
            let data_reference_index =
                descriptions.data_reference_index(run.sample_description_index())?;
            active_run = Some((run, data_reference_index));
        }
        let Some((run, data_reference_index)) = active_run else {
            continue;
        };
        let mut data_offset = u64::from(offset.chunk_offset());

        for _ in 0..run.samples_per_chunk() {
            let (Some(size), Some(delta)) = (sizes.next(), deltas.next()) else {
                return Err(SampleError::sample_count_mismatch(track_id));
            };
            let data_end = data_offset
                .checked_add(u64::from(size))
                .ok_or(SampleError::data_offset_overflow(track_id))?;

            extents.push(SampleExtent::new(
                track_id,
                decode_time,
                delta,
                0,
                0,
                run.sample_description_index(),
                data_reference_index,
                data_offset..data_end,
            ));

            decode_time = decode_time
                .checked_add(u64::from(delta))
                .ok_or(SampleError::decode_time_overflow(track_id))?;
            data_offset = data_end;
        }
    }
    if let Some(run) = runs.next() {
        return Err(SampleError::first_chunk_out_of_range(
            track_id,
            run.first_chunk(),
        ));
    }
    if sizes.next().is_some() || deltas.next().is_some() {
        return Err(SampleError::sample_count_mismatch(track_id));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;
    use core::ops::Range;

    use isobmff_boxes::{
        ChunkOffsetBox, ChunkOffsetEntry, MovieBox, MovieHeaderBox, SampleDescriptionBox,
        SampleSizeBox, SampleSizeEntry, SampleSizes, SampleTableBox, SampleToChunkBox,
        SampleToChunkEntry, TimeToSampleBox, TimeToSampleEntry, TrackBox,
    };
    use isobmff_core::{AnyBox, BoxType, Mp4EpochSeconds};
    use isobmff_test_support::{
        external_data_reference, sample_table, self_contained_data_reference, track_laid_out,
        unfragmented_movie,
    };

    use super::sample_extents;
    use crate::error::SampleError;
    use crate::sample::SampleExtent;

    /// Movie of the given tracks, continued in no fragment
    fn movie(trak: Vec<TrackBox>) -> MovieBox {
        MovieBox::new(
            MovieHeaderBox::new(
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                1_000,
                0,
                2,
            ),
            trak,
            None,
        )
        .unwrap()
    }

    /// Decode timeline of runs of samples, each `(sample_count, sample_delta)`
    fn stts(entries: &[(u32, u32)]) -> TimeToSampleBox {
        TimeToSampleBox::new(
            entries
                .iter()
                .map(|&(sample_count, sample_delta)| {
                    TimeToSampleEntry::new(sample_count, sample_delta)
                })
                .collect(),
        )
    }

    /// Runs of chunks, each `(first_chunk, samples_per_chunk)`, described by the one entry
    fn stsc(entries: &[(u32, u32)]) -> SampleToChunkBox {
        SampleToChunkBox::new(
            entries
                .iter()
                .map(|&(first_chunk, samples_per_chunk)| {
                    SampleToChunkEntry::new(first_chunk, samples_per_chunk, 1)
                })
                .collect(),
        )
    }

    /// Sizes stated one per sample
    fn stsz(sizes: &[u32]) -> SampleSizeBox {
        SampleSizeBox::new(SampleSizes::PerSample(
            sizes.iter().copied().map(SampleSizeEntry::new).collect(),
        ))
    }

    /// Chunks starting at the offsets given
    fn stco(offsets: &[u32]) -> ChunkOffsetBox {
        ChunkOffsetBox::new(offsets.iter().copied().map(ChunkOffsetEntry::new).collect())
    }

    /// Track `track_id` of the file itself, its samples laid out by the four tables
    fn track_of(
        track_id: u32,
        stts: TimeToSampleBox,
        stsc: SampleToChunkBox,
        stsz: SampleSizeBox,
        stco: ChunkOffsetBox,
    ) -> TrackBox {
        track_laid_out(
            track_id,
            self_contained_data_reference(),
            sample_table(stts, stsc, stsz, stco),
        )
    }

    /// Track 1 of four-byte samples lasting 100 units, one to a chunk, at the offsets given
    fn track_chunked_at(offsets: &[u32]) -> TrackBox {
        track_of(
            1,
            stts(&[(u32::try_from(offsets.len()).unwrap(), 100)]),
            stsc(&[(1, 1)]),
            stsz(&vec![4; offsets.len()]),
            stco(offsets),
        )
    }

    /// Extent of a sync sample of `track_id` described by entry 1, in the file itself
    fn extent(
        track_id: u32,
        decode_time: u64,
        sample_duration: u32,
        data: Range<u64>,
    ) -> SampleExtent {
        SampleExtent::new(track_id, decode_time, sample_duration, 0, 0, 1, 1, data)
    }

    /// Resolves the samples of `movie`, whole
    fn resolved(movie: &MovieBox) -> Result<Vec<SampleExtent>, SampleError> {
        sample_extents(movie).collect()
    }

    #[test]
    fn a_sample_is_read_across_the_four_tables_of_its_track() {
        let trak = track_of(
            1,
            stts(&[(2, 100), (3, 50)]),
            stsc(&[(1, 2), (2, 3)]),
            stsz(&[4, 6, 2, 8, 3]),
            stco(&[1_000, 2_000]),
        );

        assert_eq!(
            resolved(&movie(vec![trak])),
            Ok(vec![
                extent(1, 0, 100, 1_000..1_004),
                extent(1, 100, 100, 1_004..1_010),
                extent(1, 200, 50, 2_000..2_002),
                extent(1, 250, 50, 2_002..2_010),
                extent(1, 300, 50, 2_010..2_013),
            ])
        );
    }

    #[test]
    fn a_run_of_chunks_reaches_the_chunk_the_next_run_starts_at() {
        let trak = track_of(
            1,
            stts(&[(6, 100)]),
            stsc(&[(1, 1), (3, 2)]),
            stsz(&[4; 6]),
            stco(&[100, 200, 300, 400]),
        );

        assert_eq!(
            resolved(&movie(vec![trak])),
            Ok(vec![
                extent(1, 0, 100, 100..104),
                extent(1, 100, 100, 200..204),
                extent(1, 200, 100, 300..304),
                extent(1, 300, 100, 304..308),
                extent(1, 400, 100, 400..404),
                extent(1, 500, 100, 404..408),
            ])
        );
    }

    #[test]
    fn a_size_every_sample_shares_is_read_for_each_of_them() {
        let trak = track_of(
            1,
            stts(&[(3, 100)]),
            stsc(&[(1, 3)]),
            SampleSizeBox::new(SampleSizes::Uniform {
                sample_size: NonZeroU32::new(4).unwrap(),
                sample_count: 3,
            }),
            stco(&[100]),
        );

        assert_eq!(
            resolved(&movie(vec![trak])),
            Ok(vec![
                extent(1, 0, 100, 100..104),
                extent(1, 100, 100, 104..108),
                extent(1, 200, 100, 108..112),
            ])
        );
    }

    #[test]
    fn the_samples_of_a_run_take_the_description_it_names() {
        let entry =
            || AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0, 0, 0, 0, 0, 0, 0, 1]);
        let trak = track_laid_out(
            1,
            self_contained_data_reference(),
            SampleTableBox::new(
                SampleDescriptionBox::new(vec![entry(), entry()]),
                stts(&[(2, 100)]),
                SampleToChunkBox::new(vec![
                    SampleToChunkEntry::new(1, 1, 2),
                    SampleToChunkEntry::new(2, 1, 1),
                ]),
                stsz(&[4, 4]),
                stco(&[100, 200]),
            ),
        );

        assert_eq!(
            resolved(&movie(vec![trak])),
            Ok(vec![
                SampleExtent::new(1, 0, 100, 0, 0, 2, 1, 100..104),
                extent(1, 100, 100, 200..204),
            ])
        );
    }

    #[test]
    fn the_chunks_of_two_tracks_come_out_in_the_order_the_file_lays_them_down() {
        let interleaved = movie(vec![
            track_chunked_at(&[100, 300]),
            track_of(
                2,
                stts(&[(4, 1_000)]),
                stsc(&[(1, 2)]),
                stsz(&[8; 4]),
                stco(&[200, 400]),
            ),
        ]);

        assert_eq!(
            resolved(&interleaved),
            Ok(vec![
                extent(1, 0, 100, 100..104),
                extent(2, 0, 1_000, 200..208),
                extent(2, 1_000, 1_000, 208..216),
                extent(1, 100, 100, 300..304),
                extent(2, 2_000, 1_000, 400..408),
                extent(2, 3_000, 1_000, 408..416),
            ])
        );
    }

    #[test]
    fn a_movie_declaring_no_sample_in_its_tables_resolves_to_none() {
        assert_eq!(resolved(&unfragmented_movie()), Ok(vec![]));
    }

    #[test]
    fn tables_counting_different_numbers_of_samples_are_refused() {
        let more_deltas_than_sizes = track_of(
            1,
            stts(&[(4, 100)]),
            stsc(&[(1, 3)]),
            stsz(&[4; 3]),
            stco(&[100]),
        );
        let more_sizes_than_the_chunks_hold = track_of(
            1,
            stts(&[(4, 100)]),
            stsc(&[(1, 3)]),
            stsz(&[4; 4]),
            stco(&[100]),
        );
        let fewer_sizes_than_the_chunks_hold = track_of(
            1,
            stts(&[(2, 100)]),
            stsc(&[(1, 3)]),
            stsz(&[4; 2]),
            stco(&[100]),
        );

        for trak in [
            more_deltas_than_sizes,
            more_sizes_than_the_chunks_hold,
            fewer_sizes_than_the_chunks_hold,
        ] {
            assert_eq!(
                resolved(&movie(vec![trak])),
                Err(SampleError::sample_count_mismatch(1))
            );
        }
    }

    #[test]
    fn a_run_starting_past_the_last_chunk_is_refused() {
        let past_the_last_chunk = track_of(
            1,
            stts(&[(2, 100)]),
            stsc(&[(1, 2), (3, 1)]),
            stsz(&[4; 2]),
            stco(&[100]),
        );

        assert_eq!(
            resolved(&movie(vec![past_the_last_chunk])),
            Err(SampleError::first_chunk_out_of_range(1, 3))
        );
    }

    #[test]
    fn a_run_starting_at_or_before_the_start_of_the_run_before_it_is_refused() {
        let doubling_back = track_of(
            1,
            stts(&[(3, 100)]),
            stsc(&[(1, 1), (3, 1), (2, 1)]),
            stsz(&[4; 3]),
            stco(&[100, 200, 300]),
        );
        let starting_twice = track_of(
            1,
            stts(&[(3, 100)]),
            stsc(&[(1, 1), (3, 1), (3, 1)]),
            stsz(&[4; 3]),
            stco(&[100, 200, 300]),
        );

        assert_eq!(
            resolved(&movie(vec![doubling_back])),
            Err(SampleError::first_chunk_out_of_range(1, 2))
        );
        assert_eq!(
            resolved(&movie(vec![starting_twice])),
            Err(SampleError::first_chunk_out_of_range(1, 3))
        );
    }

    #[test]
    fn a_first_run_starting_anywhere_but_at_the_first_chunk_is_refused() {
        let starting_at_the_second_chunk = track_of(
            1,
            stts(&[(1, 100)]),
            stsc(&[(2, 1)]),
            stsz(&[4]),
            stco(&[100, 200]),
        );

        assert_eq!(
            resolved(&movie(vec![starting_at_the_second_chunk])),
            Err(SampleError::first_chunk_out_of_range(1, 2))
        );
    }

    #[test]
    fn chunks_no_run_lays_over_hold_no_sample() {
        let chunks_without_runs = track_of(1, stts(&[]), stsc(&[]), stsz(&[]), stco(&[100, 200]));

        assert_eq!(resolved(&movie(vec![chunks_without_runs])), Ok(vec![]));
    }

    #[test]
    fn a_run_described_by_an_entry_its_track_has_none_of_is_refused() {
        let trak = track_of(
            1,
            stts(&[(1, 100)]),
            SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 1, 2)]),
            stsz(&[4]),
            stco(&[100]),
        );

        assert_eq!(
            resolved(&movie(vec![trak])),
            Err(SampleError::unknown_sample_description_index(1, 2))
        );
    }

    #[test]
    fn a_sample_of_a_track_reading_from_an_external_file_is_refused() {
        let trak = track_laid_out(
            1,
            external_data_reference(),
            sample_table(stts(&[(1, 100)]), stsc(&[(1, 1)]), stsz(&[4]), stco(&[100])),
        );

        assert_eq!(
            resolved(&movie(vec![trak])),
            Err(SampleError::external_data_reference(1, 1))
        );
    }

    #[test]
    fn the_extents_resolved_before_a_failure_come_out_ahead_of_it() {
        let then_a_track_out_of_range = movie(vec![
            track_chunked_at(&[100, 200]),
            track_of(2, stts(&[]), stsc(&[(2, 1)]), stsz(&[]), stco(&[])),
        ]);

        assert_eq!(
            sample_extents(&then_a_track_out_of_range).collect::<Vec<_>>(),
            [
                Ok(extent(1, 0, 100, 100..104)),
                Ok(extent(1, 100, 100, 200..204)),
                Err(SampleError::first_chunk_out_of_range(2, 2)),
            ]
        );
    }
}
