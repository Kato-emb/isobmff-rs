//! Framing and box codecs checked against other implementations, on the files of MPEG's `FileFormatConformance`
//!
//! Reading is checked against the box structure and field values GPAC reports
//! for each file, and writing against what FFmpeg's `ffprobe` reads back.
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests; `ISOBMFF_FFPROBE` names
//! `ffprobe` where it is not on the path.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::{env, fs};

    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, BoxHeader, BoxType, Boxes, ChildBoxes, Error,
        FieldReader, FieldWidth, FieldWriter, FourCc, FullBoxFlags, FullBoxHeader, RawBox,
    };
    use isobmff_test_support::serde_json::{self, Value};
    use isobmff_test_support::{
        ConformanceFile, conformance_files, ffprobe, gpac_children, gpac_frame, gpac_type,
    };

    /// Box types whose payload is a sequence of boxes and nothing else
    const CONTAINERS: [[u8; 4]; 12] = [
        *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"dinf", *b"edts", *b"mvex", *b"moof",
        *b"traf", *b"mfra", *b"udta",
    ];

    /// Boxes a file holds that GPAC leaves out of the structure it reports, by file, container path, type and size
    ///
    /// GPAC reports one `strk` of the two the `udta` of this file holds, whose
    /// sizes add up to the `udta` with the one GPAC leaves out.
    const GPAC_OMISSIONS: [(&str, &str, &str, u64); 1] =
        [("sg-tl-st.mp4", "/moov/trak/udta", "strk", 58)];

    /// Type and total of a box, as GPAC spells them
    type Frame = (String, u64);

    /// Returns how many of each box `frames` holds
    fn tally(frames: impl IntoIterator<Item = Frame>) -> BTreeMap<Frame, usize> {
        frames
            .into_iter()
            .fold(BTreeMap::new(), |mut tally, frame| {
                let count = tally.entry(frame).or_insert(0_usize);
                *count = count.saturating_add(1);
                tally
            })
    }

    /// Records where the boxes in `payload` differ from what GPAC reports in `node`, then descends into containers
    fn compare(
        file: &str,
        path: &str,
        payload: &[u8],
        node: &Value,
        differences: &mut Vec<String>,
    ) {
        let framed = match Boxes::new(payload).collect::<Result<Vec<RawBox<'_>>, _>>() {
            Ok(framed) => framed,
            Err(error) => {
                differences.push(format!("{file}{path}: {error}"));
                return;
            }
        };

        let total_of = |child: &RawBox<'_>| {
            u64::try_from(child.payload().len())
                .ok()
                .and_then(|payload_len| payload_len.checked_add(child.header().encoded_len()))
                .unwrap_or(u64::MAX)
        };
        let ours = tally(framed.iter().map(|child| {
            (
                gpac_type(child.header().box_type().four_cc().as_bytes()),
                total_of(child),
            )
        }));
        let omitted = GPAC_OMISSIONS
            .iter()
            .filter(|(omitted_file, omitted_path, _, _)| {
                *omitted_file == file && *omitted_path == path
            })
            .map(|(_, _, box_type, size)| ((*box_type).to_owned(), *size));
        let gpac_nodes = gpac_children(node);
        let theirs = tally(
            gpac_nodes
                .iter()
                .map(|child| gpac_frame(child))
                .chain(omitted),
        );
        if ours != theirs {
            differences.push(format!(
                "{file}{path}: framed {ours:?}, GPAC reports {theirs:?}"
            ));
        }

        let mut unmatched = gpac_nodes;
        for child in &framed {
            let box_type = child.header().box_type();
            if !CONTAINERS.contains(box_type.four_cc().as_bytes()) {
                continue;
            }

            let frame = (gpac_type(box_type.four_cc().as_bytes()), total_of(child));
            if let Some(position) = unmatched.iter().position(|node| gpac_frame(node) == frame) {
                let node = unmatched.remove(position);
                compare(
                    file,
                    &format!("{path}/{box_type}"),
                    child.payload(),
                    node,
                    differences,
                );
            }
        }
    }

    /// [`FileTypeBox`] (`ftyp`), ISO/IEC 14496-12 §5.2, written on this crate's public items alone
    #[derive(PartialEq, Eq, Debug)]
    struct FileTypeBox {
        major_brand: FourCc,
        minor_version: u32,
        compatible_brands: Vec<FourCc>,
    }

    impl BoxDefinition for FileTypeBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"ftyp");
    }

    impl BoxDecode for FileTypeBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            Ok(Self {
                major_brand: FourCc::new(*reader.read_bytes::<4>()?),
                minor_version: reader.read_u32()?,
                compatible_brands: reader
                    .take_remainder()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|brand| FourCc::new(*brand))
                    .collect(),
            })
        }
    }

    impl BoxEncode for FileTypeBox {
        fn payload_len(&self) -> u64 {
            u64::try_from(self.compatible_brands.len())
                .unwrap()
                .checked_mul(4)
                .and_then(|brands_len| brands_len.checked_add(8))
                .unwrap()
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            writer.write_bytes(self.major_brand.as_bytes())?;
            writer.write_u32(self.minor_version)?;
            for brand in &self.compatible_brands {
                writer.write_bytes(brand.as_bytes())?;
            }

            Ok(())
        }
    }

    /// [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2, written on this crate's public items alone
    ///
    /// The fields from `rate` to `pre_defined` are kept as the bytes they were
    /// read from, so a box writes back what it was read from.
    #[derive(PartialEq, Eq, Debug)]
    struct MovieHeaderBox {
        version: u8,
        creation_time: u64,
        modification_time: u64,
        timescale: u32,
        duration: u64,
        rate_to_pre_defined: [u8; 76],
        next_track_id: u32,
    }

    impl MovieHeaderBox {
        fn width(version: u8) -> Result<FieldWidth, Error> {
            match version {
                0 => Ok(FieldWidth::Compact),
                1 => Ok(FieldWidth::Extended),
                _ => Err(Error::unsupported_version(version)),
            }
        }
    }

    impl BoxDefinition for MovieHeaderBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"mvhd");
    }

    impl BoxDecode for MovieHeaderBox {
        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
            let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
            let width = Self::width(version)?;

            Ok(Self {
                version,
                creation_time: reader.read_unsigned(width)?,
                modification_time: reader.read_unsigned(width)?,
                timescale: reader.read_u32()?,
                duration: reader.read_unsigned(width)?,
                rate_to_pre_defined: *reader.read_bytes::<76>()?,
                next_track_id: reader.read_u32()?,
            })
        }
    }

    impl BoxEncode for MovieHeaderBox {
        fn payload_len(&self) -> u64 {
            if self.version == 0 { 100 } else { 112 }
        }

        fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
            let width = Self::width(self.version)?;
            writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
            writer.write_unsigned(width, self.creation_time)?;
            writer.write_unsigned(width, self.modification_time)?;
            writer.write_u32(self.timescale)?;
            writer.write_unsigned(width, self.duration)?;
            writer.write_bytes(&self.rate_to_pre_defined)?;
            writer.write_u32(self.next_track_id)
        }
    }

    /// Returns the first box of `box_type` in `payload`, as the bytes it was framed from
    fn find(payload: &[u8], box_type: BoxType) -> Option<&[u8]> {
        let mut rest = payload;
        while let Ok((framed, after)) = RawBox::split_first(rest) {
            if framed.header().box_type() == box_type {
                let framed_len = usize::try_from(framed.header().encoded_len())
                    .unwrap()
                    .checked_add(framed.payload().len())
                    .unwrap();
                return rest.get(..framed_len);
            }
            rest = after;
        }

        None
    }

    /// Returns what GPAC reports for the `ftyp` and `mvhd` of a file, in the fields of the boxes above
    fn gpac_headers(report: &Value) -> (FileTypeBox, [u64; 5]) {
        let file = report.get("IsoMediaFile").unwrap();
        let number = |node: &Value, field: &str| -> u64 {
            node.get(field).unwrap().as_str().unwrap().parse().unwrap()
        };
        let brand = |node: &Value, field: &str| -> FourCc {
            let spelled = node.get(field).unwrap().as_str().unwrap();
            FourCc::new(spelled.as_bytes().try_into().unwrap())
        };

        let file_type = file.get("FileTypeBox").unwrap();
        let entries = match file_type.get("BrandEntry") {
            Some(Value::Array(entries)) => entries.iter().collect(),
            Some(entry) => vec![entry],
            None => Vec::new(),
        };
        let file_type = FileTypeBox {
            major_brand: brand(file_type, "@MajorBrand"),
            minor_version: u32::try_from(number(file_type, "@MinorVersion")).unwrap(),
            compatible_brands: entries
                .into_iter()
                .map(|entry| brand(entry, "@AlternateBrand"))
                .collect(),
        };

        let movie_header = file
            .get("MovieBox")
            .and_then(|movie| movie.get("MovieHeaderBox"))
            .unwrap();
        let movie_header = [
            "@Version",
            "@CreationTime",
            "@ModificationTime",
            "@TimeScale",
            "@Duration",
        ]
        .map(|field| number(movie_header, field));

        (file_type, movie_header)
    }

    /// Returns what `ffprobe` reads off `file`: the `ftyp` brands as its format tags, and the `mvhd` timescale
    fn ffprobe_headers(file: &Path) -> (Value, Option<String>) {
        let tags = ffprobe(
            &[
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_entries",
                "format_tags=major_brand,minor_version,compatible_brands",
            ],
            file,
        );
        let trace = ffprobe(&["-v", "trace"], file);
        let timescale = String::from_utf8_lossy(&trace.stderr)
            .lines()
            .find_map(|line| line.split_once("time scale = "))
            .map(|(_, timescale)| timescale.trim().to_owned());
        let tags: Value = serde_json::from_slice(&tags.stdout).unwrap();

        (
            tags.pointer("/format/tags").cloned().unwrap_or_default(),
            timescale,
        )
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn every_box_of_a_conformance_file_frames_as_gpac_reports_it() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            compare(
                &name,
                "",
                &bytes,
                report.get("IsoMediaFile").unwrap(),
                &mut differences,
            );
        }

        assert_eq!(differences, Vec::<String>::new());
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn ftyp_and_mvhd_read_as_gpac_reports_and_write_what_was_read_and_what_ffmpeg_reads() {
        let mut differences = Vec::new();
        for ConformanceFile {
            name,
            bytes,
            report,
            ..
        } in conformance_files()
        {
            let ftyp = find(&bytes, FileTypeBox::BOX_TYPE).unwrap();
            let moov = find(&bytes, BoxType::compact(*b"moov")).unwrap();
            let (_moov_header, moov_payload) = BoxHeader::decode(moov).unwrap();
            let mvhd = find(moov_payload, MovieHeaderBox::BOX_TYPE).unwrap();

            let (file_type, _) = FileTypeBox::decode(ftyp).unwrap();
            let movie_header: MovieHeaderBox = ChildBoxes::decode(moov_payload)
                .unwrap()
                .take_exactly_one()
                .unwrap();
            let read = (
                file_type,
                [
                    u64::from(movie_header.version),
                    movie_header.creation_time,
                    movie_header.modification_time,
                    u64::from(movie_header.timescale),
                    movie_header.duration,
                ],
            );
            if read != gpac_headers(&report) {
                differences.push(format!(
                    "{name}: read {read:?}, GPAC reports {:?}",
                    gpac_headers(&report)
                ));
            }
            let (file_type, _) = read;

            let encoded = |len: u64| vec![0; usize::try_from(len).unwrap()];
            let mut written_ftyp = encoded(file_type.encoded_len());
            file_type.encode(&mut written_ftyp).unwrap();
            let mut written_mvhd = encoded(movie_header.encoded_len());
            movie_header.encode(&mut written_mvhd).unwrap();
            if (written_ftyp.as_slice(), written_mvhd.as_slice()) != (ftyp, mvhd) {
                differences.push(format!("{name}: writes bytes other than it read"));
            }

            let moov_header =
                BoxHeader::with_payload_len(BoxType::compact(*b"moov"), movie_header.encoded_len())
                    .unwrap();
            let mut written_moov_header = encoded(moov_header.encoded_len());
            moov_header.encode(&mut written_moov_header).unwrap();
            let written = [written_ftyp, written_moov_header, written_mvhd].concat();

            let (gpac_file_type, gpac_movie_header) = gpac_headers(&report);
            let brand = |brand: &FourCc| String::from_utf8_lossy(brand.as_bytes()).into_owned();
            let expected = (
                serde_json::json!({
                    "major_brand": brand(&gpac_file_type.major_brand),
                    "minor_version": gpac_file_type.minor_version.to_string(),
                    "compatible_brands": gpac_file_type
                        .compatible_brands
                        .iter()
                        .map(brand)
                        .collect::<String>(),
                }),
                Some(gpac_movie_header[3].to_string()),
            );
            let probed = env::temp_dir().join(format!("isobmff-core-conformance-{name}"));
            fs::write(&probed, &written).unwrap();
            let read_back = ffprobe_headers(&probed);
            fs::remove_file(&probed).unwrap();
            if read_back != expected {
                differences.push(format!(
                    "{name}: FFmpeg reads {read_back:?} from what was written, GPAC reports {expected:?}"
                ));
            }
        }

        assert_eq!(differences, Vec::<String>::new());
    }
}
