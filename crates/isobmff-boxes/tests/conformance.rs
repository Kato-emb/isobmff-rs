//! Boxes read and written back on the files of MPEG's `FileFormatConformance`, read as GPAC reports them, and written as FFmpeg reads them
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests; `ISOBMFF_FFPROBE` names
//! `ffprobe` where it is not on the path.

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use isobmff_boxes::{
        AudioSampleEntryVariant, ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant,
        CompactSampleSizeBox, DataInformationBox, EditBox, ExtendedTypeBox, FileTypeBox, Free,
        FreeSpaceBox, HandlerBox, IdentifiedMediaDataBox, LanguageCode, Matrix, MediaBox,
        MediaDataBox, MediaHeaderBox, MediaInformationBox, MovieBox, MovieExtendsBox,
        MovieFragmentBox, MovieFragmentRandomAccessBox, MovieHeaderBox, OriginalFileTypeBox,
        ProducerReferenceTimeBox, ProgressiveDownloadInfoBox, RollRecoveryEntry,
        SampleDependencyTypeBox, SampleDescription, SampleDescriptionBox, SampleEntry, SampleFlags,
        SampleGroupDescriptionBox, SampleGroupDescriptionEntry, SampleSizeBox, SampleSizeVariant,
        SampleSizes, SampleTableBox, SampleToChunkBox, SampleToChunkEntry, SampleToGroupBox,
        SampleToGroupEntry, SegmentIndexBox, SegmentIndexReference, SegmentTypeBox, Skip,
        SubsegmentIndexBox, TemporalLevelEntry, TimeToSampleBox, TimeToSampleEntry, TrackBox,
        TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackFragmentHeaderBox,
        TrackHeaderBox, TrackReference, TrackReferenceBox, TrackReferenceTypeBox, TrackRunBox,
        TrackRunSample, TrackRunSamples, VisualRandomAccessEntry, VisualSampleEntry,
    };
    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, FourCc, I8F8, RawBox, U16F16,
        Utf8String,
    };
    use isobmff_test_support::serde_json::{self, Value};
    use isobmff_test_support::{ConformanceFile, conformance_files, ffprobe, gpac_children};

    /// Reads `payload` as a `Box` and returns the payload it writes back
    ///
    /// The payload is compared rather than the whole box: the size form a box
    /// was framed under, such as a size of zero running to the end of the
    /// file, is not a value the box holds.
    fn written_back<Box: BoxDecode + BoxEncode>(payload: &[u8]) -> Result<Vec<u8>, Error> {
        let read = Box::decode_payload(payload)?;
        let mut written = vec![0; usize::try_from(read.payload_len()).unwrap()];
        read.encode_payload(&mut written)?;

        Ok(written)
    }

    /// Reads a payload as one box and writes it back, as [`written_back`] does for a type
    type WrittenBack = fn(&[u8]) -> Result<Vec<u8>, Error>;

    /// Box types that can lie at the top level of a file and have a type here, each with how it is read and written back
    const TOP_LEVEL: [(BoxType, WrittenBack); 12] = [
        (FileTypeBox::BOX_TYPE, written_back::<FileTypeBox>),
        (SegmentTypeBox::BOX_TYPE, written_back::<SegmentTypeBox>),
        (ExtendedTypeBox::BOX_TYPE, written_back::<ExtendedTypeBox>),
        (
            OriginalFileTypeBox::BOX_TYPE,
            written_back::<OriginalFileTypeBox>,
        ),
        (
            FreeSpaceBox::<Free>::BOX_TYPE,
            written_back::<FreeSpaceBox<Free>>,
        ),
        (
            FreeSpaceBox::<Skip>::BOX_TYPE,
            written_back::<FreeSpaceBox<Skip>>,
        ),
        (MediaDataBox::BOX_TYPE, written_back::<MediaDataBox>),
        (
            IdentifiedMediaDataBox::BOX_TYPE,
            written_back::<IdentifiedMediaDataBox>,
        ),
        (
            ProgressiveDownloadInfoBox::BOX_TYPE,
            written_back::<ProgressiveDownloadInfoBox>,
        ),
        (SegmentIndexBox::BOX_TYPE, written_back::<SegmentIndexBox>),
        (
            SubsegmentIndexBox::BOX_TYPE,
            written_back::<SubsegmentIndexBox>,
        ),
        (
            ProducerReferenceTimeBox::BOX_TYPE,
            written_back::<ProducerReferenceTimeBox>,
        ),
    ];

    /// Returns the top-level boxes of a file
    fn top_level(bytes: &[u8]) -> Vec<RawBox<'_>> {
        Boxes::new(bytes).collect::<Result<_, _>>().unwrap()
    }

    /// Reads every box of `boxes` that is a `Box`, in the order they lie
    fn read_all<Box: BoxDecode + BoxDefinition>(boxes: &[RawBox<'_>]) -> Vec<Box> {
        boxes
            .iter()
            .filter(|framed| framed.header().box_type() == Box::BOX_TYPE)
            .map(|framed| Box::decode_payload(framed.payload()).unwrap())
            .collect()
    }

    /// Returns the `GeneralTypeBox` fields GPAC reports for `node`
    fn gpac_general_type(node: &Value) -> (FourCc, u32, Vec<FourCc>) {
        let brand = |node: &Value, field: &str| -> FourCc {
            let spelled = node.get(field).unwrap().as_str().unwrap();
            FourCc::new(spelled.as_bytes().try_into().unwrap())
        };
        let entries = gpac_elements(node, "BrandEntry");
        let minor_version = node.get("@MinorVersion").unwrap().as_str().unwrap();

        (
            brand(node, "@MajorBrand"),
            minor_version.parse().unwrap(),
            entries
                .into_iter()
                .map(|entry| brand(entry, "@AlternateBrand"))
                .collect(),
        )
    }

    /// Returns the top-level boxes GPAC reports of the four-character code `box_type`
    fn gpac_top_level<'report>(report: &'report Value, box_type: &str) -> Vec<&'report Value> {
        gpac_boxes(report.get("IsoMediaFile").unwrap(), box_type)
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn every_top_level_box_with_a_type_writes_back_the_payload_it_was_read_from() {
        let mut differences = Vec::new();
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            for framed in top_level(&bytes) {
                let box_type = framed.header().box_type();
                let Some((_, written_back)) =
                    TOP_LEVEL.iter().find(|(known, _)| *known == box_type)
                else {
                    continue;
                };
                match written_back(framed.payload()) {
                    Ok(written) if written == framed.payload() => {}
                    Ok(_) => {
                        differences.push(format!("{name}: {box_type} writes back other bytes"))
                    }
                    Err(error) => differences.push(format!("{name}: {box_type}: {error}")),
                }
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Returns the sample entries of the tracks of `bytes` whose handler type is `handler_type`, each with its `stsd` as GPAC reports it
    fn sample_entries_of<'report>(
        bytes: &[u8],
        report: &'report Value,
        handler_type: &[u8; 4],
    ) -> Vec<(SampleEntry, &'report Value)> {
        read_all::<MovieBox>(&top_level(bytes))
            .into_iter()
            .flat_map(|movie| movie.trak)
            .zip(
                gpac_top_level(report, "moov")
                    .into_iter()
                    .flat_map(|movie| gpac_boxes(movie, "trak")),
            )
            .filter(|(track, _)| track.mdia.hdlr.handler_type == FourCc::new(*handler_type))
            .flat_map(|(track, reported)| {
                let media = gpac_box(reported, "mdia");
                let sample_table = gpac_box(gpac_box(media, "minf"), "stbl");
                track
                    .mdia
                    .minf
                    .stbl
                    .stsd
                    .sample_descriptions
                    .into_iter()
                    .zip(gpac_children(gpac_box(sample_table, "stsd")))
                    .filter_map(|(entry, reported)| {
                        if let SampleDescription::Typed(entry) = entry {
                            Some((entry, reported))
                        } else {
                            None
                        }
                    })
            })
            .collect()
    }

    /// Returns `ours` with the fields GPAC reports for a visual sample entry set as GPAC reports them in `node`
    fn gpac_visual_sample_entry(ours: &VisualSampleEntry, node: &Value) -> VisualSampleEntry {
        let mut visual = ours.clone();
        visual.width = gpac_integer(node, "@Width");
        visual.height = gpac_integer(node, "@Height");
        visual.horiz_resolution = U16F16::from_raw(gpac_integer(node, "@XDPI"));
        visual.vert_resolution = U16F16::from_raw(gpac_integer(node, "@YDPI"));
        visual.depth = gpac_integer(node, "@BitDepth");
        visual
    }

    /// Reads every sample entry of the tracks of the handler type `handler_type` into `Class`, holds it against what GPAC reports, as `gpac` sets it, and writes it back, and returns what differs
    ///
    /// An entry written back holds the boxes it was read from after the
    /// `fields_len` bytes of the fields of `Class`.
    fn sample_entries_read_as_gpac_reports_them_and_write_back<Class>(
        handler_type: &[u8; 4],
        fields_len: usize,
        gpac: fn(&Class, &Value) -> Class,
    ) -> Vec<String>
    where
        Class:
            for<'entry> TryFrom<&'entry SampleEntry, Error = Error> + PartialEq + std::fmt::Debug,
        SampleEntry: for<'class> TryFrom<&'class Class, Error = Error>,
    {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            for (entry, reported) in sample_entries_of(&bytes, &report, handler_type) {
                let read = match Class::try_from(&entry) {
                    Ok(read) => read,
                    Err(error) => {
                        differences.push(format!("{name}: {:?}: {error}", entry.format));
                        continue;
                    }
                };
                let expected = gpac(&read, reported);
                if read != expected {
                    differences.push(format!("{name}: read {read:?}, GPAC reports {expected:?}"));
                }
                match SampleEntry::try_from(&read) {
                    Ok(written)
                        if same_boxes_after(&written, &entry, fields_len)
                            && Class::try_from(&written).as_ref() == Ok(&read) => {}
                    Ok(_) => differences.push(format!(
                        "{name}: {:?} writes back other bytes",
                        entry.format
                    )),
                    Err(error) => differences.push(format!("{name}: {:?}: {error}", entry.format)),
                }
            }
        }

        differences
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_sample_entries_of_video_tracks_read_as_gpac_reports_them_and_write_back() {
        assert_eq!(
            sample_entries_read_as_gpac_reports_them_and_write_back(
                b"vide",
                70,
                gpac_visual_sample_entry
            ),
            Vec::<String>::new()
        );
    }

    /// Returns `ours` with the fields GPAC reports for an audio sample entry set as GPAC reports them in `node`
    ///
    /// GPAC reports the integer part of `samplerate`, under `@SampleRate`, or
    /// `@Samplerate` with `@ChannelCount` for an entry it reads as QuickTime
    /// does, such as `agsm`.
    fn gpac_audio_sample_entry(
        ours: &AudioSampleEntryVariant,
        node: &Value,
    ) -> AudioSampleEntryVariant {
        let field = |names: [&'static str; 2]| -> &'static str {
            names
                .into_iter()
                .find(|name| node.get(name).is_some())
                .unwrap()
        };
        let channel_count = gpac_integer(node, field(["@Channels", "@ChannelCount"]));
        let sample_size = gpac_integer(node, "@BitsPerSample");
        let sample_rate_integer: u32 = gpac_integer(node, field(["@SampleRate", "@Samplerate"]));
        let sample_rate =
            |ours: U16F16| U16F16::from_raw((sample_rate_integer << 16) | (ours.raw() & 0xffff));
        let mut audio = ours.clone();
        if let AudioSampleEntryVariant::AudioSampleEntry(audio) = &mut audio {
            audio.channel_count = channel_count;
            audio.sample_size = sample_size;
            audio.sample_rate = sample_rate(audio.sample_rate);
        } else if let AudioSampleEntryVariant::AudioSampleEntryV1(audio) = &mut audio {
            audio.channel_count = channel_count;
            audio.sample_size = sample_size;
            audio.sample_rate = sample_rate(audio.sample_rate);
        }
        audio
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_sample_entries_of_audio_tracks_read_as_gpac_reports_them_and_write_back() {
        assert_eq!(
            sample_entries_read_as_gpac_reports_them_and_write_back(
                b"soun",
                20,
                gpac_audio_sample_entry
            ),
            Vec::<String>::new()
        );
    }

    /// Returns whether `written` holds the format and `data_reference_index` `original` holds, and after the `fields_len` bytes of its other fields the boxes `original` holds, in any order
    ///
    /// The other fields are not compared as bytes: a field the specification
    /// fixes is written as it states it, whatever it was read as, such as the
    /// last `pre_defined` of the `resv` of `restricted.mp4`, `0x00ff` for -1.
    fn same_boxes_after(written: &SampleEntry, original: &SampleEntry, fields_len: usize) -> bool {
        let mut unmatched = top_level(written.remaining_data.get(fields_len..).unwrap_or_default());
        (written.format, written.data_reference_index)
            == (original.format, original.data_reference_index)
            && top_level(
                original
                    .remaining_data
                    .get(fields_len..)
                    .unwrap_or_default(),
            )
            .into_iter()
            .all(|child| {
                unmatched
                    .iter()
                    .position(|candidate| {
                        candidate.header().box_type() == child.header().box_type()
                            && candidate.payload() == child.payload()
                    })
                    .map(|position| unmatched.remove(position))
                    .is_some()
            })
            && unmatched.is_empty()
    }

    /// Returns `stsd` with the format and `data_reference_index` of each entry set as GPAC reports them in `node`
    ///
    /// GPAC reports no `data_reference_index` for some formats, such as
    /// `wvtt`, and spells it `@dataReferenceIndex` for others, such as `tx3g`.
    fn gpac_sample_descriptions(ours: &SampleDescriptionBox, node: &Value) -> SampleDescriptionBox {
        let mut sample_descriptions = ours.clone();
        sample_descriptions.sample_descriptions = gpac_children(node)
            .into_iter()
            .enumerate()
            .map(|(index, reported)| {
                let mut entry = match ours.sample_descriptions.get(index) {
                    Some(SampleDescription::Typed(entry)) => entry.clone(),
                    _ => SampleEntry::default(),
                };
                entry.format =
                    FourCc::new(gpac_str(reported, "@Type").as_bytes().try_into().unwrap());
                if let Some(field) = ["@DataReferenceIndex", "@dataReferenceIndex"]
                    .into_iter()
                    .find(|field| reported.get(field).is_some())
                {
                    entry.data_reference_index = gpac_integer(reported, field);
                }
                SampleDescription::Typed(entry)
            })
            .collect();
        sample_descriptions
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_sample_descriptions_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let ours: Vec<SampleDescriptionBox> = read_all::<MovieBox>(&top_level(&bytes))
                .into_iter()
                .flat_map(|movie| movie.trak)
                .map(|track| track.mdia.minf.stbl.stsd)
                .collect();
            let theirs: Vec<SampleDescriptionBox> = gpac_top_level(&report, "moov")
                .into_iter()
                .flat_map(|movie| gpac_boxes(movie, "trak"))
                .zip(&ours)
                .map(|(track, ours)| {
                    let media = gpac_box(track, "mdia");
                    let sample_table = gpac_box(gpac_box(media, "minf"), "stbl");
                    gpac_sample_descriptions(ours, gpac_box(sample_table, "stsd"))
                })
                .collect();
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn ftyp_and_styp_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let boxes = top_level(&bytes);
            let ours = (
                read_all::<FileTypeBox>(&boxes),
                read_all::<SegmentTypeBox>(&boxes),
            );
            let theirs = (
                gpac_top_level(&report, "ftyp")
                    .into_iter()
                    .map(|node| {
                        let mut file_type = FileTypeBox::default();
                        (
                            file_type.major_brand,
                            file_type.minor_version,
                            file_type.compatible_brands,
                        ) = gpac_general_type(node);
                        file_type
                    })
                    .collect(),
                gpac_top_level(&report, "styp")
                    .into_iter()
                    .map(|node| {
                        let mut segment_type = SegmentTypeBox::default();
                        (
                            segment_type.major_brand,
                            segment_type.minor_version,
                            segment_type.compatible_brands,
                        ) = gpac_general_type(node);
                        segment_type
                    })
                    .collect(),
            );
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Box types of the containers a written-back container may hold its children in another order in
    const REORDERED: [BoxType; 11] = [
        MovieBox::BOX_TYPE,
        TrackBox::BOX_TYPE,
        MediaBox::BOX_TYPE,
        MediaInformationBox::BOX_TYPE,
        SampleTableBox::BOX_TYPE,
        DataInformationBox::BOX_TYPE,
        EditBox::BOX_TYPE,
        MovieExtendsBox::BOX_TYPE,
        MovieFragmentBox::BOX_TYPE,
        TrackFragmentBox::BOX_TYPE,
        MovieFragmentRandomAccessBox::BOX_TYPE,
    ];

    /// Box types of the top-level containers, each with how it is read and written back
    const CONTAINERS: [(BoxType, WrittenBack); 3] = [
        (MovieBox::BOX_TYPE, written_back::<MovieBox>),
        (MovieFragmentBox::BOX_TYPE, written_back::<MovieFragmentBox>),
        (
            MovieFragmentRandomAccessBox::BOX_TYPE,
            written_back::<MovieFragmentRandomAccessBox>,
        ),
    ];

    /// Returns whether `written` holds the boxes `original` holds, those inside the containers of [`REORDERED`] in any order
    ///
    /// An `sdtp` is the same box if it is written with the `flags` of 0 the
    /// specification fixes, whatever it was read with.
    fn same_boxes(original: &[u8], written: &[u8]) -> bool {
        let mut unmatched = top_level(written);
        top_level(original).into_iter().all(|child| {
            let box_type = child.header().box_type();
            let position = unmatched.iter().position(|candidate| {
                candidate.header().box_type() == box_type
                    && if REORDERED.contains(&box_type) {
                        same_boxes(child.payload(), candidate.payload())
                    } else if box_type == SampleDependencyTypeBox::BOX_TYPE {
                        // Why not compare the bytes whole: `08_bifs_carousel_v2.mp4`
                        // holds an `sdtp` of `flags` 1, which is written as 0.
                        let (original, written) = (child.payload(), candidate.payload());
                        original.first() == written.first()
                            && written.get(1..4) == Some([0, 0, 0].as_slice())
                            && original.get(4..) == written.get(4..)
                    } else {
                        candidate.payload() == child.payload()
                    }
            });
            position
                .map(|position| unmatched.remove(position))
                .is_some()
        }) && unmatched.is_empty()
    }

    /// Returns the file `bytes` with each of its top-level boxes that has a type here read and written back
    fn with_boxes_written_back(bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let mut written = Vec::with_capacity(bytes.len());
        for framed in top_level(bytes) {
            let box_type = framed.header().box_type();
            let mut header = vec![0; usize::try_from(framed.header().encoded_len()).unwrap()];
            framed.header().encode(&mut header)?;
            written.extend(header);
            match CONTAINERS
                .iter()
                .chain(&TOP_LEVEL)
                .find(|(known, _)| *known == box_type)
            {
                Some((_, written_back)) => written.extend(written_back(framed.payload())?),
                None => written.extend(framed.payload()),
            }
        }

        Ok(written)
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn every_top_level_container_writes_back_every_box_it_was_read_from() {
        let mut differences = Vec::new();
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            for framed in top_level(&bytes) {
                let box_type = framed.header().box_type();
                let Some((_, written_back)) =
                    CONTAINERS.iter().find(|(known, _)| *known == box_type)
                else {
                    continue;
                };
                match written_back(framed.payload()) {
                    Ok(written) if same_boxes(framed.payload(), &written) => {}
                    Ok(_) => {
                        differences.push(format!("{name}: {box_type} writes back other boxes"))
                    }
                    Err(error) => differences.push(format!("{name}: {box_type}: {error}")),
                }
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Returns the text GPAC reports for `field` of `node`
    fn gpac_str<'report>(node: &'report Value, field: &str) -> &'report str {
        node.get(field).unwrap().as_str().unwrap()
    }

    /// Returns the integer GPAC reports for `field` of `node`
    fn gpac_integer<Integer: std::str::FromStr>(node: &Value, field: &str) -> Integer {
        node.get(field)
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .ok()
            .unwrap()
    }

    /// Returns `ours` if GPAC's two-decimal rendering of it, a fixed-point number of `fraction_bits`, is `spelled`, and otherwise the number `spelled` denotes
    fn gpac_fixed_point(ours: u32, fraction_bits: u32, spelled: &str) -> u32 {
        let one = 1_u64.checked_shl(fraction_bits).unwrap();
        let hundredths = u64::from(ours)
            .checked_mul(100)
            .and_then(|scaled| scaled.checked_add(one / 2))
            .and_then(|scaled| scaled.checked_div(one))
            .unwrap();
        let (integer, fraction) = spelled.split_once('.').unwrap();
        let reported = integer
            .parse::<u64>()
            .unwrap()
            .checked_mul(100)
            .and_then(|scaled| scaled.checked_add(fraction.parse().unwrap()))
            .unwrap();
        if hundredths == reported {
            ours
        } else {
            u32::try_from(
                reported
                    .checked_mul(one)
                    .and_then(|scaled| scaled.checked_div(100))
                    .unwrap(),
            )
            .unwrap()
        }
    }

    /// Returns the boxes GPAC reports directly inside `node` of the four-character code `box_type`
    fn gpac_boxes<'report>(node: &'report Value, box_type: &str) -> Vec<&'report Value> {
        gpac_children(node)
            .into_iter()
            .filter(|child| child.get("@Type").and_then(Value::as_str) == Some(box_type))
            .collect()
    }

    /// Returns the first box GPAC reports directly inside `node` of the four-character code `box_type`
    fn gpac_box<'report>(node: &'report Value, box_type: &str) -> &'report Value {
        gpac_boxes(node, box_type).into_iter().next().unwrap()
    }

    /// Returns `mvhd` with the fields GPAC reports for it set as GPAC reports them
    fn gpac_movie_header(ours: &MovieHeaderBox, node: &Value) -> MovieHeaderBox {
        let mut movie_header = ours.clone();
        movie_header.version = gpac_integer(node, "@Version");
        movie_header.creation_time = gpac_integer(node, "@CreationTime");
        movie_header.modification_time = gpac_integer(node, "@ModificationTime");
        movie_header.timescale = gpac_integer(node, "@TimeScale");
        movie_header.duration = gpac_integer(node, "@Duration");
        movie_header.next_track_id = gpac_integer(node, "@NextTrackID");
        movie_header
    }

    /// Returns `tkhd` with the fields GPAC reports for it set as GPAC reports them
    ///
    /// GPAC reports `volume` where it is not zero, and otherwise `width` and
    /// `height` where either is not zero; the matrix it reports where `width`
    /// or `height` is not zero.
    fn gpac_track_header(ours: &TrackHeaderBox, node: &Value) -> TrackHeaderBox {
        let flags: u32 = gpac_integer(node, "@Flags");
        let mut track_header = ours.clone();
        track_header.version = gpac_integer(node, "@Version");
        track_header.track_enabled = flags & 0x01 != 0;
        track_header.track_in_movie = flags & 0x02 != 0;
        track_header.track_in_preview = flags & 0x04 != 0;
        track_header.track_size_is_aspect_ratio = flags & 0x08 != 0;
        track_header.creation_time = gpac_integer(node, "@CreationTime");
        track_header.modification_time = gpac_integer(node, "@ModificationTime");
        track_header.track_id = gpac_integer(node, "@TrackID");
        track_header.duration = gpac_integer(node, "@Duration");
        if let Some(volume) = node.get("@Volume").and_then(Value::as_str) {
            let raw = u32::from(ours.volume.raw().cast_unsigned());
            let reported = gpac_fixed_point(raw, 8, volume);
            track_header.volume = I8F8::from_raw(u16::try_from(reported).unwrap().cast_signed());
        }
        if let (Some(width), Some(height)) = (
            node.get("@Width").and_then(Value::as_str),
            node.get("@Height").and_then(Value::as_str),
        ) {
            track_header.width = U16F16::from_raw(gpac_fixed_point(ours.width.raw(), 16, width));
            track_header.height = U16F16::from_raw(gpac_fixed_point(ours.height.raw(), 16, height));
        }
        if let Some(matrix) = node.get("Matrix") {
            let values = [
                "m11", "m12", "m13", "m21", "m22", "m23", "m31", "m32", "m33",
            ]
            .map(|field| {
                let spelled = gpac_str(matrix, &format!("@{field}"));
                u32::from_str_radix(spelled.trim_start_matches("0x"), 16)
                    .unwrap()
                    .cast_signed()
            });
            track_header.matrix = Matrix::from_raw(values);
        }
        track_header
    }

    /// Returns `mdhd` with the fields GPAC reports for it set as GPAC reports them
    ///
    /// GPAC reports a `language` below `0x400`, a Macintosh language code of
    /// QuickTime, as the ISO 639-2/T code its table maps it to, so such a
    /// `language` is left as read.
    fn gpac_media_header(ours: &MediaHeaderBox, node: &Value) -> MediaHeaderBox {
        let language = gpac_str(node, "@LanguageCode");
        let mut media_header = ours.clone();
        media_header.version = gpac_integer(node, "@Version");
        media_header.creation_time = gpac_integer(node, "@CreationTime");
        media_header.modification_time = gpac_integer(node, "@ModificationTime");
        media_header.timescale = gpac_integer(node, "@TimeScale");
        media_header.duration = gpac_integer(node, "@Duration");
        let [first, ..] = ours.language.letters();
        if first != b'`' && language.as_bytes() != ours.language.letters() {
            media_header.language =
                LanguageCode::new(language.as_bytes().try_into().unwrap()).unwrap_or_default();
        }
        media_header
    }

    /// Returns `hdlr` with the fields GPAC reports for it set as GPAC reports them
    ///
    /// GPAC reports a `name` whose first byte counts the bytes after it
    /// without that byte, as a reader of the Pascal strings QuickTime writes.
    fn gpac_handler(ours: &HandlerBox, node: &Value) -> HandlerBox {
        let name = gpac_str(node, "@Name");
        let bytes = ours.name.as_bytes();
        let unprefixed = match bytes.split_first() {
            Some((&length, rest)) if usize::from(length) == rest.len() => rest,
            _ => bytes,
        };
        let mut handler = ours.clone();
        handler.handler_type =
            FourCc::new(gpac_str(node, "@hdlrType").as_bytes().try_into().unwrap());
        if name.as_bytes() != unprefixed {
            handler.name = Utf8String::from_slice(name.as_bytes());
        }
        handler
    }

    /// Header boxes of a movie: its `mvhd`, and the `tkhd`, `mdhd` and `hdlr` of each track
    type Headers = (
        MovieHeaderBox,
        Vec<(TrackHeaderBox, MediaHeaderBox, HandlerBox)>,
    );

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_headers_of_a_movie_and_its_tracks_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let ours: Vec<Headers> = read_all::<MovieBox>(&top_level(&bytes))
                .into_iter()
                .map(|movie| {
                    let tracks = movie
                        .trak
                        .into_iter()
                        .map(|track| (track.tkhd, track.mdia.mdhd, track.mdia.hdlr))
                        .collect();
                    (movie.mvhd, tracks)
                })
                .collect();
            let theirs: Vec<Headers> = gpac_top_level(&report, "moov")
                .into_iter()
                .zip(&ours)
                .map(|(movie, (movie_header, tracks))| {
                    let gpac_tracks = gpac_boxes(movie, "trak")
                        .into_iter()
                        .zip(tracks)
                        .map(|(track, (track_header, media_header, handler))| {
                            let media = gpac_box(track, "mdia");
                            (
                                gpac_track_header(track_header, gpac_box(track, "tkhd")),
                                gpac_media_header(media_header, gpac_box(media, "mdhd")),
                                gpac_handler(handler, gpac_box(media, "hdlr")),
                            )
                        })
                        .collect();
                    (
                        gpac_movie_header(movie_header, gpac_box(movie, "mvhd")),
                        gpac_tracks,
                    )
                })
                .collect();
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Returns the elements GPAC reports in `node` under `element`, which it reports as an array where there is more than one
    fn gpac_elements<'report>(node: &'report Value, element: &str) -> Vec<&'report Value> {
        match node.get(element) {
            Some(Value::Array(elements)) => elements.iter().collect(),
            Some(single) => vec![single],
            None => Vec::new(),
        }
    }

    /// Returns `stts` as GPAC reports it in `node`
    fn gpac_time_to_sample(node: &Value) -> TimeToSampleBox {
        let mut time_to_sample = TimeToSampleBox::default();
        time_to_sample.entries = gpac_elements(node, "TimeToSampleEntry")
            .into_iter()
            .map(|reported| {
                let mut entry = TimeToSampleEntry::default();
                entry.sample_count = gpac_integer(reported, "@SampleCount");
                entry.sample_delta = gpac_integer(reported, "@SampleDelta");
                entry
            })
            .collect();
        time_to_sample
    }

    /// Returns `stsc` as GPAC reports it in `node`
    fn gpac_sample_to_chunk(node: &Value) -> SampleToChunkBox {
        let mut sample_to_chunk = SampleToChunkBox::default();
        sample_to_chunk.entries = gpac_elements(node, "SampleToChunkEntry")
            .into_iter()
            .map(|reported| {
                let mut entry = SampleToChunkEntry::default();
                entry.first_chunk = gpac_integer(reported, "@FirstChunk");
                entry.samples_per_chunk = gpac_integer(reported, "@SamplesPerChunk");
                entry.sample_description_index = gpac_integer(reported, "@SampleDescriptionIndex");
                entry
            })
            .collect();
        sample_to_chunk
    }

    /// Returns the sample size variant as GPAC reports it in `node`, an `stsz` or an `stz2`
    fn gpac_sample_size(node: &Value) -> SampleSizeVariant {
        let entries = gpac_elements(node, "SampleSizeEntry").into_iter();
        if gpac_str(node, "@Type") == "stz2" {
            let mut compact = CompactSampleSizeBox::default();
            compact.field_size = gpac_integer(node, "@SampleSizeBits");
            compact.entry_sizes = entries
                .map(|reported| gpac_integer(reported, "@Size"))
                .collect();
            return SampleSizeVariant::CompactSampleSize(compact);
        }
        let mut sample_size = SampleSizeBox::default();
        sample_size.sample_sizes = if node.get("@ConstantSampleSize").is_some() {
            SampleSizes::Constant {
                sample_size: gpac_integer(node, "@ConstantSampleSize"),
                sample_count: gpac_integer(node, "@SampleCount"),
            }
        } else {
            SampleSizes::Table {
                entry_sizes: entries
                    .map(|reported| gpac_integer(reported, "@Size"))
                    .collect(),
            }
        };
        SampleSizeVariant::SampleSize(sample_size)
    }

    /// Returns the chunk offset variant as GPAC reports it in `node`, an `stco` or a `co64`
    fn gpac_chunk_offset(node: &Value) -> ChunkOffsetVariant {
        let large = gpac_str(node, "@Type") == "co64";
        let offsets = gpac_elements(
            node,
            if large {
                "ChunkOffsetEntry"
            } else {
                "ChunkEntry"
            },
        )
        .into_iter()
        .map(|reported| gpac_integer::<u64>(reported, "@offset"));
        if large {
            let mut large_offset = ChunkLargeOffsetBox::default();
            large_offset.chunk_offsets = offsets.collect();
            return ChunkOffsetVariant::ChunkLargeOffset(large_offset);
        }
        let mut offset = ChunkOffsetBox::default();
        offset.chunk_offsets = offsets
            .map(|offset| u32::try_from(offset).unwrap())
            .collect();
        ChunkOffsetVariant::ChunkOffset(offset)
    }

    /// Tables of a sample table that locate its samples: `stts`, `stsc`, and the sample size and chunk offset variants
    type SampleLocation = (
        Option<TimeToSampleBox>,
        Option<SampleToChunkBox>,
        Option<SampleSizeVariant>,
        Option<ChunkOffsetVariant>,
    );

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_tables_that_locate_the_samples_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let ours: Vec<SampleLocation> = read_all::<MovieBox>(&top_level(&bytes))
                .into_iter()
                .flat_map(|movie| movie.trak)
                .map(|track| {
                    let sample_table = track.mdia.minf.stbl;
                    (
                        sample_table.stts,
                        sample_table.stsc,
                        sample_table.sample_size,
                        sample_table.chunk_offset,
                    )
                })
                .collect();
            let theirs: Vec<SampleLocation> = gpac_top_level(&report, "moov")
                .into_iter()
                .flat_map(|movie| gpac_boxes(movie, "trak"))
                .map(|track| {
                    let media = gpac_box(track, "mdia");
                    let sample_table = gpac_box(gpac_box(media, "minf"), "stbl");
                    let first_of = |box_types: &[&str]| {
                        box_types.iter().find_map(|box_type| {
                            gpac_boxes(sample_table, box_type).first().copied()
                        })
                    };
                    (
                        first_of(&["stts"]).map(gpac_time_to_sample),
                        first_of(&["stsc"]).map(gpac_sample_to_chunk),
                        first_of(&["stsz", "stz2"]).map(gpac_sample_size),
                        first_of(&["stco", "co64"]).map(gpac_chunk_offset),
                    )
                })
                .collect();
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Returns `tref` as GPAC reports it in `node`
    fn gpac_track_reference(node: &Value) -> TrackReferenceBox {
        let mut track_reference = TrackReferenceBox::default();
        track_reference.track_references = gpac_children(node)
            .into_iter()
            .map(|reported| {
                let mut typed = TrackReferenceTypeBox::default();
                typed.reference_type =
                    FourCc::new(gpac_str(reported, "@Type").as_bytes().try_into().unwrap());
                typed.track_ids = gpac_elements(reported, "TrackReferenceEntry")
                    .into_iter()
                    .map(|entry| gpac_integer(entry, "@TrackID"))
                    .collect();
                TrackReference::Typed(typed)
            })
            .collect();
        track_reference
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_track_references_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let ours: Vec<Option<TrackReferenceBox>> = read_all::<MovieBox>(&top_level(&bytes))
                .into_iter()
                .flat_map(|movie| movie.trak)
                .map(|track| track.tref)
                .collect();
            let theirs: Vec<Option<TrackReferenceBox>> = gpac_top_level(&report, "moov")
                .into_iter()
                .flat_map(|movie| gpac_boxes(movie, "trak"))
                .map(|track| {
                    gpac_boxes(track, "tref")
                        .first()
                        .copied()
                        .map(gpac_track_reference)
                })
                .collect();
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Returns the four-character code GPAC reports for `field` of `node`
    fn gpac_four_cc(node: &Value, field: &str) -> FourCc {
        FourCc::new(gpac_str(node, field).as_bytes().try_into().unwrap())
    }

    /// Returns `sbgp` as GPAC reports it in `node`
    fn gpac_sample_to_group(node: &Value) -> SampleToGroupBox {
        let mut sample_to_group = SampleToGroupBox::default();
        sample_to_group.grouping_type = gpac_four_cc(node, "@grouping_type");
        sample_to_group.grouping_type_parameter = node
            .get("@grouping_type_parameter")
            .map(|_| gpac_integer(node, "@grouping_type_parameter"));
        sample_to_group.entries = gpac_elements(node, "SampleGroupBoxEntry")
            .into_iter()
            .map(|reported| {
                let mut entry = SampleToGroupEntry::default();
                entry.sample_count = gpac_integer(reported, "@sample_count");
                entry.group_description_index = gpac_integer(reported, "@group_description_index");
                entry
            })
            .collect();
        sample_to_group
    }

    /// Returns `sgpd` with the fields GPAC reports for it set as GPAC reports them
    ///
    /// GPAC reports the entries of the `grouping_type`s `roll`, `rap ` and
    /// `tele` here; the entries of others are left as read. It reports a
    /// `num_leading_samples` only where `num_leading_samples_known` is set.
    fn gpac_sample_group_description(
        ours: SampleGroupDescriptionBox,
        node: &Value,
    ) -> SampleGroupDescriptionBox {
        let flags: u32 = gpac_integer(node, "@Flags");
        let mut description = ours;
        description.version = gpac_integer(node, "@Version");
        description.static_group_description = flags & 0x1 != 0;
        description.static_mapping = flags & 0x2 != 0;
        description.grouping_type = gpac_four_cc(node, "@grouping_type");
        if node.get("@default_length").is_some() {
            description.default_length = gpac_integer(node, "@default_length");
        }
        let entries = |element: &str| gpac_elements(node, element).into_iter();
        description.entries = match description.grouping_type.as_bytes() {
            b"roll" => entries("RollRecoveryEntry")
                .map(|reported| {
                    let mut entry = RollRecoveryEntry::default();
                    entry.roll_distance = gpac_integer(reported, "@roll_distance");
                    SampleGroupDescriptionEntry::RollRecovery(entry)
                })
                .collect(),
            b"rap " => entries("VisualRandomAccessEntry")
                .map(|reported| {
                    let mut entry = VisualRandomAccessEntry::default();
                    entry.num_leading_samples_known =
                        gpac_str(reported, "@num_leading_samples_known") == "yes";
                    if entry.num_leading_samples_known {
                        entry.num_leading_samples = gpac_integer(reported, "@num_leading_samples");
                    }
                    SampleGroupDescriptionEntry::VisualRandomAccess(entry)
                })
                .collect(),
            b"tele" => entries("TemporalLevelEntry")
                .map(|reported| {
                    let mut entry = TemporalLevelEntry::default();
                    entry.level_independently_decodable =
                        gpac_str(reported, "@level_independently_decodable") == "1";
                    SampleGroupDescriptionEntry::TemporalLevel(entry)
                })
                .collect(),
            _ => return description,
        };
        description
    }

    /// Sample groups of a sample table: its `sbgp`s and its `sgpd`s
    type SampleGroups = (Vec<SampleToGroupBox>, Vec<SampleGroupDescriptionBox>);

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_sample_groups_of_a_sample_table_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let ours: Vec<SampleGroups> = read_all::<MovieBox>(&top_level(&bytes))
                .into_iter()
                .flat_map(|movie| movie.trak)
                .map(|track| {
                    let sample_table = track.mdia.minf.stbl;
                    (sample_table.sbgp, sample_table.sgpd)
                })
                .collect();
            let theirs: Vec<SampleGroups> = gpac_top_level(&report, "moov")
                .into_iter()
                .flat_map(|movie| gpac_boxes(movie, "trak"))
                .zip(&ours)
                .map(|(track, (_, descriptions))| {
                    let media = gpac_box(track, "mdia");
                    let sample_table = gpac_box(gpac_box(media, "minf"), "stbl");
                    (
                        gpac_boxes(sample_table, "sbgp")
                            .into_iter()
                            .map(gpac_sample_to_group)
                            .collect(),
                        gpac_boxes(sample_table, "sgpd")
                            .into_iter()
                            .enumerate()
                            .map(|(index, node)| {
                                let read = descriptions.get(index).cloned().unwrap_or_default();
                                gpac_sample_group_description(read, node)
                            })
                            .collect(),
                    )
                })
                .collect();
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Returns the sample flags GPAC reports in `node`, whose names it gives `prefix` where it reports default or first sample flags and no prefix in a track run row
    fn gpac_sample_flags(node: &Value, prefix: &str) -> SampleFlags {
        let mut flags = SampleFlags::default();
        flags.is_leading = gpac_integer(node, "@IsLeading");
        flags.sample_depends_on = gpac_integer(node, &format!("@{prefix}DependsOn"));
        flags.sample_is_depended_on = gpac_integer(node, &format!("@{prefix}IsDependedOn"));
        flags.sample_has_redundancy = gpac_integer(node, &format!("@{prefix}HasRedundancy"));
        flags.sample_padding_value = gpac_integer(node, "@SamplePadding");
        flags.sample_is_non_sync_sample = gpac_str(node, &format!("@{prefix}Sync")) == "0";
        flags.sample_degradation_priority =
            gpac_integer(node, &format!("@{prefix}DegradationPriority"));
        flags
    }

    /// Returns `tfhd` as GPAC reports it in `node`
    fn gpac_track_fragment_header(node: &Value) -> TrackFragmentHeaderBox {
        let tf_flags: u32 = gpac_integer(node, "@Flags");
        let field =
            |mask: u32, name: &str| (tf_flags & mask != 0).then(|| gpac_integer(node, name));
        let mut header = TrackFragmentHeaderBox::default();
        header.duration_is_empty = tf_flags & 0x01_0000 != 0;
        header.default_base_is_moof = tf_flags & 0x02_0000 != 0;
        header.track_id = gpac_integer(node, "@TrackID");
        header.base_data_offset =
            (tf_flags & 0x00_0001 != 0).then(|| gpac_integer(node, "@BaseDataOffset"));
        header.sample_description_index = field(0x00_0002, "@SampleDescriptionIndex");
        header.default_sample_duration = field(0x00_0008, "@SampleDuration");
        header.default_sample_size = field(0x00_0010, "@SampleSize");
        header.default_sample_flags = (tf_flags & 0x00_0020 != 0)
            .then(|| gpac_sample_flags(node.get("DefaultSampleFlags").unwrap(), "Sample"));
        header
    }

    /// Returns `trun` as GPAC reports it in `node`
    fn gpac_track_run(node: &Value) -> TrackRunBox {
        let tr_flags: u32 = gpac_integer(node, "@Flags");
        let mut track_run = TrackRunBox::default();
        track_run.version = gpac_integer(node, "@Version");
        track_run.data_offset =
            (tr_flags & 0x00_0001 != 0).then(|| gpac_integer(node, "@DataOffset"));
        track_run.first_sample_flags = (tr_flags & 0x00_0004 != 0)
            .then(|| gpac_sample_flags(node.get("FirstSampleFlags").unwrap(), "Sample"));
        track_run.samples = if tr_flags & 0x00_0f00 == 0 {
            TrackRunSamples::EmptyRows {
                sample_count: gpac_integer(node, "@SampleCount"),
            }
        } else {
            TrackRunSamples::Rows {
                samples: gpac_elements(node, "TrackRunEntry")
                    .into_iter()
                    .map(|reported| {
                        let field = |mask: u32, name: &str| {
                            (tr_flags & mask != 0).then(|| gpac_integer(reported, name))
                        };
                        let mut sample = TrackRunSample::default();
                        sample.sample_duration = field(0x00_0100, "@Duration");
                        sample.sample_size = field(0x00_0200, "@Size");
                        sample.sample_flags =
                            (tr_flags & 0x00_0400 != 0).then(|| gpac_sample_flags(reported, ""));
                        sample.sample_composition_time_offset = (tr_flags & 0x00_0800 != 0)
                            .then(|| gpac_integer(reported, "@CTSOffset"));
                        sample
                    })
                    .collect(),
            }
        };
        track_run
    }

    /// Returns `tfdt` as GPAC reports it in `node`
    fn gpac_decode_time(node: &Value) -> TrackFragmentBaseMediaDecodeTimeBox {
        let mut decode_time = TrackFragmentBaseMediaDecodeTimeBox::default();
        decode_time.version = gpac_integer(node, "@Version");
        decode_time.base_media_decode_time = gpac_integer(node, "@baseMediaDecodeTime");
        decode_time
    }

    /// Returns `sidx` as GPAC reports it in `node`
    fn gpac_segment_index(node: &Value) -> SegmentIndexBox {
        let mut segment_index = SegmentIndexBox::default();
        segment_index.version = gpac_integer(node, "@Version");
        segment_index.reference_id = gpac_integer(node, "@reference_ID");
        segment_index.timescale = gpac_integer(node, "@timescale");
        segment_index.earliest_presentation_time =
            gpac_integer(node, "@earliest_presentation_time");
        segment_index.first_offset = gpac_integer(node, "@first_offset");
        segment_index.references = gpac_elements(node, "Reference")
            .into_iter()
            .map(|reported| {
                let mut reference = SegmentIndexReference::default();
                reference.reference_type = gpac_str(reported, "@type") == "1";
                reference.referenced_size = gpac_integer(reported, "@size");
                reference.subsegment_duration = gpac_integer(reported, "@duration");
                reference.starts_with_sap = gpac_str(reported, "@startsWithSAP") == "1";
                reference.sap_type = gpac_integer(reported, "@SAP_type");
                reference.sap_delta_time = gpac_integer(reported, "@SAPDeltaTime");
                reference
            })
            .collect();
        segment_index
    }

    /// Boxes of a track fragment that state its samples: its `tfhd`, its `tfdt` and its track runs
    type TrackFragmentSamples = (
        TrackFragmentHeaderBox,
        Option<TrackFragmentBaseMediaDecodeTimeBox>,
        Vec<TrackRunBox>,
    );

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_track_fragments_and_segment_indexes_read_as_gpac_reports_them() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let boxes = top_level(&bytes);
            let ours: (Vec<TrackFragmentSamples>, Vec<SegmentIndexBox>) = (
                read_all::<MovieFragmentBox>(&boxes)
                    .into_iter()
                    .flat_map(|fragment| fragment.traf)
                    .map(|track_fragment| {
                        (
                            track_fragment.tfhd,
                            track_fragment.tfdt,
                            track_fragment.trun,
                        )
                    })
                    .collect(),
                read_all::<SegmentIndexBox>(&boxes),
            );
            let theirs: (Vec<TrackFragmentSamples>, Vec<SegmentIndexBox>) = (
                gpac_top_level(&report, "moof")
                    .into_iter()
                    .flat_map(|fragment| gpac_boxes(fragment, "traf"))
                    .map(|track_fragment| {
                        (
                            gpac_track_fragment_header(gpac_box(track_fragment, "tfhd")),
                            gpac_boxes(track_fragment, "tfdt")
                                .first()
                                .copied()
                                .map(gpac_decode_time),
                            gpac_boxes(track_fragment, "trun")
                                .into_iter()
                                .map(gpac_track_run)
                                .collect(),
                        )
                    })
                    .collect(),
                gpac_top_level(&report, "sidx")
                    .into_iter()
                    .map(gpac_segment_index)
                    .collect(),
            );
            if ours != theirs {
                differences.push(format!("{name}: read {ours:?}, GPAC reports {theirs:?}"));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    /// Returns what `ffprobe` reads off `bytes`: the time base, duration and number of samples of each stream, and the duration of the file
    fn ffprobe_streams(name: &str, bytes: &[u8]) -> Value {
        let probed = env::temp_dir().join(format!("isobmff-boxes-conformance-{name}"));
        fs::write(&probed, bytes).unwrap();
        let output = ffprobe(
            &[
                "-v",
                "error",
                "-print_format",
                "json",
                "-count_packets",
                "-show_entries",
                "stream=index,time_base,duration_ts,nb_read_packets:format=duration",
            ],
            &probed,
        );
        fs::remove_file(&probed).unwrap();

        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// Files that point into their `moov` by offsets from the start of the file, which a `moov` written back in another order moves
    ///
    /// The `saio` of this file points at the `senc` inside its `trak`, and the
    /// `trak` moves when the `iods` before it is written back after it.
    const OFFSETS_INTO_MOOV: [&str; 1] = ["18_pssh_v2.mp4"];

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn a_file_with_its_boxes_written_back_reads_in_ffmpeg_as_the_file_it_was_read_from() {
        let mut differences = Vec::new();
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            if OFFSETS_INTO_MOOV.contains(&name.as_str()) {
                continue;
            }
            let written = match with_boxes_written_back(&bytes) {
                Ok(written) => written,
                Err(error) => {
                    differences.push(format!("{name}: {error}"));
                    continue;
                }
            };
            let (read_back, expected) = (
                ffprobe_streams(&name, &written),
                ffprobe_streams(&name, &bytes),
            );
            if read_back != expected {
                differences.push(format!(
                    "{name}: FFmpeg reads {read_back} from what was written, {expected} from the file"
                ));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }
}
