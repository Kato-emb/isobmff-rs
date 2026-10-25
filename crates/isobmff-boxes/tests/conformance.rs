//! Boxes read and written back on the files of MPEG's `FileFormatConformance`, read as GPAC reports them, and written as FFmpeg reads them
//!
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests; `ISOBMFF_FFPROBE` names
//! `ffprobe` where it is not on the path.

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use isobmff_boxes::{
        DataInformationBox, EditBox, ExtendedTypeBox, FileTypeBox, Free, FreeSpaceBox, HandlerBox,
        IdentifiedMediaDataBox, LanguageCode, Matrix, MediaBox, MediaDataBox, MediaHeaderBox,
        MediaInformationBox, MovieBox, MovieHeaderBox, OriginalFileTypeBox,
        ProgressiveDownloadInfoBox, SegmentTypeBox, Skip, TrackBox, TrackHeaderBox,
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
    const TOP_LEVEL: [(BoxType, WrittenBack); 9] = [
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
        let entries = match node.get("BrandEntry") {
            Some(Value::Array(entries)) => entries.iter().collect(),
            Some(entry) => vec![entry],
            None => Vec::new(),
        };
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

    /// Box types of the containers a written-back `moov` may hold its children in another order in
    const REORDERED: [BoxType; 6] = [
        MovieBox::BOX_TYPE,
        TrackBox::BOX_TYPE,
        MediaBox::BOX_TYPE,
        MediaInformationBox::BOX_TYPE,
        DataInformationBox::BOX_TYPE,
        EditBox::BOX_TYPE,
    ];

    /// Returns whether `written` holds the boxes `original` holds, those inside the containers of [`REORDERED`] in any order
    fn same_boxes(original: &[u8], written: &[u8]) -> bool {
        let mut unmatched = top_level(written);
        top_level(original).into_iter().all(|child| {
            let box_type = child.header().box_type();
            let position = unmatched.iter().position(|candidate| {
                candidate.header().box_type() == box_type
                    && if REORDERED.contains(&box_type) {
                        same_boxes(child.payload(), candidate.payload())
                    } else {
                        candidate.payload() == child.payload()
                    }
            });
            position
                .map(|position| unmatched.remove(position))
                .is_some()
        }) && unmatched.is_empty()
    }

    /// Returns the file `bytes` with each of its top-level `moov`s read and written back
    fn with_moov_written_back(bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let mut written = Vec::with_capacity(bytes.len());
        for framed in top_level(bytes) {
            if framed.header().box_type() == MovieBox::BOX_TYPE {
                let movie = MovieBox::decode_payload(framed.payload())?;
                let mut box_bytes = vec![0; usize::try_from(movie.encoded_len()).unwrap()];
                movie.encode(&mut box_bytes)?;
                written.extend(box_bytes);
            } else {
                let mut header = vec![0; usize::try_from(framed.header().encoded_len()).unwrap()];
                framed.header().encode(&mut header)?;
                written.extend(header);
                written.extend(framed.payload());
            }
        }

        Ok(written)
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn moov_writes_back_every_box_it_was_read_from() {
        let mut differences = Vec::new();
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            for framed in top_level(&bytes) {
                if framed.header().box_type() != MovieBox::BOX_TYPE {
                    continue;
                }
                match written_back::<MovieBox>(framed.payload()) {
                    Ok(written) if same_boxes(framed.payload(), &written) => {}
                    Ok(_) => differences.push(format!("{name}: moov writes back other boxes")),
                    Err(error) => differences.push(format!("{name}: moov: {error}")),
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

    /// Returns what `ffprobe` reads off `bytes`: the time base and duration of each stream, and the duration of the file
    fn ffprobe_timing(name: &str, bytes: &[u8]) -> Value {
        let probed = env::temp_dir().join(format!("isobmff-boxes-conformance-{name}"));
        fs::write(&probed, bytes).unwrap();
        let output = ffprobe(
            &[
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_entries",
                "stream=index,time_base,duration_ts:format=duration",
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
    fn a_file_with_its_moov_written_back_reads_in_ffmpeg_as_the_file_it_was_read_from() {
        let mut differences = Vec::new();
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            if OFFSETS_INTO_MOOV.contains(&name.as_str()) {
                continue;
            }
            let written = match with_moov_written_back(&bytes) {
                Ok(written) => written,
                Err(error) => {
                    differences.push(format!("{name}: moov: {error}"));
                    continue;
                }
            };
            let (read_back, expected) = (
                ffprobe_timing(&name, &written),
                ffprobe_timing(&name, &bytes),
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
