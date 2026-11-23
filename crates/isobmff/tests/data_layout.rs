//! The data reference entry the sample entry of a sample names, checked against GPAC on the conformance files

#[cfg(test)]
mod tests {
    use isobmff::boxes::{
        DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox,
        DataReferenceBox, MovieBox, SampleDescription, SampleDescriptionBox, SampleEntry,
    };
    use isobmff::structure_data::Error;
    use isobmff::structure_data::data_layout::data_entry;
    use isobmff::{AnyBox, BoxDecode, BoxDefinition, BoxType, Boxes, FourCc, Utf8String};
    use isobmff_test_support::{conformance_files, gpac_children, gpac_list};

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
            let Some(moov) = Boxes::new(&file.bytes)
                .map_while(Result::ok)
                .find(|raw_box| raw_box.header().box_type() == MovieBox::BOX_TYPE)
                .and_then(|moov| MovieBox::decode_payload(moov.payload()).ok())
            else {
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
}
