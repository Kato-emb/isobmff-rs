//! The offsets a `moov` gives into the file it is in, moved where a mapping says

#[cfg(test)]
mod tests {
    use isobmff::structure_data::{Error, relocate_offsets};
    use isobmff_boxes::{
        ChunkOffsetBox, ChunkOffsetVariant, DataEntry, DataEntryImdaBox, DataEntryUrlBox, MovieBox,
        SampleAuxiliaryInformationOffsetsBox, SampleDescription, SampleEntry, SampleToChunkBox,
        SampleToChunkEntry, TrackBox,
    };
    use isobmff_core::BoxDefinition;
    use isobmff_test_support::fixtures::{encoded, in_another_file, movie, track};

    /// The first offset past 32 bits, 4,294,967,296
    const PAST_32_BITS: u64 = 1 << 32;

    fn trak(movie: &MovieBox, track: usize) -> &TrackBox {
        movie.trak.get(track).unwrap()
    }

    fn chunk_offsets(movie: &MovieBox, track: usize) -> Option<(&'static str, Vec<u64>)> {
        match &trak(movie, track).mdia.minf.stbl.chunk_offset {
            Some(ChunkOffsetVariant::ChunkOffset(stco)) => Some((
                "stco",
                stco.chunk_offsets.iter().copied().map(u64::from).collect(),
            )),
            Some(ChunkOffsetVariant::ChunkLargeOffset(co64)) => {
                Some(("co64", co64.chunk_offsets.clone()))
            }
            _ => None,
        }
    }

    /// Returns a movie of `tracks` that writes
    fn movie_of(tracks: impl IntoIterator<Item = TrackBox>) -> MovieBox {
        let mut movie = movie(tracks);
        movie.mvhd.next_track_id = 4;
        movie
    }

    fn with_saio(mut trak: TrackBox, offsets: &[u64]) -> TrackBox {
        let mut saio = SampleAuxiliaryInformationOffsetsBox::default();
        saio.version = u8::from(offsets.iter().any(|&offset| u32::try_from(offset).is_err()));
        saio.offsets = offsets.to_vec();
        trak.mdia.minf.stbl.saio = vec![saio];
        trak
    }

    fn saio(movie: &MovieBox, track: usize) -> (u8, Vec<u64>) {
        let saio = trak(movie, track).mdia.minf.stbl.saio.first().unwrap();
        (saio.version, saio.offsets.clone())
    }

    /// Returns `movie` written, with no offset in the tables `relocate_offsets` maps
    fn without_offsets(movie: &MovieBox) -> Vec<u8> {
        let mut movie = movie.clone();
        for trak in &mut movie.trak {
            let stbl = &mut trak.mdia.minf.stbl;
            stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkOffset(ChunkOffsetBox::default()));
            for saio in &mut stbl.saio {
                saio.version = 0;
                saio.offsets.clear();
            }
        }
        encoded(&movie)
    }

    #[test]
    fn offsets_mapped_past_32_bits_are_written_in_a_co64_and_a_saio_of_version_1() {
        let mut movie = movie_of([with_saio(
            track(1, 10, &[4, 4], &[(1, 1)], &[100, 200]),
            &[50, 60],
        )]);
        relocate_offsets(&mut movie, |offset| offset.checked_add(PAST_32_BITS)).unwrap();
        assert_eq!(
            chunk_offsets(&movie, 0).unwrap(),
            ("co64", vec![4_294_967_396, 4_294_967_496])
        );
        assert_eq!(saio(&movie, 0), (1, vec![4_294_967_346, 4_294_967_356]));
    }

    #[test]
    fn offsets_mapped_back_into_32_bits_are_written_in_a_stco_and_a_saio_of_version_0() {
        let mut movie = movie_of([with_saio(
            track(1, 10, &[4, 4], &[(1, 1)], &[4_294_967_396, 4_294_967_496]),
            &[4_294_967_346, 4_294_967_356],
        )]);
        relocate_offsets(&mut movie, |offset| offset.checked_sub(PAST_32_BITS)).unwrap();
        assert_eq!(chunk_offsets(&movie, 0).unwrap(), ("stco", vec![100, 200]));
        assert_eq!(saio(&movie, 0), (0, vec![50, 60]));
    }

    #[test]
    fn the_last_offset_of_32_bits_stays_in_a_stco_and_one_past_it_takes_a_co64() {
        let last = u64::from(u32::MAX);
        let mut fits = movie_of([track(1, 10, &[4], &[(1, 1)], &[4_294_967_294])]);
        relocate_offsets(&mut fits, |offset| offset.checked_add(1)).unwrap();
        assert_eq!(chunk_offsets(&fits, 0).unwrap(), ("stco", vec![last]));

        let mut past = movie_of([track(1, 10, &[4], &[(1, 1)], &[last])]);
        relocate_offsets(&mut past, |offset| offset.checked_add(1)).unwrap();
        assert_eq!(
            chunk_offsets(&past, 0).unwrap(),
            ("co64", vec![PAST_32_BITS])
        );
    }

    #[test]
    fn one_saio_offset_for_every_chunk_moves_as_the_first_chunk_does() {
        let mut movie = movie_of([with_saio(
            track(1, 10, &[4, 4], &[(1, 1)], &[100, 200]),
            &[50],
        )]);
        relocate_offsets(&mut movie, |offset| offset.checked_add(1_000)).unwrap();
        assert_eq!(saio(&movie, 0), (0, vec![1_050]));
    }

    #[test]
    fn the_chunks_of_media_data_in_another_file_or_an_imda_keep_their_offsets() {
        let mut imda = track(3, 10, &[4], &[(1, 1)], &[8]);
        let mut entry = DataEntryImdaBox::default();
        entry.imda_ref_identifier = 1;
        imda.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Imda(entry)];
        let mut movie = movie_of([
            track(1, 10, &[4], &[(1, 1)], &[100]),
            with_saio(
                in_another_file(track(2, 10, &[4], &[(1, 1)], &[300])),
                &[20],
            ),
            imda,
        ]);
        let before = movie.clone();
        relocate_offsets(&mut movie, |offset| offset.checked_add(1_000)).unwrap();
        assert_eq!(chunk_offsets(&movie, 0).unwrap(), ("stco", vec![1_100]));
        for track in [1, 2] {
            assert_eq!(encoded(trak(&movie, track)), encoded(trak(&before, track)));
        }
    }

    #[test]
    fn a_chunk_moves_as_the_data_reference_entry_its_sample_entry_names_says() {
        let mut trak = with_saio(
            track(1, 10, &[4, 4, 4], &[(1, 1)], &[100, 200, 300]),
            &[10, 20, 30],
        );
        let mut elsewhere = DataEntryUrlBox::default();
        elsewhere.location = Some(isobmff_core::Utf8String::from_slice(b"other.mp4"));
        trak.mdia.minf.dinf.dref.data_entries = vec![
            DataEntry::Url(DataEntryUrlBox::default()),
            DataEntry::Url(elsewhere),
        ];
        let mut second = SampleEntry::default();
        second.data_reference_index = 2;
        let stbl = &mut trak.mdia.minf.stbl;
        stbl.stsd
            .sample_descriptions
            .push(SampleDescription::Typed(second));
        let mut run = SampleToChunkEntry::default();
        run.first_chunk = 2;
        run.samples_per_chunk = 1;
        run.sample_description_index = 2;
        let mut back = run;
        back.first_chunk = 3;
        back.sample_description_index = 1;
        stbl.stsc.as_mut().unwrap().entries.extend([run, back]);
        let mut movie = movie_of([trak]);
        relocate_offsets(&mut movie, |offset| offset.checked_add(1_000)).unwrap();
        assert_eq!(
            chunk_offsets(&movie, 0).unwrap(),
            ("stco", vec![1_100, 200, 1_300])
        );
        assert_eq!(saio(&movie, 0), (0, vec![1_010, 20, 1_030]));
    }

    #[test]
    fn an_offset_the_mapping_does_not_move_is_refused_and_the_movie_is_left_as_it_was() {
        let mut movie = movie_of([
            track(1, 10, &[4], &[(1, 1)], &[100]),
            with_saio(track(2, 10, &[4, 4], &[(1, 1)], &[200, 300]), &[400, 500]),
        ]);
        let before = encoded(&movie);
        assert_eq!(
            relocate_offsets(&mut movie, |offset| offset
                .checked_add(1)
                .filter(|_| offset != 500)),
            Err(Error::OffsetNotRelocated {
                track_id: 2,
                offset: 500
            })
        );
        assert_eq!(encoded(&movie), before);
    }

    #[test]
    fn an_offset_mapped_past_64_bits_is_refused() {
        let mut movie = movie_of([track(1, 10, &[4], &[(1, 1)], &[100])]);
        assert_eq!(
            relocate_offsets(&mut movie, |offset| offset.checked_add(u64::MAX)),
            Err(Error::OffsetNotRelocated {
                track_id: 1,
                offset: 100
            })
        );
    }

    #[test]
    fn every_box_but_the_offset_tables_is_written_as_it_was() {
        let mut movie = movie_of([
            with_saio(track(1, 10, &[4, 5], &[(1, 1)], &[100, 200]), &[50, 60]),
            track(2, 20, &[6, 7, 8], &[(1, 3)], &[300]),
        ]);
        let before = without_offsets(&movie);
        relocate_offsets(&mut movie, |offset| offset.checked_add(PAST_32_BITS)).unwrap();
        assert_eq!(without_offsets(&movie), before);
    }

    #[test]
    fn a_saio_of_neither_one_offset_nor_one_for_each_chunk_is_refused() {
        let mut movie = movie_of([with_saio(
            track(1, 10, &[4, 4], &[(1, 1)], &[100, 200]),
            &[10, 20, 30],
        )]);
        assert_eq!(
            relocate_offsets(&mut movie, |offset| offset.checked_add(1)),
            Err(Error::InvalidEntry {
                track_id: 1,
                box_type: SampleAuxiliaryInformationOffsetsBox::BOX_TYPE,
                entry: 0
            })
        );
    }

    #[test]
    fn a_chunk_under_no_sample_to_chunk_entry_is_refused() {
        let mut movie = movie_of([track(1, 10, &[4, 4], &[(2, 1)], &[100, 200])]);
        assert_eq!(
            relocate_offsets(&mut movie, |offset| offset.checked_add(1)),
            Err(Error::InvalidEntry {
                track_id: 1,
                box_type: SampleToChunkBox::BOX_TYPE,
                entry: 0
            })
        );
    }
}
