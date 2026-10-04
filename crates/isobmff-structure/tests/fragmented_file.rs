//! The samples of a fragmented file laid out by hand, read back through its structure

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/reading.rs`. The
// `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use isobmff_boxes::MovieFragmentRandomAccessOffsetBox;
    use isobmff_core::BoxType;
    use isobmff_sequence::BoxEvent;
    use isobmff_structure::{Error, ErrorKind, FragmentedDemuxFsm, WantedInput};
    use isobmff_test_support::{
        IndexedFile, events_of, fragmented_file_samples, fragmented_file_with_samples,
        indexed_fragmented_file, indexed_fragmented_file_without_decode_times,
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
}
