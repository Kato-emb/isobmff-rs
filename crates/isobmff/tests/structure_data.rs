//! The location of each sample the sample tables of a movie declare, checked against GPAC and FFmpeg on the conformance files and the fixtures

#[cfg(test)]
mod common;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error as _;
    use std::num::NonZeroU32;

    use isobmff::boxes::{
        ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant, CompositionOffsetBox,
        CompositionOffsetEntry, DataEntry, DataEntrySeqNumImdaBox, MovieBox,
        SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleDescription, SampleEntry,
        SampleFlags, SampleSizeBox, SampleSizeVariant, SampleSizes, SampleToChunkBox,
        SampleToChunkEntry, SyncSampleBox, TimeToSampleBox, TrackBox,
    };
    use isobmff::structure_data::{Error, MovieReader};
    use isobmff::{
        BoxDefinition, BoxType, DataOrigin, FourCc, MediaDataContainer, SampleLocation,
        SampleProperties,
    };
    use isobmff_test_support::fixtures::{movie, track};

    use crate::common::{REFUSED_FIXTURES, compare_with_gpac_and_ffprobe};
    use isobmff_test_support::fixture_movie_box;

    /// The location of the sample of track `track_id` decoded at `decoding_time` for `sample_duration`, at data offset `offset` of the file for `size` bytes, of the first sample entry
    fn location(
        track_id: u32,
        (decoding_time, sample_duration): (u64, u32),
        (offset, size): (u64, u32),
    ) -> SampleLocation {
        SampleLocation {
            properties: SampleProperties {
                track_id,
                decoding_time,
                sample_duration,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            container: MediaDataContainer::SameFile,
            data_origin: DataOrigin::Container,
            offset,
            size,
        }
    }

    /// Returns why `movie` is refused, for the track it names
    fn refused(movie: &MovieBox) -> Option<Error> {
        MovieReader::new().handle_movie(movie).err()
    }

    /// The refusal of the tables of track `track_id`, for `source`
    fn of_track(track_id: u32, source: Error) -> Option<Error> {
        Some(Error::Track {
            track_id,
            source: Box::new(source),
        })
    }

    #[test]
    fn the_samples_of_each_track_come_in_decoding_order_track_after_track() {
        let movie = movie([
            track(1, 10, &[3, 4, 5], &[(1, 2), (2, 1)], &[100, 200]),
            track(2, 7, &[], &[], &[]),
            track(3, 20, &[6, 7], &[(1, 1)], &[300, 50]),
        ]);
        let mut locations = MovieReader::new().handle_movie(&movie).unwrap();
        assert_eq!(locations.remaining(), 5);
        let first = locations.next();
        assert_eq!(locations.remaining(), 4);
        assert_eq!(
            first.into_iter().chain(locations).collect::<Vec<_>>(),
            [
                location(1, (0, 10), (100, 3)),
                location(1, (10, 10), (103, 4)),
                location(1, (20, 10), (200, 5)),
                location(3, (0, 20), (300, 6)),
                location(3, (20, 20), (50, 7)),
            ]
        );
    }

    /// A track of 4 samples of 1 byte, lasting 1 each, 2 in each of its 2 chunks
    fn track_of_4() -> TrackBox {
        track(1, 1, &[1; 4], &[(1, 2)], &[0, 100])
    }

    #[test]
    fn a_table_that_covers_another_number_of_samples_than_the_sample_size_table_is_refused() {
        let mismatch = |box_type, table_sample_count| {
            of_track(
                1,
                Error::SampleCountMismatch {
                    box_type,
                    table_sample_count,
                    sample_count: 4,
                },
            )
        };
        let mut trak = track_of_4();
        let stbl = &mut trak.mdia.minf.stbl;
        stbl.stts
            .as_mut()
            .unwrap()
            .entries
            .first_mut()
            .unwrap()
            .sample_count = 5;
        assert_eq!(
            refused(&movie([trak])),
            mismatch(TimeToSampleBox::BOX_TYPE, 5)
        );

        let mut trak = track_of_4();
        let mut ctts = CompositionOffsetBox::default();
        let mut entry = CompositionOffsetEntry::default();
        entry.sample_count = 3;
        ctts.entries = vec![entry];
        trak.mdia.minf.stbl.ctts = Some(ctts);
        assert_eq!(
            refused(&movie([trak])),
            mismatch(CompositionOffsetBox::BOX_TYPE, 3)
        );

        let mut trak = track_of_4();
        let mut sdtp = SampleDependencyTypeBox::default();
        sdtp.entries = vec![SampleDependencyTypeEntry::default(); 6];
        trak.mdia.minf.stbl.sdtp = Some(sdtp);
        assert_eq!(
            refused(&movie([trak])),
            mismatch(SampleDependencyTypeBox::BOX_TYPE, 6)
        );

        let trak = track(1, 1, &[1; 4], &[(1, 3)], &[0, 100]);
        assert_eq!(
            refused(&movie([trak])),
            mismatch(SampleToChunkBox::BOX_TYPE, 6)
        );
    }

    #[test]
    fn a_sample_table_without_a_mandatory_box_is_refused() {
        let without = |remove: fn(&mut TrackBox), box_type| {
            let mut trak = track_of_4();
            remove(&mut trak);
            assert_eq!(
                refused(&movie([trak])),
                of_track(1, Error::MandatoryBoxMissing { box_type })
            );
        };
        without(
            |trak| trak.mdia.minf.stbl.stts = None,
            TimeToSampleBox::BOX_TYPE,
        );
        without(
            |trak| trak.mdia.minf.stbl.stsc = None,
            SampleToChunkBox::BOX_TYPE,
        );
        without(
            |trak| trak.mdia.minf.stbl.chunk_offset = None,
            ChunkOffsetBox::BOX_TYPE,
        );
        without(
            |trak| trak.mdia.minf.stbl.sample_size = None,
            SampleSizeBox::BOX_TYPE,
        );
    }

    #[test]
    fn a_sync_sample_table_that_names_no_sample_is_refused() {
        let mut trak = track_of_4();
        let mut stss = SyncSampleBox::default();
        stss.sample_numbers = vec![1, 5];
        trak.mdia.minf.stbl.stss = Some(stss);
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::SyncSampleNumberOutOfRange {
                    entry: 1,
                    sample_number: 5,
                    sample_count: 4
                }
            )
        );
    }

    #[test]
    fn a_data_reference_to_the_sequence_number_of_a_movie_fragment_is_refused_for_a_track_of_samples_alone()
     {
        let sequence_number = DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default());
        let mut no_sample = track(1, 20, &[], &[], &[]);
        no_sample.mdia.minf.dinf.dref.data_entries = vec![sequence_number.clone()];
        let mut some = track(2, 20, &[1], &[(1, 1)], &[0]);
        some.mdia.minf.dinf.dref.data_entries = vec![sequence_number];
        assert_eq!(refused(&movie([no_sample])), None);
        assert_eq!(
            refused(&movie([some])),
            of_track(
                2,
                Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 1
                }
            )
        );
    }

    #[test]
    fn a_run_of_no_sample_may_name_a_sample_entry_the_track_does_not_have() {
        let mut trak = track(1, 1, &[1; 2], &[(1, 1), (2, 0), (3, 1)], &[0, 10, 20]);
        stsc_entry(&mut trak, 1).sample_description_index = 9;
        assert_eq!(refused(&movie([trak.clone()])), None);
        stsc_entry(&mut trak, 2).sample_description_index = 9;
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::SampleDescriptionIndexOutOfRange {
                    sample_description_index: 9,
                    sample_entry_count: 1
                }
            )
        );
    }

    /// The entry `index`, from 0, of the sample-to-chunk table of `trak`
    fn stsc_entry(trak: &mut TrackBox, index: usize) -> &mut SampleToChunkEntry {
        trak.mdia
            .minf
            .stbl
            .stsc
            .as_mut()
            .unwrap()
            .entries
            .get_mut(index)
            .unwrap()
    }

    #[test]
    fn a_data_reference_is_reported_before_a_sample_whose_end_is_past_a_u64() {
        let mut trak = track(1, 1, &[10, 1], &[(1, 1), (2, 1)], &[0, 0]);
        let mut co64 = ChunkLargeOffsetBox::default();
        co64.chunk_offsets = vec![u64::MAX - 5, 0];
        trak.mdia.minf.stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkLargeOffset(co64));
        let descriptions = &mut trak.mdia.minf.stbl.stsd.sample_descriptions;
        descriptions.push(SampleDescription::Typed(SampleEntry::new(
            FourCc::new([0; 4]),
            2,
            Vec::new(),
        )));
        trak.mdia
            .minf
            .dinf
            .dref
            .data_entries
            .push(DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default()));
        stsc_entry(&mut trak, 1).sample_description_index = 2;
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::SequenceNumberOutsideMovieFragment {
                    sample_description_index: 2
                }
            )
        );
    }

    #[test]
    fn a_sample_whose_end_is_past_what_a_u64_counts_is_refused() {
        let mut trak = track(1, 1, &[1, 10, 1], &[(1, 2), (2, 1)], &[0, 0]);
        let mut co64 = ChunkLargeOffsetBox::default();
        co64.chunk_offsets = vec![u64::MAX - 5, 0];
        trak.mdia.minf.stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkLargeOffset(co64));
        assert_eq!(
            refused(&movie([trak])),
            of_track(
                1,
                Error::DataOffsetOverflow {
                    sample_number: 2,
                    offset: u64::MAX - 4,
                    size: 10
                }
            )
        );
    }

    #[test]
    fn a_movie_is_refused_whole_for_the_first_track_refused() {
        let mut second = track(2, 1, &[1], &[(1, 1)], &[0]);
        second.mdia.minf.stbl.stts = None;
        let mut third = track(3, 1, &[1], &[(1, 1)], &[0]);
        third.mdia.minf.stbl.stsc = None;
        let error = refused(&movie([track_of_4(), second, third])).unwrap();
        assert_eq!(
            (error.to_string(), error.source().map(ToString::to_string)),
            (
                String::from("the structure-data of track 2 is refused"),
                Some(String::from(
                    "the sample table holds no `stts` this crate reads, which is mandatory"
                ))
            )
        );
    }

    /// A track of `track_id` declaring the most samples a sample size table counts, each of 1 byte and lasting 1, in one chunk, with one entry in each table
    fn track_of_the_most_samples(track_id: u32) -> TrackBox {
        let mut trak = track(track_id, 1, &[], &[(1, u32::MAX)], &[0]);
        let stbl = &mut trak.mdia.minf.stbl;
        let mut stsz = SampleSizeBox::default();
        stsz.sample_sizes = SampleSizes::Constant {
            sample_size: NonZeroU32::MIN,
            sample_count: u32::MAX,
        };
        stbl.sample_size = Some(SampleSizeVariant::SampleSize(stsz));
        stbl.stts
            .as_mut()
            .unwrap()
            .entries
            .first_mut()
            .unwrap()
            .sample_count = u32::MAX;
        trak
    }

    #[test]
    fn the_tables_are_checked_without_a_step_for_each_sample_they_declare() {
        let movie = movie((1..=4).map(track_of_the_most_samples));
        let locations = MovieReader::new().handle_movie(&movie).unwrap();
        assert_eq!(locations.remaining(), 4 * u64::from(u32::MAX));
    }

    #[test]
    fn the_fixtures_whose_tables_disagree_are_refused() {
        for (name, box_type, table_sample_count) in REFUSED_FIXTURES {
            let movie = fixture_movie_box(name);
            assert_eq!(
                refused(&movie),
                of_track(
                    1,
                    Error::SampleCountMismatch {
                        box_type: BoxType::compact(box_type),
                        table_sample_count,
                        sample_count: 12
                    }
                ),
                "{name}"
            );
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe"]
    fn the_samples_of_each_track_lie_and_are_timed_where_gpac_and_ffprobe_read_them() {
        compare_with_gpac_and_ffprobe(
            |_| None,
            |_, moov| {
                let locations = MovieReader::new()
                    .handle_movie(moov)
                    .map_err(|error| format!("{error}: {:?}", error.source()))?;
                let declared = locations.remaining();
                let mut ours = BTreeMap::<u32, Vec<SampleLocation>>::new();
                for location in locations {
                    ours.entry(location.properties.track_id)
                        .or_default()
                        .push(location);
                }
                let placed = ours.values().map(Vec::len).sum::<usize>();
                if u64::try_from(placed).unwrap() != declared {
                    return Err(format!(
                        "{placed} samples returned of the {declared} counted"
                    ));
                }
                Ok(ours)
            },
        );
    }
}
