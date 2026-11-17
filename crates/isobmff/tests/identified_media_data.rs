//! The samples that lie in an `imda`, resolved by the structure-data reader from where the `imda` lies

#[cfg(test)]
mod tests {
    use isobmff::boxes::{
        DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, MovieBox, MovieExtendsBox,
        MovieFragmentBox, SampleFlags, TrackBox, TrackExtendsBox, TrackFragmentBox, TrackRunBox,
        TrackRunSamples,
    };
    use isobmff::structure_data::{Error, StructureDataReader};
    use isobmff::{SampleLocation, SampleProperties};
    use isobmff_test_support::fixtures::{movie, track};

    /// A sample of track 1 at `offset`, of `size` bytes, at `decoding_time` lasting `sample_duration`, that the first sample entry describes
    fn at(offset: u64, size: u32, decoding_time: u64, sample_duration: u32) -> SampleLocation {
        SampleLocation {
            properties: SampleProperties {
                track_id: 1,
                decoding_time,
                sample_duration,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            offset,
            size,
        }
    }

    /// `trak` with its media data in the `imda` its only data reference entry names
    fn in_identified_media_data(mut trak: TrackBox, entry: DataEntry) -> TrackBox {
        trak.mdia.minf.dinf.dref.data_entries = vec![entry];
        trak
    }

    fn imdt(imda_ref_identifier: u32) -> DataEntry {
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = imda_ref_identifier;
        DataEntry::Imda(imdt)
    }

    fn snim() -> DataEntry {
        DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default())
    }

    /// A movie of track 1, of samples of 2 and 3 bytes in one chunk at `chunk_offset` lasting 10, in the `imda` of identifier 7
    fn movie_in_identified_media_data(chunk_offset: u64) -> MovieBox {
        movie([in_identified_media_data(
            track(1, 10, &[2, 3], &[(1, 2)], &[chunk_offset]),
            imdt(7),
        )])
    }

    /// A movie of track 1 with no sample, whose fragments lie in the `imda` of their sequence number and take the defaults of a `trex`: samples of 4 bytes lasting 20, sample entry 1
    fn movie_of_fragments() -> MovieBox {
        let mut movie = movie([in_identified_media_data(
            track(1, 10, &[], &[], &[]),
            snim(),
        )]);
        let mut trex = TrackExtendsBox::default();
        trex.track_id = 1;
        trex.default_sample_description_index = 1;
        trex.default_sample_duration = 20;
        trex.default_sample_size = 4;
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = vec![trex];
        movie.mvex = Some(mvex);
        movie
    }

    /// A track fragment of track 1 of one run of `sample_count` samples that take every default, at `data_offset` from its base
    fn traf(sample_count: u32, data_offset: Option<i32>) -> TrackFragmentBox {
        let mut traf = TrackFragmentBox::default();
        traf.tfhd.track_id = 1;
        let mut trun = TrackRunBox::default();
        trun.data_offset = data_offset;
        trun.samples = TrackRunSamples::EmptyRows { sample_count };
        traf.trun = vec![trun];
        traf
    }

    /// A movie fragment of `sequence_number`
    fn moof(
        sequence_number: u32,
        trafs: impl IntoIterator<Item = TrackFragmentBox>,
    ) -> MovieFragmentBox {
        let mut moof = MovieFragmentBox::default();
        moof.mfhd.sequence_number = sequence_number;
        moof.traf = trafs.into_iter().collect();
        moof
    }

    fn reader() -> StructureDataReader {
        StructureDataReader::new(1024, 16)
    }

    #[test]
    fn the_chunk_offsets_of_a_track_in_an_imda_count_from_its_payload_whether_it_comes_before_or_after_the_moov()
     {
        let expected = [at(1004, 2, 0, 10), at(1006, 3, 10, 10)];
        let mut after = reader();
        assert_eq!(
            after
                .handle_movie(&movie_in_identified_media_data(4))
                .unwrap()
                .len(),
            0
        );
        let found: Vec<_> = after
            .handle_identified_media_data(7, 1000)
            .unwrap()
            .collect();
        assert_eq!(found, expected);
        assert_eq!(after.finish(), Ok(()));

        let mut before = reader();
        assert_eq!(
            before.handle_identified_media_data(7, 1000).unwrap().len(),
            0
        );
        let found: Vec<_> = before
            .handle_movie(&movie_in_identified_media_data(4))
            .unwrap()
            .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn a_movie_whose_samples_lie_in_the_imda_of_a_sequence_number_is_refused() {
        let movie = movie([in_identified_media_data(
            track(1, 10, &[2, 3], &[(1, 2)], &[4]),
            snim(),
        )]);
        let refused = Err(Error::SequenceNumberOutsideMovieFragment {
            track_id: 1,
            data_reference_index: 1,
        });
        assert_eq!(reader().handle_movie(&movie).map(|_| ()), refused);
        assert_eq!(reader().handle_movie_lossy(&movie).map(|_| ()), refused);
    }

    /// The locations `moof`, at file offset 500, resolves to once the `imda` of identifier 2 is read at file offset 1000
    fn resolved(moof: &MovieFragmentBox) -> Vec<SampleLocation> {
        let mut reader = reader();
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        let mut found: Vec<_> = reader.handle_movie_fragment(moof, 500).unwrap().collect();
        found.extend(reader.handle_identified_media_data(2, 1000).unwrap());
        assert_eq!(reader.finish(), Ok(()));
        found
    }

    #[test]
    fn the_first_track_fragment_counts_from_the_payload_of_the_imda_of_its_sequence_number_and_the_next_from_the_end_of_its_data()
     {
        assert_eq!(
            resolved(&moof(2, [traf(1, Some(4)), traf(1, None)])),
            [at(1004, 4, 0, 20), at(1008, 4, 20, 20)]
        );
    }

    #[test]
    fn a_base_the_tfhd_gives_counts_from_the_payload_of_the_imda() {
        let mut based = traf(1, Some(4));
        based.tfhd.base_data_offset = Some(100);
        assert_eq!(resolved(&moof(2, [based])), [at(1104, 4, 0, 20)]);
    }

    #[test]
    fn a_track_fragment_based_on_the_moof_counts_from_it_whatever_its_data_reference_entry() {
        let mut on_moof = traf(1, Some(16));
        on_moof.tfhd.default_base_is_moof = true;
        assert_eq!(resolved(&moof(2, [on_moof])), [at(516, 4, 0, 20)]);
    }

    #[test]
    fn the_samples_of_an_imda_read_before_their_fragment_are_resolved_with_it() {
        let mut reader = reader();
        assert_eq!(
            reader.handle_identified_media_data(3, 2000).unwrap().len(),
            0
        );
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        let found: Vec<_> = reader
            .handle_movie_fragment(&moof(3, [traf(2, Some(4))]), 500)
            .unwrap()
            .collect();
        assert_eq!(found, [at(2004, 4, 0, 20), at(2008, 4, 20, 20)]);
    }

    #[test]
    fn an_imda_of_the_identifier_of_one_read_before_is_refused_and_the_first_kept() {
        let mut reader = reader();
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        assert_eq!(
            reader.handle_identified_media_data(1, 1000).unwrap().len(),
            0
        );
        assert_eq!(
            reader.handle_identified_media_data(1, 3000).map(|_| ()),
            Err(Error::DuplicateImdaIdentifier { imda_identifier: 1 })
        );
        let found: Vec<_> = reader
            .handle_movie_fragment(&moof(1, [traf(1, Some(4))]), 500)
            .unwrap()
            .collect();
        assert_eq!(found, [at(1004, 4, 0, 20)]);
    }

    #[test]
    fn an_imda_past_the_limit_is_not_remembered() {
        let mut reader = StructureDataReader::new(1024, 1);
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        assert_eq!(
            reader.handle_identified_media_data(1, 1000).unwrap().len(),
            0
        );
        assert_eq!(
            reader.handle_identified_media_data(2, 2000).map(|_| ()),
            Err(Error::IdentifiedMediaDataOverLimit {
                imda_identifier: 2,
                limit: 1
            })
        );
        assert_eq!(
            reader
                .handle_movie_fragment(&moof(2, [traf(1, Some(4))]), 500)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            reader.finish(),
            Err(Error::IdentifiedMediaDataNotFound {
                imda_identifier: 2,
                sample_count: 1
            })
        );
    }

    #[test]
    fn the_samples_that_wait_for_an_imda_count_against_the_sample_limit() {
        let mut reader = StructureDataReader::new(2, 16);
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        assert_eq!(
            reader
                .handle_movie_fragment(&moof(1, [traf(2, Some(4))]), 500)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            reader
                .handle_movie_fragment(&moof(2, [traf(1, Some(4))]), 600)
                .map(|_| ()),
            Err(Error::SampleCountOverLimit {
                sample_count: 3,
                limit: 2
            })
        );
        assert_eq!(
            reader.handle_identified_media_data(1, 1000).unwrap().len(),
            2
        );
        assert_eq!(
            reader
                .handle_movie_fragment(&moof(2, [traf(1, Some(4))]), 600)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn a_fragment_refused_leaves_no_sample_waiting() {
        let mut reader = reader();
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        let mut unknown = traf(1, None);
        unknown.tfhd.track_id = 9;
        assert_eq!(
            reader
                .handle_movie_fragment(&moof(1, [traf(1, Some(4)), unknown]), 500)
                .map(|_| ()),
            Err(Error::UnknownTrack { track_id: 9 })
        );
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn the_end_refuses_the_samples_of_an_imda_the_file_lacks_naming_the_least_identifier() {
        let mut reader = reader();
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        for (sequence_number, sample_count) in [(5, 1), (3, 2), (4, 1)] {
            let _ = reader
                .handle_movie_fragment(&moof(sequence_number, [traf(sample_count, Some(4))]), 500)
                .unwrap()
                .count();
        }
        assert_eq!(
            reader.finish(),
            Err(Error::IdentifiedMediaDataNotFound {
                imda_identifier: 3,
                sample_count: 2
            })
        );
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    #[should_panic(expected = "after finish")]
    fn an_imda_after_the_end_is_a_bug_of_the_caller() {
        let mut reader = reader();
        let _ = reader.finish();
        let _ = reader.handle_identified_media_data(1, 1000);
    }

    const OVERFLOW: Result<(), Error> = Err(Error::Overflow { track_id: 1 });

    #[test]
    fn an_imda_past_which_a_sample_that_waits_for_it_lies_is_refused_and_the_sample_goes_on_waiting()
     {
        let mut reader = reader();
        let _ = reader
            .handle_movie(&movie_in_identified_media_data(u64::from(u32::MAX)))
            .unwrap()
            .count();
        assert_eq!(
            reader
                .handle_identified_media_data(7, u64::MAX - u64::from(u32::MAX))
                .map(|_| ()),
            OVERFLOW
        );
        assert_eq!(reader.handle_identified_media_data(7, 0).unwrap().len(), 2);
    }

    #[test]
    fn a_movie_whose_samples_lie_past_the_last_file_offset_from_an_imda_read_before_is_refused() {
        let mut reader = reader_with(7, u64::MAX - 4);
        assert_eq!(
            reader
                .handle_movie(&movie_in_identified_media_data(4))
                .map(|_| ()),
            OVERFLOW
        );
    }

    #[test]
    fn a_fragment_whose_tfhd_base_lies_past_the_last_file_offset_from_an_imda_read_before_is_refused()
     {
        let mut based = traf(1, Some(i32::MAX));
        based.tfhd.base_data_offset = Some(u64::MAX - u64::from(u32::MAX));
        let mut reader = reader_with(2, u64::from(u32::MAX));
        let _ = reader.handle_movie(&movie_of_fragments()).unwrap().count();
        assert_eq!(
            reader
                .handle_movie_fragment(&moof(2, [based]), 500)
                .map(|_| ()),
            OVERFLOW
        );
    }

    /// A reader that has read the `imda` of `imda_identifier` with its payload at `payload_offset`
    fn reader_with(imda_identifier: u32, payload_offset: u64) -> StructureDataReader {
        let mut reader = reader();
        assert_eq!(
            reader
                .handle_identified_media_data(imda_identifier, payload_offset)
                .unwrap()
                .len(),
            0
        );
        reader
    }
}
