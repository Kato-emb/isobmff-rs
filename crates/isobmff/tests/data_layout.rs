//! The data reference entry the sample entry of a sample names, checked against GPAC on the conformance files, and the file offset each sample lies at

#[cfg(test)]
mod tests {
    use isobmff::boxes::{
        DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox,
        DataReferenceBox, MovieBox, SampleDescription, SampleDescriptionBox, SampleEntry,
        SampleFlags,
    };
    use isobmff::structure_data::data_layout::{DataOrigins, data_entry};
    use isobmff::structure_data::{Error, MovieReader};
    use isobmff::{
        AnyBox, BoxDefinition, BoxType, DataOrigin, FourCc, SampleLocation, SampleProperties,
        Utf8String,
    };
    use isobmff_test_support::{conformance_files, fixture_movie_box, gpac_children, gpac_list};

    /// `stsd` of sample entries of the `data_reference_index`es `data_reference_indexes`
    fn stsd(data_reference_indexes: &[u16]) -> SampleDescriptionBox {
        let mut stsd = SampleDescriptionBox::default();
        stsd.sample_descriptions = data_reference_indexes
            .iter()
            .map(|&data_reference_index| {
                SampleDescription::Typed(SampleEntry::new(
                    FourCc::new(*b"avc1"),
                    data_reference_index,
                    Vec::new(),
                ))
            })
            .collect();
        stsd
    }

    fn dref(data_entries: Vec<DataEntry>) -> DataReferenceBox {
        let mut dref = DataReferenceBox::default();
        dref.data_entries = data_entries;
        dref
    }

    #[test]
    fn a_sample_entry_names_the_data_reference_entry_of_its_data_reference_index() {
        let mut urn = DataEntryUrnBox::default();
        urn.name = Utf8String::new(String::from("urn:x")).unwrap();
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = 7;
        let entries = vec![
            DataEntry::Url(DataEntryUrlBox::default()),
            DataEntry::Urn(urn),
            DataEntry::Imda(imdt),
            DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default()),
            DataEntry::Other(AnyBox::from_raw_bytes(
                BoxType::compact(*b"xxxx"),
                Vec::new(),
            )),
        ];
        let (stsd, dref) = (stsd(&[5, 4, 3, 2, 1]), dref(entries.clone()));
        let named = (1..=5)
            .map(|sample_description_index| {
                data_entry(&stsd, &dref, sample_description_index).cloned()
            })
            .collect::<Vec<_>>();
        assert_eq!(named, entries.into_iter().rev().map(Ok).collect::<Vec<_>>());
    }

    #[test]
    fn a_sample_description_index_that_names_no_sample_entry_is_refused() {
        let (stsd, dref) = (
            stsd(&[1, 1]),
            dref(vec![DataEntry::Url(DataEntryUrlBox::default())]),
        );
        let refused = |sample_description_index| {
            Err(Error::SampleDescriptionIndexOutOfRange {
                sample_description_index,
                sample_entry_count: 2,
            })
        };
        assert_eq!(data_entry(&stsd, &dref, 0), refused(0));
        assert_eq!(data_entry(&stsd, &dref, 3), refused(3));
    }

    #[test]
    fn a_sample_entry_that_ends_before_its_data_reference_index_is_refused() {
        let mut stsd = SampleDescriptionBox::default();
        stsd.sample_descriptions = vec![SampleDescription::Other(AnyBox::from_raw_bytes(
            BoxType::compact(*b"avc1"),
            vec![0; 7],
        ))];
        let dref = dref(vec![DataEntry::Url(DataEntryUrlBox::default())]);
        assert_eq!(
            data_entry(&stsd, &dref, 1),
            Err(Error::DataReferenceIndexMissing {
                sample_description_index: 1
            })
        );
    }

    #[test]
    fn a_data_reference_index_that_names_no_data_reference_entry_is_refused() {
        let (stsd, dref) = (
            stsd(&[0, 2]),
            dref(vec![DataEntry::Url(DataEntryUrlBox::default())]),
        );
        let refused = |sample_description_index, data_reference_index| {
            Err(Error::DataReferenceIndexOutOfRange {
                sample_description_index,
                data_reference_index,
                data_entry_count: 1,
            })
        };
        assert_eq!(data_entry(&stsd, &dref, 1), refused(1, 0));
        assert_eq!(data_entry(&stsd, &dref, 2), refused(2, 2));
    }

    /// The type of a data reference entry, and the location it gives, as GPAC reports them or as this crate reads them
    fn kind(data_entry: &DataEntry) -> (String, Option<String>) {
        match data_entry {
            DataEntry::Url(url) => (
                String::from("url "),
                url.location
                    .as_ref()
                    .and_then(Utf8String::as_str)
                    .map(String::from),
            ),
            DataEntry::Urn(urn) => (
                String::from("urn "),
                urn.location
                    .as_ref()
                    .and_then(Utf8String::as_str)
                    .map(String::from),
            ),
            DataEntry::Imda(_) => (String::from("imdt"), None),
            DataEntry::SeqNumImda(_) => (String::from("snim"), None),
            DataEntry::Other(other) => (other.box_type().to_string(), None),
            _ => (String::new(), None),
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn each_sample_entry_names_the_data_reference_entry_gpac_reports() {
        let (mut tracks, mut sample_entries, mut left_out) = (0, 0, 0);
        let mut differences = Vec::<String>::new();
        for file in conformance_files() {
            let Some(moov) = file.movie_box() else {
                continue;
            };
            let gpac_tracks = file
                .report
                .pointer("/IsoMediaFile/MovieBox/TrackBox")
                .map(gpac_list)
                .unwrap_or_default();
            for trak in &moov.trak {
                let track_id = trak.tkhd.track_id;
                let Some(minf) = gpac_tracks
                    .iter()
                    .find(|gpac| {
                        gpac.pointer("/TrackHeaderBox/@TrackID")
                            .and_then(|id| id.as_str())
                            == Some(&track_id.to_string())
                    })
                    .and_then(|gpac| gpac.pointer("/MediaBox/MediaInformationBox"))
                else {
                    differences.push(format!("{}: GPAC reports no track {track_id}", file.name));
                    continue;
                };
                let gpac_entries = minf
                    .pointer("/DataInformationBox/DataReferenceBox/URLDataEntryBox")
                    .map(gpac_list)
                    .unwrap_or_default();
                // Why compare as tallies: GPAC reports the sample entries of
                // different classes apart, out of the order they lie in.
                let gpac_sample_entries = minf
                    .pointer("/SampleTableBox/SampleDescriptionBox")
                    .map(gpac_children)
                    .unwrap_or_default();
                // Why leave a track out where GPAC gives no index: it does
                // not report the data_reference_index of some classes, such
                // as WebVTTSampleEntry, and spells it dataReferenceIndex for
                // others.
                let indexes = gpac_sample_entries
                    .iter()
                    .map(|entry| {
                        entry
                            .get("@DataReferenceIndex")
                            .or_else(|| entry.get("@dataReferenceIndex"))
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(indexes) = indexes else {
                    left_out += 1;
                    continue;
                };
                let mut theirs = indexes
                    .into_iter()
                    .map(|index| {
                        let index = index.as_str()?.parse::<usize>().ok()?;
                        let gpac_entry = gpac_entries.get(index.checked_sub(1)?)?;
                        let location = (gpac_entry.get("@Flags")?.as_str() != Some("1"))
                            .then(|| Some(String::from(gpac_entry.get("@URL")?.as_str()?)))
                            .flatten();
                        Some((String::from(gpac_entry.get("@Type")?.as_str()?), location))
                    })
                    .collect::<Option<Vec<_>>>()
                    .unwrap_or_default();
                let stbl = &trak.mdia.minf.stbl;
                let mut ours = (1..=u32::try_from(stbl.stsd.sample_descriptions.len()).unwrap())
                    .map(|index| data_entry(&stbl.stsd, &trak.mdia.minf.dinf.dref, index).map(kind))
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap_or_default();
                ours.sort();
                theirs.sort();
                if ours.is_empty() || ours != theirs {
                    differences.push(format!(
                        "{} track {track_id}: {ours:?}, GPAC {theirs:?}",
                        file.name
                    ));
                }
                tracks += 1;
                sample_entries += ours.len();
            }
        }
        println!(
            "compared {tracks} tracks, {sample_entries} sample entries; left out {left_out} tracks GPAC gives no data_reference_index of"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(tracks > 0 && sample_entries > 0, "no track was compared");
    }

    /// The location of each sample of `movie`, with the data reference entry of its sample entry
    fn locations_of(movie: &MovieBox) -> Vec<(SampleLocation, &DataEntry)> {
        MovieReader::new()
            .handle_movie(movie)
            .unwrap()
            .map(|location| {
                let trak = movie
                    .trak
                    .iter()
                    .find(|trak| trak.tkhd.track_id == location.properties.track_id)
                    .unwrap();
                let (stsd, dref) = (&trak.mdia.minf.stbl.stsd, &trak.mdia.minf.dinf.dref);
                let entry =
                    data_entry(stsd, dref, location.properties.sample_description_index).unwrap();
                (location, entry)
            })
            .collect()
    }

    /// The number of the samples of each track, and whether the data reference entry of each places them in the same file, in the order of the tracks
    fn tracks_of(locations: &[(SampleLocation, &DataEntry)]) -> Vec<(u32, usize, bool)> {
        let mut tracks = Vec::<(u32, usize, bool)>::new();
        for (location, entry) in locations {
            match tracks.last_mut() {
                Some((track_id, samples, _)) if *track_id == location.properties.track_id => {
                    *samples = samples.saturating_add(1);
                }
                _ => tracks.push((location.properties.track_id, 1, entry.is_in_same_file())),
            }
        }
        tracks
    }

    #[test]
    fn a_sample_that_counts_from_the_start_of_its_file_lies_at_its_data_offset_there() {
        for (name, tracks) in [
            ("movie_moov_first.mp4", [(1, 12, true), (2, 58, true)]),
            ("external_movie.mp4", [(1, 12, false), (2, 58, true)]),
        ] {
            let movie = fixture_movie_box(name);
            let locations = locations_of(&movie);
            assert_eq!(tracks_of(&locations), tracks, "{name}");
            let mut data_origins = DataOrigins::new();
            let placed = locations
                .iter()
                .map(|&(location, entry)| data_origins.handle_location(location, entry))
                .collect::<Vec<_>>();
            assert_eq!(
                placed,
                locations
                    .iter()
                    .map(|&(location, _)| Ok(location))
                    .collect::<Vec<_>>(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_sample_that_counts_from_an_identified_media_data_is_refused_while_none_is_taken() {
        let movie = fixture_movie_box("imda_movie_moov_first.mp4");
        let locations = locations_of(&movie);
        assert_eq!(tracks_of(&locations), [(1, 12, false), (2, 58, false)]);
        let mut data_origins = DataOrigins::new();
        for (location, entry) in locations {
            assert_eq!(
                data_origins.handle_location(location, entry),
                Err(Error::IdentifiedMediaDataNotRead { imda_identifier: 1 })
            );
        }
    }

    /// The location of a sample of the first sample entry, 16 bytes from `data_origin`, of 100 bytes
    fn location(data_origin: DataOrigin) -> SampleLocation {
        SampleLocation {
            properties: SampleProperties {
                track_id: 1,
                decoding_time: 0,
                sample_duration: 1,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::default(),
                sample_description_index: 1,
            },
            data_origin,
            offset: 16,
            size: 100,
        }
    }

    #[test]
    fn each_data_origin_is_taken_with_the_data_reference_entry_of_its_sample() {
        let mut imdt = DataEntryImdaBox::default();
        imdt.imda_ref_identifier = 7;
        let entries = [
            DataEntry::Url(DataEntryUrlBox::default()),
            DataEntry::Imda(imdt),
            DataEntry::SeqNumImda(DataEntrySeqNumImdaBox::default()),
            DataEntry::Other(AnyBox::from_raw_bytes(
                DataEntryImdaBox::BOX_TYPE,
                vec![1, 0, 0, 0],
            )),
        ];
        let origins = [
            DataOrigin::File,
            DataOrigin::MovieFragment,
            DataOrigin::IdentifiedMediaData,
        ];
        let at_1016 = Ok(SampleLocation {
            data_origin: DataOrigin::File,
            offset: 1016,
            ..location(DataOrigin::MovieFragment)
        });
        let mismatch = |imda_identifier| {
            Err(Error::DataOriginMismatch {
                sample_description_index: 1,
                imda_identifier,
            })
        };
        let not_read = |imda_identifier| Err(Error::IdentifiedMediaDataNotRead { imda_identifier });
        let outside = Err(Error::SequenceNumberOutsideMovieFragment {
            sample_description_index: 1,
        });
        let not_decoded = Err(Error::DataEntryNotDecoded {
            sample_description_index: 1,
            box_type: DataEntryImdaBox::BOX_TYPE,
        });
        let missing = Err(Error::MovieFragmentPositionMissing);
        let before_any_moof = [
            [
                Ok(location(DataOrigin::File)),
                missing.clone(),
                mismatch(None),
            ],
            [mismatch(Some(7)), missing.clone(), not_read(7)],
            [outside.clone(), outside.clone(), outside],
            [
                not_decoded.clone(),
                not_decoded.clone(),
                not_decoded.clone(),
            ],
        ];
        let after_a_moof = [
            [
                Ok(location(DataOrigin::File)),
                at_1016.clone(),
                mismatch(None),
            ],
            [mismatch(Some(7)), at_1016.clone(), not_read(7)],
            [mismatch(Some(3)), at_1016, not_read(3)],
            [not_decoded.clone(), not_decoded.clone(), not_decoded],
        ];
        for (moof, expected) in [(None, before_any_moof), (Some((1000, 3)), after_a_moof)] {
            let mut data_origins = DataOrigins::new();
            if let Some((moof_offset, sequence_number)) = moof {
                data_origins.handle_movie_fragment(moof_offset, sequence_number);
            }
            let taken = entries
                .iter()
                .map(|entry| {
                    origins.map(|data_origin| {
                        data_origins.handle_location(location(data_origin), entry)
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(taken, expected, "after the moof {moof:?}");
        }
    }

    #[test]
    fn a_refused_sample_past_the_last_file_offset_leaves_the_origins_as_they_were() {
        let same_file = DataEntry::Url(DataEntryUrlBox::default());
        let mut data_origins = DataOrigins::new();
        data_origins.handle_movie_fragment(u64::MAX - 120, 1);
        let mut past = location(DataOrigin::MovieFragment);
        past.size = 105;
        assert_eq!(
            data_origins.handle_location(past, &same_file),
            Err(Error::FileOffsetOverflow {
                data_origin_offset: u64::MAX - 120,
                offset: 16,
                size: 105
            })
        );
        past.offset = u64::MAX;
        assert_eq!(
            data_origins.handle_location(past, &same_file),
            Err(Error::FileOffsetOverflow {
                data_origin_offset: u64::MAX - 120,
                offset: u64::MAX,
                size: 105
            })
        );
        let at = |offset| {
            Ok(SampleLocation {
                data_origin: DataOrigin::File,
                offset,
                ..location(DataOrigin::MovieFragment)
            })
        };
        assert_eq!(
            data_origins.handle_location(location(DataOrigin::MovieFragment), &same_file),
            at(u64::MAX - 104)
        );
    }

    #[test]
    fn the_samples_count_from_the_moof_taken_last() {
        let same_file = DataEntry::Url(DataEntryUrlBox::default());
        let mut data_origins = DataOrigins::new();
        let at = |offset| {
            Ok(SampleLocation {
                data_origin: DataOrigin::File,
                offset,
                ..location(DataOrigin::MovieFragment)
            })
        };
        data_origins.handle_movie_fragment(1000, 1);
        assert_eq!(
            data_origins.handle_location(location(DataOrigin::MovieFragment), &same_file),
            at(1016)
        );
        data_origins.handle_movie_fragment(2000, 2);
        assert_eq!(
            data_origins.handle_location(location(DataOrigin::MovieFragment), &same_file),
            at(2016)
        );
    }
}
