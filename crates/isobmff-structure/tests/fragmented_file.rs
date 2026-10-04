//! The samples of a fragmented file laid out by hand, read back through its structure

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/reading.rs`. The
// `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use isobmff_boxes::{
        ChunkOffsets, MediaDataBox, MovieBox, MovieFragmentBox, MovieFragmentHeaderBox,
        MovieFragmentRandomAccessOffsetBox, SampleFlags, SampleSizeBox, SampleSizes,
        SampleToChunkBox, TimeToSampleBox, TrackBox, TrackFragmentBaseMediaDecodeTimeBox,
        TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentHeaderFlags, TrackRunBox,
        TrackRunSample,
    };
    use isobmff_core::{BoxEncode, BoxType};
    use isobmff_sample::Sample;
    use isobmff_sequence::BoxEvent;
    use isobmff_structure::{Error, ErrorKind, FragmentedDemuxFsm, WantedInput};
    use isobmff_test_support::{
        HybridFile, IndexedFile, SAMPLE_CHUNKS, events_of, file_type, fragmented_file_samples,
        fragmented_file_with_samples, hybrid_file, indexed_fragmented_file,
        indexed_fragmented_file_without_decode_times, non_fragmented_file,
        non_fragmented_file_samples, sample_table, self_contained_data_reference, track_laid_out,
        written,
    };

    use super::reading::{drained, read_on, samples_of};

    /// Reader that read `file` whole and was declared over
    fn read_whole(file: &IndexedFile) -> FragmentedDemuxFsm {
        let mut demux_fsm = FragmentedDemuxFsm::new();
        demux_fsm.handle_input(0, &file.bytes).unwrap();
        demux_fsm.finish().unwrap();

        demux_fsm
    }

    /// Resumes `demux_fsm` at `offset` and hands it the rest of `file` from there
    fn resumed_at(
        demux_fsm: &mut FragmentedDemuxFsm,
        file: &IndexedFile,
        offset: u64,
    ) -> Result<(), Error> {
        demux_fsm.resume_at(offset)?;
        demux_fsm.handle_input(
            offset,
            file.bytes.get(usize::try_from(offset).unwrap()..).unwrap(),
        )?;
        demux_fsm.finish()
    }

    #[test]
    fn the_samples_of_a_fragmented_file_are_read_off_the_bytes_it_lies_as() {
        let file = fragmented_file_with_samples();

        assert_eq!(samples_of(&file, file.len()), fragmented_file_samples());
    }

    #[test]
    fn the_samples_are_the_same_however_the_file_was_cut() {
        let file = fragmented_file_with_samples();

        for cut_length in [1, 3, 7, 64, file.len().saturating_sub(1)] {
            assert_eq!(samples_of(&file, cut_length), fragmented_file_samples());
        }
    }

    #[test]
    fn a_file_read_in_order_yields_every_sample_and_both_indexes_pointing_at_its_fragments() {
        let file = indexed_fragmented_file();

        let mut demux_fsm = read_whole(&file);

        assert_eq!(drained(&mut demux_fsm), file.fragment_samples.concat());
        assert_eq!(
            demux_fsm
                .segment_indexes()
                .iter()
                .map(|segment_index| segment_index
                    .subsegments()
                    .iter()
                    .map(|subsegment| subsegment.extent().start)
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [file.moof_offsets.as_slice()]
        );
        assert_eq!(
            demux_fsm
                .movie_fragment_random_access()
                .unwrap()
                .tfra()
                .iter()
                .flat_map(|tfra| tfra.entries())
                .map(|entry| entry.moof_offset())
                .collect::<Vec<_>>(),
            file.moof_offsets
        );
    }

    #[test]
    fn resuming_at_the_second_fragment_yields_its_samples_alone() {
        let file = indexed_fragmented_file();
        let second = *file.moof_offsets.get(1).unwrap();

        let mut finished = read_whole(&file);
        drained(&mut finished);
        resumed_at(&mut finished, &file, second).unwrap();

        let mut reading = FragmentedDemuxFsm::new();
        reading
            .handle_input(
                0,
                file.bytes.get(..usize::try_from(second).unwrap()).unwrap(),
            )
            .unwrap();
        drained(&mut reading);
        resumed_at(&mut reading, &file, second).unwrap();

        let second_samples = file.fragment_samples.get(1).unwrap();
        assert_eq!(&drained(&mut finished), second_samples);
        assert_eq!(&drained(&mut reading), second_samples);
    }

    #[test]
    fn an_index_read_again_on_resuming_at_it_is_held_once() {
        let file = indexed_fragmented_file();
        let segment_index = events_of(&file.bytes, file.bytes.len())
            .unwrap()
            .into_iter()
            .find_map(|(extent, event)| {
                if let BoxEvent::Header(header) = event {
                    (header.box_type() == BoxType::compact(*b"sidx")).then_some(extent.start)
                } else {
                    None
                }
            })
            .unwrap();

        let mut demux_fsm = read_whole(&file);
        drained(&mut demux_fsm);
        resumed_at(&mut demux_fsm, &file, segment_index).unwrap();

        assert_eq!(demux_fsm.segment_indexes().len(), 1);
        assert_eq!(drained(&mut demux_fsm), file.fragment_samples.concat());
    }

    #[test]
    fn resuming_at_media_data_is_out_of_order_and_leaves_the_demux_fsm_failed() {
        let file = indexed_fragmented_file();
        let second = *file.moof_offsets.get(1).unwrap();
        let moof_size = file
            .bytes
            .get(usize::try_from(second).unwrap()..)
            .and_then(|moof| moof.first_chunk::<4>())
            .unwrap();
        let media_data = second.saturating_add(u64::from(u32::from_be_bytes(*moof_size)));
        let out_of_order = Error::box_out_of_order(BoxType::compact(*b"mdat"));

        let mut demux_fsm = read_whole(&file);

        assert_eq!(
            resumed_at(&mut demux_fsm, &file, media_data),
            Err(out_of_order)
        );
        assert_eq!(demux_fsm.resume_at(second), Err(out_of_order));
    }

    #[test]
    fn a_file_stating_no_decode_time_read_in_order_starts_its_timeline_at_zero() {
        let file = indexed_fragmented_file_without_decode_times();

        let mut demux_fsm = read_whole(&file);

        assert_eq!(drained(&mut demux_fsm), file.fragment_samples.concat());
    }

    #[test]
    fn resuming_at_a_fragment_stating_no_decode_time_fails_for_the_missing_decode_time() {
        let file = indexed_fragmented_file_without_decode_times();

        let mut demux_fsm = read_whole(&file);

        assert_eq!(
            resumed_at(&mut demux_fsm, &file, *file.moof_offsets.get(1).unwrap())
                .map_err(Error::kind),
            Err(ErrorKind::Sample(
                isobmff_sample::ErrorKind::MissingDecodeTime
            ))
        );
    }

    #[test]
    fn the_continuation_is_wanted_after_the_bytes_handed_over_since_the_reading_last_started() {
        let file = indexed_fragmented_file();
        let second = *file.moof_offsets.get(1).unwrap();
        let mut demux_fsm = FragmentedDemuxFsm::new();

        let created = demux_fsm.wanted_input();
        demux_fsm.handle_input(0, &file.bytes).unwrap();
        let handed = demux_fsm.wanted_input();
        demux_fsm.resume_at(second).unwrap();
        let resumed = demux_fsm.wanted_input();
        demux_fsm
            .handle_input(
                second,
                file.bytes.get(usize::try_from(second).unwrap()..).unwrap(),
            )
            .unwrap();

        let file_length = u64::try_from(file.bytes.len()).unwrap();
        assert_eq!(
            [created, handed, resumed, demux_fsm.wanted_input()],
            [
                Some(WantedInput::new(0, None)),
                Some(WantedInput::new(file_length, None)),
                Some(WantedInput::new(second, None)),
                Some(WantedInput::new(file_length, None))
            ]
        );
    }

    /// Demux FSM that read `bytes` up to `first`, the offset of the first fragment, then was told the file is `file_len` long to find its `mfra`
    fn locating_after_the_movie(bytes: &[u8], first: u64, file_len: u64) -> FragmentedDemuxFsm {
        let mut demux_fsm = FragmentedDemuxFsm::new();
        demux_fsm
            .handle_input(0, bytes.get(..usize::try_from(first).unwrap()).unwrap())
            .unwrap();
        demux_fsm
            .resume_at_movie_fragment_random_access(file_len)
            .unwrap();

        demux_fsm
    }

    #[test]
    fn the_mfra_closing_the_file_is_read_however_its_last_bytes_are_cut() {
        let file = indexed_fragmented_file();
        let first = *file.moof_offsets.first().unwrap();
        let file_len = u64::try_from(file.bytes.len()).unwrap();
        let read_in_order = read_whole(&file).movie_fragment_random_access().cloned();

        for cut_length in [1, 7, file.bytes.len()] {
            let mut demux_fsm = locating_after_the_movie(&file.bytes, first, file_len);

            assert_eq!(
                (
                    read_on(&mut demux_fsm, &file.bytes, cut_length),
                    demux_fsm.movie_fragment_random_access().cloned()
                ),
                (Vec::new(), read_in_order.clone())
            );
        }
    }

    #[test]
    fn a_file_too_short_for_an_mfro_closing_with_none_or_with_one_stepping_back_past_its_start_is_declared_over_and_read_again_from_a_fragment()
     {
        let file = indexed_fragmented_file();
        let first = *file.moof_offsets.first().unwrap();
        let mfra_start = file
            .bytes
            .last_chunk::<4>()
            .map(|size| usize::try_from(u32::from_be_bytes(*size)).unwrap())
            .and_then(|size| file.bytes.len().checked_sub(size))
            .unwrap();
        let no_mfra = file.bytes.get(..mfra_start).unwrap();
        let mut past_the_start = file.bytes.clone();
        *past_the_start.last_chunk_mut::<4>().unwrap() = u32::MAX.to_be_bytes();

        let read = [
            (
                file.bytes.as_slice(),
                MovieFragmentRandomAccessOffsetBox::ENCODED_LEN.saturating_sub(1),
            ),
            (no_mfra, no_mfra.len()),
            (past_the_start.as_slice(), past_the_start.len()),
        ]
        .map(|(bytes, file_len)| {
            let mut demux_fsm =
                locating_after_the_movie(bytes, first, u64::try_from(file_len).unwrap());
            let located = read_on(&mut demux_fsm, bytes, 7);
            let declared_over = (
                demux_fsm.wanted_input(),
                demux_fsm.movie_fragment_random_access().cloned(),
            );
            demux_fsm.resume_at(first).unwrap();

            (located, declared_over, read_on(&mut demux_fsm, bytes, 7))
        });

        let read_again = (Vec::new(), (None, None), file.fragment_samples.concat());
        assert_eq!(vec![read_again; 3], read);
    }

    #[test]
    fn input_elsewhere_than_the_closing_bytes_is_refused_and_finishing_then_declares_the_file_over()
    {
        let file = indexed_fragmented_file();
        let file_len = u64::try_from(file.bytes.len()).unwrap();
        let closing_len = u64::try_from(MovieFragmentRandomAccessOffsetBox::ENCODED_LEN).unwrap();
        let mut demux_fsm = FragmentedDemuxFsm::new();
        demux_fsm
            .resume_at_movie_fragment_random_access(file_len)
            .unwrap();

        let refused = demux_fsm.handle_input(0, &file.bytes);
        let wanted = demux_fsm.wanted_input();
        let finished = demux_fsm.finish();

        assert_eq!(
            (refused, wanted, finished, demux_fsm.wanted_input()),
            (
                Err(Error::unwanted_input(0)),
                Some(WantedInput::new(
                    file_len.saturating_sub(closing_len),
                    Some(closing_len)
                )),
                Ok(()),
                None
            )
        );
    }

    /// The samples `file` was built to carry, those of its movie first
    fn every_sample_of(file: &HybridFile) -> Vec<Sample> {
        [file.movie_samples.clone(), file.fragment_samples.clone()].concat()
    }

    /// `samples` ordered by track, then by decode time
    fn by_track(mut samples: Vec<Sample>) -> Vec<Sample> {
        // Why not comparing the order they come out in: the samples of the
        // movie and those of the fragment are held apart, and which complete
        // first is decided by where the input is cut and which bytes are
        // wanted back.
        samples.sort_by_key(|sample| (sample.track_id(), sample.decode_time()));

        samples
    }

    #[test]
    fn the_samples_a_movie_declares_beside_its_fragments_are_read_with_theirs() {
        for movie_first in [true, false] {
            let file = hybrid_file(movie_first, true);

            for cut_length in [7, file.bytes.len()] {
                assert_eq!(
                    by_track(samples_of(&file.bytes, cut_length)),
                    every_sample_of(&file),
                    "movie first: {movie_first}, cut length: {cut_length}"
                );
            }
        }
    }

    #[test]
    fn a_fragment_stating_no_decode_time_starts_where_the_sample_table_of_the_movie_leaves_its_track()
     {
        for movie_first in [true, false] {
            let file = hybrid_file(movie_first, false);

            assert_eq!(
                by_track(samples_of(&file.bytes, 7)),
                every_sample_of(&file),
                "movie first: {movie_first}"
            );
        }
    }

    #[test]
    fn media_data_lying_before_the_movie_is_wanted_once_the_movie_is_read() {
        let file = hybrid_file(false, true);
        let first_sample = file
            .bytes
            .windows(8)
            .position(|bytes| bytes == b"SAMPLE_1")
            .unwrap();
        let mut demux_fsm = FragmentedDemuxFsm::new();

        demux_fsm
            .handle_input(
                0,
                file.bytes
                    .get(..usize::try_from(file.moof_offset).unwrap())
                    .unwrap(),
            )
            .unwrap();

        assert_eq!(
            demux_fsm.wanted_input(),
            Some(WantedInput::new(
                u64::try_from(first_sample).unwrap(),
                Some(8)
            ))
        );
        assert_eq!(
            [
                drained(&mut demux_fsm),
                read_on(&mut demux_fsm, &file.bytes, 7)
            ]
            .concat(),
            every_sample_of(&file)
        );
    }

    #[test]
    fn a_file_of_no_fragment_whose_movie_carries_no_mvex_yields_every_sample_its_movie_declares() {
        for movie_first in [true, false] {
            assert_eq!(
                samples_of(&non_fragmented_file(&SAMPLE_CHUNKS, movie_first), 7),
                non_fragmented_file_samples(),
                "movie first: {movie_first}"
            );
        }
    }

    #[test]
    fn resuming_drops_the_samples_of_the_movie_and_a_fragment_stating_no_decode_time_then_fails() {
        let read_resuming_at_the_fragment = |file: &HybridFile| {
            let fragment = usize::try_from(file.moof_offset).unwrap();
            let mut demux_fsm = FragmentedDemuxFsm::new();
            demux_fsm
                .handle_input(0, file.bytes.get(..fragment).unwrap())
                .unwrap();

            demux_fsm.resume_at(file.moof_offset).unwrap();
            let resumed = demux_fsm
                .handle_input(file.moof_offset, file.bytes.get(fragment..).unwrap())
                .and_then(|()| demux_fsm.finish())
                .map_err(Error::kind);

            (resumed, drained(&mut demux_fsm))
        };
        let stating = hybrid_file(false, true);

        assert_eq!(
            read_resuming_at_the_fragment(&stating),
            (Ok(()), stating.fragment_samples.clone())
        );
        assert_eq!(
            read_resuming_at_the_fragment(&hybrid_file(false, false)),
            (
                Err(ErrorKind::Sample(
                    isobmff_sample::ErrorKind::MissingDecodeTime
                )),
                Vec::new()
            )
        );
    }

    /// Ticks a second the movie of [`two_tracks_laid_out_past_their_fragment`] is timed in
    const TIMESCALE: u32 = 90_000;

    /// Decode time the fragment of [`two_tracks_laid_out_past_their_fragment`] states, where it states one
    const FRAGMENT_DECODE_TIME: u64 = 180_000;

    /// A file of two tracks the movie and one fragment each declare samples of, the media data of the movie lying past that of the fragment
    ///
    /// The file is `ftyp moov moof mdat mdat`: track 1 declares two samples of
    /// 3 000 ticks in the sample table and one in the fragment, track 2 one of
    /// 1 000 ticks in each. The fragment states its decode times in a `tfdt`
    /// where `decode_time_stated` is set. Returns the file and the samples it
    /// carries, in the order their bytes lie.
    fn two_tracks_laid_out_past_their_fragment(decode_time_stated: bool) -> (Vec<u8>, Vec<Sample>) {
        let track_chunks: [(u32, u32, &[&[u8]]); 2] =
            [(1, 3_000, &[b"MOV1", b"MOV2"]), (2, 1_000, &[b"MOV3"])];
        let fragment_runs: [(u32, u32, &[u8]); 2] = [(1, 3_000, b"FRG1"), (2, 1_000, b"FRG2")];

        let movie_laying_out = |chunk_offsets: [u64; 2]| {
            let trak: Vec<TrackBox> = track_chunks
                .iter()
                .zip(chunk_offsets)
                .map(|(&(track_id, duration, samples), chunk_offset)| {
                    let sample_count = u32::try_from(samples.len()).unwrap();
                    track_laid_out(
                        track_id,
                        self_contained_data_reference(),
                        sample_table(
                            TimeToSampleBox::from_deltas(samples.iter().map(|_sample| duration)),
                            SampleToChunkBox::from_chunks([(u64::from(sample_count), 1)]).unwrap(),
                            SampleSizes::Stsz(SampleSizeBox::from_sizes(
                                samples
                                    .iter()
                                    .map(|sample| u32::try_from(sample.len()).unwrap()),
                            )),
                            ChunkOffsets::from_offsets([chunk_offset]),
                        ),
                    )
                })
                .collect();

            MovieBox::new_fragmented(TIMESCALE, trak).unwrap()
        };
        let fragment_laying_out = |data_start: u64| {
            let mut data_offset = data_start;
            let track_fragments = fragment_runs
                .iter()
                .map(|&(track_id, duration, sample)| {
                    let sample_size = u32::try_from(sample.len()).unwrap();
                    let run = TrackRunBox::new(
                        Some(i32::try_from(data_offset).unwrap()),
                        None,
                        vec![TrackRunSample::new(
                            Some(duration),
                            Some(sample_size),
                            None,
                            None,
                        )],
                    )
                    .unwrap();
                    data_offset = data_offset.saturating_add(u64::from(sample_size));
                    let track_fragment = TrackFragmentBox::new(
                        TrackFragmentHeaderBox::new(
                            TrackFragmentHeaderFlags::DEFAULT_BASE_IS_MOOF,
                            track_id,
                            None,
                            None,
                            None,
                            None,
                            None,
                        ),
                        vec![run],
                    )
                    .unwrap();

                    if decode_time_stated {
                        track_fragment.with_tfdt(TrackFragmentBaseMediaDecodeTimeBox::new(
                            FRAGMENT_DECODE_TIME,
                        ))
                    } else {
                        track_fragment
                    }
                })
                .collect();

            MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), track_fragments)
        };

        let fragment_media_data = MediaDataBox::new(
            fragment_runs
                .iter()
                .flat_map(|(_track_id, _duration, sample)| sample.iter().copied())
                .collect(),
        );
        let movie_media_data = MediaDataBox::new(
            track_chunks
                .iter()
                .flat_map(|(_track_id, _duration, samples)| samples.concat())
                .collect(),
        );
        let header_len = |media_data: &MediaDataBox| {
            media_data
                .encoded_len()
                .saturating_sub(media_data.payload_len())
        };
        let fragment_len = fragment_laying_out(0).encoded_len();
        let fragment =
            fragment_laying_out(fragment_len.saturating_add(header_len(&fragment_media_data)));
        // Why not building the movie once: its chunk offsets lie past the
        // movie itself. The placeholders and the offsets both fit 32 bits, so
        // both movies state them in a `stco` of the same length.
        let movie_len = movie_laying_out([0, 0]).encoded_len();
        let first_chunk = u64::try_from(written(&file_type()).len())
            .unwrap()
            .saturating_add(movie_len)
            .saturating_add(fragment.encoded_len())
            .saturating_add(fragment_media_data.encoded_len())
            .saturating_add(header_len(&movie_media_data));
        let [(_track_id, _duration, first_chunk_samples), _second] = track_chunks;
        let second_chunk =
            first_chunk.saturating_add(u64::try_from(first_chunk_samples.concat().len()).unwrap());
        let movie = movie_laying_out([first_chunk, second_chunk]);

        let file = [
            written(&file_type()),
            written(&movie),
            written(&fragment),
            written(&fragment_media_data),
            written(&movie_media_data),
        ]
        .concat();
        let sample = |track_id, decode_time, duration, data: &[u8]| {
            Sample::new(
                track_id,
                decode_time,
                duration,
                0,
                SampleFlags::ZERO,
                1,
                data.to_vec(),
            )
        };
        let fragment_start = |movie_duration| {
            if decode_time_stated {
                FRAGMENT_DECODE_TIME
            } else {
                movie_duration
            }
        };
        let samples = vec![
            sample(1, fragment_start(6_000), 3_000, b"FRG1"),
            sample(2, fragment_start(1_000), 1_000, b"FRG2"),
            sample(1, 0, 3_000, b"MOV1"),
            sample(1, 3_000, 3_000, b"MOV2"),
            sample(2, 0, 1_000, b"MOV3"),
        ];

        (file, samples)
    }

    #[test]
    fn the_samples_of_several_tracks_a_movie_declares_past_its_fragment_are_read_with_theirs() {
        for decode_time_stated in [true, false] {
            let (file, samples) = two_tracks_laid_out_past_their_fragment(decode_time_stated);

            for cut_length in [7, file.len()] {
                assert_eq!(
                    samples_of(&file, cut_length),
                    samples,
                    "decode time stated: {decode_time_stated}, cut length: {cut_length}"
                );
            }
        }
    }
}
