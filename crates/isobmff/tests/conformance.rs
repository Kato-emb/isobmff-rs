//! Reading and writing of the boxes of a file as its bytes arrive, checked on the files of MPEG's `FileFormatConformance`
//!
//! What the reader frames out of a file cut into pieces is checked against
//! what [`Boxes`] frames out of the file whole, and what the writer writes
//! from the reader's events against the file itself. The samples gathered
//! out of the media data of a file cut into pieces are checked against the
//! bytes their locations name in the file whole, and those in another file
//! against the bytes they name in the file their data reference names. The
//! samples of a file, muxed into another file structure, read back the same
//! through the demuxer and `ffprobe`.
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ops::Range;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::{env, fs, iter, process};

    use isobmff::boxes::{
        DataEntry, FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox,
    };
    use isobmff::framing::{BoxEvent, BoxReader, BoxWriter};
    use isobmff::media_data::MediaDataReader;
    use isobmff::structure_data::SegmentIndex;
    use isobmff::{
        BoxDefinition, BoxHeader, Boxes, ByteRange, DemuxEvent, DemuxLimits, Demuxer,
        FileStructure, FourCc, Muxer, Sample, SampleLocation,
    };
    use isobmff_test_support::serde_json::Value;
    use isobmff_test_support::{ConformanceFile, conformance_files, ffprobe_packets};

    /// Lengths the files are cut into, the last of them longer than any file
    const PIECE_LENS: [usize; 4] = [1, 7, 4096, usize::MAX];

    /// What the caller answers each header with
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Answer {
        Gather,
        Stream,
        Skip,
    }

    /// What a caller saw reading a file
    struct Read {
        /// Header of each box, and where it lies
        boxes: Vec<(BoxHeader, ByteRange)>,
        /// Bytes the caller passed to the reader
        passed_bytes: usize,
        /// The file as a writer wrote it from the events of the boxes gathered or streamed
        written: Vec<u8>,
    }

    /// Returns the `piece_len` bytes of `file` from where `wanted` starts, or as many as there are
    fn piece(file: &[u8], wanted: ByteRange, piece_len: usize) -> &[u8] {
        let rest = file
            .get(usize::try_from(wanted.offset()).unwrap()..)
            .unwrap_or_default();
        rest.get(..piece_len).unwrap_or(rest)
    }

    /// Reads `file` from wherever the reader wants it, `piece_len` bytes at a time, answering every header with `answer`
    fn read(file: &[u8], piece_len: usize, answer: Answer) -> Read {
        let mut reader = BoxReader::new();
        let mut writer = BoxWriter::new();
        let mut boxes = Vec::new();
        let mut passed_bytes = 0_usize;
        let mut written = Vec::new();
        while let Some(wanted) = reader.wanted_input() {
            let input = piece(file, wanted, piece_len);
            if input.is_empty() {
                break;
            }
            passed_bytes = passed_bytes.saturating_add(input.len());
            while let Some(event) = reader.handle_input(wanted.offset(), input).unwrap() {
                if let BoxEvent::Header(header) = event {
                    let byte_range = reader.open_box().unwrap();
                    boxes.push((header, byte_range));
                    match answer {
                        Answer::Gather => reader.gather(u64::MAX).unwrap(),
                        Answer::Stream => reader.stream().unwrap(),
                        Answer::Skip => {
                            reader.skip().unwrap();
                            assert_eq!(
                                reader.wanted_input(),
                                byte_range.length().map(|length| {
                                    ByteRange::new(byte_range.offset().saturating_add(length), None)
                                }),
                                "the reader wants the file offset past the box it skips"
                            );
                        }
                    }
                }
                write_back(&mut writer, &reader, event, answer, &mut written);
            }
        }
        if let Some(event) = reader.finish().unwrap() {
            write_back(&mut writer, &reader, event, answer, &mut written);
        }
        writer.finish().unwrap();
        Read {
            boxes,
            passed_bytes,
            written,
        }
    }

    /// Passes `event` of a box gathered or streamed on to `writer`, and keeps the bytes it returns
    fn write_back(
        writer: &mut BoxWriter,
        reader: &BoxReader,
        event: BoxEvent<'_>,
        answer: Answer,
        written: &mut Vec<u8>,
    ) {
        if answer == Answer::Skip {
            return;
        }
        if let Some(raw_box) = reader.gathered_box() {
            written.extend_from_slice(
                writer
                    .handle_event(BoxEvent::Payload(raw_box.payload()))
                    .unwrap(),
            );
        }
        written.extend_from_slice(writer.handle_event(event).unwrap());
    }

    /// Header of each box [`Boxes`] frames out of the whole of `file`, and where it lies
    fn framed_whole(file: &[u8]) -> Vec<(BoxHeader, ByteRange)> {
        let mut offset = 0_u64;
        Boxes::new(file)
            .map(|framed| {
                let framed = framed.unwrap();
                let header = framed.header();
                let byte_range = ByteRange::new(offset, header.size().total_bytes());
                offset = offset
                    .saturating_add(header.encoded_len())
                    .saturating_add(u64::try_from(framed.payload().len()).unwrap());
                (header, byte_range)
            })
            .collect()
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_boxes_of_a_file_cut_into_pieces_frame_as_the_whole_file_does_and_write_back_to_it() {
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            let expected = framed_whole(&bytes);
            for answer in [Answer::Gather, Answer::Stream] {
                for piece_len in PIECE_LENS {
                    let read = read(&bytes, piece_len, answer);
                    assert_eq!(
                        read.boxes, expected,
                        "{name} in pieces of {piece_len}, {answer:?}"
                    );
                    assert!(
                        read.written == bytes,
                        "{name} in pieces of {piece_len}, {answer:?}, writes back to other bytes"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_payloads_of_skipped_boxes_are_never_wanted() {
        for ConformanceFile { name, bytes, .. } in conformance_files() {
            let expected = framed_whole(&bytes);
            let headers_bytes: u64 = expected
                .iter()
                .map(|(header, _)| header.encoded_len())
                .sum();
            for piece_len in PIECE_LENS {
                let skipped = read(&bytes, piece_len, Answer::Skip);
                assert_eq!(skipped.boxes, expected, "{name} in pieces of {piece_len}");
                if piece_len == 1 {
                    assert_eq!(
                        u64::try_from(skipped.passed_bytes).unwrap(),
                        headers_bytes,
                        "{name} byte by byte passes more than the headers"
                    );
                }
            }
        }
    }

    /// Files whose samples are gathered: a non-fragmented movie file with its `moov` first, a fragmented movie file, and a segment
    const MEDIA_DATA_FILES: [&str; 3] =
        ["01_simple.mp4", "a4-tone-fragmented.mp4", "21_segment.mp4"];

    /// What a caller saw reading a file
    #[derive(Default)]
    struct Gathered {
        /// Locations of the samples the `moov` and the `moofs` declare, in the order the boxes lie
        locations: Vec<SampleLocation>,
        /// Samples the demuxer returned, in the order they came out
        samples: Vec<Sample>,
    }

    impl Gathered {
        /// Keeps the locations `event` carries, if any
        fn see(&mut self, event: DemuxEvent) {
            if let DemuxEvent::Movie { samples, .. } | DemuxEvent::MovieFragment { samples, .. } =
                event
            {
                self.locations.extend(samples);
            }
        }

        /// Takes the samples `demuxer` has gathered
        fn poll(&mut self, demuxer: &mut Demuxer) {
            self.samples.extend(iter::from_fn(|| demuxer.poll_sample()));
        }
    }

    /// Limits that hold every sample of a file
    const HOLD_EVERY_SAMPLE: DemuxLimits = DemuxLimits::new()
        .with_held_location_count(u64::MAX)
        .with_held_bytes(u64::MAX);

    /// Returns a demuxer that holds every sample of a file
    fn demuxer() -> Demuxer {
        Demuxer::with_limits(HOLD_EVERY_SAMPLE)
    }

    /// Reads `file` from its start as a caller that cannot seek, `piece_len` bytes at a time
    fn gather_in_order(file: &[u8], piece_len: usize) -> Gathered {
        let mut demuxer = demuxer();
        let mut gathered = Gathered::default();
        let mut offset = 0_u64;
        for piece in file.chunks(piece_len) {
            while let Some(event) = demuxer.handle_input(offset, piece).unwrap() {
                gathered.see(event);
            }
            gathered.poll(&mut demuxer);
            offset = offset.saturating_add(u64::try_from(piece.len()).unwrap());
        }
        while let Some(event) = demuxer.finish().unwrap() {
            gathered.see(event);
        }
        gathered.poll(&mut demuxer);

        gathered
    }

    /// Reads `file` from wherever the demuxer wants it, `piece_len` bytes at a time and no more than it wants
    fn gather_seeking(file: &[u8], piece_len: usize) -> Gathered {
        let mut demuxer = demuxer();
        let mut gathered = Gathered::default();
        while let Some(wanted) = demuxer.wanted_input() {
            let len = wanted.length().map_or(piece_len, |length| {
                piece_len.min(usize::try_from(length).unwrap())
            });
            let input = piece(file, wanted, len);
            let event = match input {
                [] => demuxer.finish().unwrap(),
                input => demuxer.handle_input(wanted.offset(), input).unwrap(),
            };
            if let Some(event) = event {
                gathered.see(event);
            }
            gathered.poll(&mut demuxer);
        }

        gathered
    }

    /// Returns the sample of each of `locations`, with the bytes its byte range names in `file`
    fn samples_at(file: &[u8], locations: &[SampleLocation]) -> Vec<Sample> {
        locations
            .iter()
            .map(|location| {
                let byte_range = location.byte_range();
                let data = file
                    .get(
                        usize::try_from(byte_range.start).unwrap()
                            ..usize::try_from(byte_range.end).unwrap(),
                    )
                    .unwrap();
                Sample::new(*location.properties(), data.to_vec())
            })
            .collect()
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_samples_gathered_from_a_file_cut_into_pieces_are_the_bytes_their_locations_name() {
        let files: Vec<_> = conformance_files()
            .into_iter()
            .filter(|file| MEDIA_DATA_FILES.contains(&file.name.as_str()))
            .collect();
        assert_eq!(files.len(), MEDIA_DATA_FILES.len());
        for ConformanceFile { name, bytes, .. } in &files {
            for (piece_len, gathered) in PIECE_LENS.into_iter().flat_map(|piece_len| {
                [
                    (piece_len, gather_in_order(bytes, piece_len)),
                    (piece_len, gather_seeking(bytes, piece_len)),
                ]
            }) {
                let expected = samples_at(bytes, &gathered.locations);
                assert!(!expected.is_empty(), "{name} declares no sample");
                assert!(
                    gathered.samples == expected,
                    "{name} in pieces of {piece_len} gathers other samples"
                );
            }
        }
    }

    /// File whose samples are all in another file, which its data reference names relative to it
    const MEDIA_DATA_ELSEWHERE: &str = "a6_tone_multifile.mp4";

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_samples_in_another_file_are_gathered_out_of_the_file_their_data_reference_names() {
        let ConformanceFile {
            bytes,
            path,
            report,
            ..
        } = conformance_files()
            .into_iter()
            .find(|file| file.name == MEDIA_DATA_ELSEWHERE)
            .unwrap();
        let mut demuxer = demuxer();
        let mut elsewhere = BTreeMap::<String, Vec<SampleLocation>>::new();
        while let Some(wanted) = demuxer.wanted_input() {
            let event = match piece(&bytes, wanted, usize::MAX) {
                [] => demuxer.finish(),
                input => demuxer.handle_input(wanted.offset(), input),
            }
            .unwrap();
            if let Some(DemuxEvent::Movie { samples, .. }) = event {
                for location in samples {
                    let named = match demuxer
                        .data_reference(location.data_reference())
                        .map(|data_reference| data_reference.entry())
                    {
                        Some(DataEntry::Url(url)) => {
                            url.location.as_ref().and_then(|named| named.as_str())
                        }
                        _ => None,
                    }
                    .expect("the samples are in another file a url entry names");
                    elsewhere
                        .entry(named.to_owned())
                        .or_default()
                        .push(location);
                }
            }
            assert_eq!(demuxer.poll_sample(), None);
        }
        assert_eq!(elsewhere.keys().collect::<Vec<_>>(), ["./myData.dat"]);

        let stbl = report
            .pointer("/IsoMediaFile/MovieBox/TrackBox/MediaBox/MediaInformationBox/SampleTableBox")
            .unwrap();
        let number = |node: &Value, field: &str| -> u64 {
            node.get(field)
                .and_then(Value::as_str)
                .unwrap()
                .parse()
                .unwrap()
        };
        let sample_sizes: Vec<_> = stbl
            .pointer("/SampleSizeBox/SampleSizeEntry")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|entry| number(entry, "@Size"))
            .collect();
        // Why not resolve a run of stsc entries: GPAC reports the one entry of this movie as an object, and an array would make this a second sample table reader
        let samples_per_chunk = number(
            stbl.pointer("/SampleToChunkBox/SampleToChunkEntry")
                .unwrap(),
            "@SamplesPerChunk",
        );
        let reported: Vec<_> = stbl
            .pointer("/ChunkOffsetBox/ChunkEntry")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|entry| number(entry, "@offset"))
            .zip(sample_sizes.chunks(usize::try_from(samples_per_chunk).unwrap()))
            .flat_map(|(chunk_offset, sample_sizes)| {
                sample_sizes.iter().scan(chunk_offset, |start, size| {
                    let byte_range = *start..start.checked_add(*size).unwrap();
                    *start = byte_range.end;
                    Some(byte_range)
                })
            })
            .collect();

        for (named, locations) in elsewhere {
            let byte_ranges: Vec<_> = locations.iter().map(SampleLocation::byte_range).collect();
            assert_eq!(
                byte_ranges, reported,
                "the samples in {named} lie where GPAC reports the chunks and sample sizes"
            );
            let other = fs::read(path.with_file_name(&named)).unwrap();
            let expected = samples_at(&other, &locations);
            for piece_len in PIECE_LENS {
                let mut media_data = MediaDataReader::new(HOLD_EVERY_SAMPLE.media_data());
                media_data
                    .handle_sample_locations(locations.clone())
                    .unwrap();
                let mut samples = Vec::new();
                while let Some(wanted) = media_data.wanted_input() {
                    let input = piece(&other, wanted, piece_len);
                    assert!(!input.is_empty(), "{named} ends before {wanted:?}");
                    media_data.handle_input(wanted.offset(), input).unwrap();
                    samples.extend(iter::from_fn(|| media_data.poll_sample()));
                }
                media_data.finish().unwrap();
                assert!(
                    samples == expected,
                    "{named} in pieces of {piece_len} gathers other samples"
                );
            }
        }
    }

    /// What a demuxer read of a file: its brands, its movie and its samples, in the order they came out, and its indexes
    #[derive(PartialEq, Debug)]
    struct Demuxed {
        file_type: Option<FileTypeBox>,
        movie: MovieBox,
        samples: Vec<Sample>,
        segment_indexes: Vec<SegmentIndex>,
        movie_fragment_random_access: Option<MovieFragmentRandomAccessBox>,
    }

    /// Reads `file` with a [`Demuxer`] from wherever it wants
    fn demux(file: &[u8]) -> Demuxed {
        let mut demuxer = demuxer();
        let mut samples = Vec::new();
        while let Some(wanted) = demuxer.wanted_input() {
            let input = piece(file, wanted, usize::MAX);
            let event = match input {
                [] => demuxer.finish(),
                input => demuxer.handle_input(wanted.offset(), input),
            };
            samples.extend(iter::from_fn(|| demuxer.poll_sample()));
            event.unwrap();
        }
        Demuxed {
            file_type: demuxer.file_type().cloned(),
            movie: demuxer.movie().cloned().unwrap(),
            samples,
            segment_indexes: demuxer.segment_indexes().to_vec(),
            movie_fragment_random_access: demuxer.movie_fragment_random_access().cloned(),
        }
    }

    /// Samples a muxer writes together, in the order they come out of a demuxer
    const SAMPLES_PER_CALL: usize = 20;

    /// Returns the boxes a muxer laid out as `file_structure` writes of `file_type`, `movie` and `samples`, each whole
    fn mux(
        file_structure: FileStructure,
        file_type: Option<FileTypeBox>,
        movie: MovieBox,
        samples: &[Sample],
    ) -> Vec<u8> {
        mux_runs(
            file_structure,
            file_type,
            movie,
            samples.chunks(SAMPLES_PER_CALL),
        )
    }

    /// Returns the boxes a muxer laid out as `file_structure` writes of `file_type`, `movie` and each of `runs` in one call, each whole
    fn mux_runs<'samples>(
        file_structure: FileStructure,
        file_type: Option<FileTypeBox>,
        movie: MovieBox,
        runs: impl IntoIterator<Item = &'samples [Sample]>,
    ) -> Vec<u8> {
        let mut muxer = Muxer::new(file_structure);
        if let Some(file_type) = file_type {
            muxer.handle_file_type(file_type).unwrap();
        }
        muxer.handle_movie(movie).unwrap();
        for samples in runs {
            muxer.handle_samples(samples.iter().cloned()).unwrap();
        }
        muxer.finish().unwrap();
        iter::from_fn(|| muxer.poll_output()).flatten().collect()
    }

    /// Returns the initialization segment of `movie`: the file of its brands and its movie alone
    fn initialization(movie: MovieBox) -> Vec<u8> {
        mux(
            FileStructure::Fragmented {
                first_sequence_number: 1,
                random_access: false,
            },
            Some(fragmented_brands()),
            movie,
            &[],
        )
    }

    /// Returns the brands of a fragmented file, and of a segment
    fn fragmented_brands() -> FileTypeBox {
        let mut ftyp = FileTypeBox::default();
        ftyp.major_brand = FourCc::new(*b"iso6");
        ftyp.compatible_brands = vec![FourCc::new(*b"iso6"), FourCc::new(*b"mp41")];
        ftyp
    }

    /// Returns the track, size and decoding timestamp of each packet `ffprobe` reads of `file`, sorted
    fn ffprobe_read(name: &str, file: &[u8]) -> Vec<(u32, u64, Option<i64>)> {
        // Why not the process and the name alone: tests in one process probe files of the same name at once
        static PROBES: AtomicUsize = AtomicUsize::new(0);
        let probe = PROBES.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("isobmff-mux-{}-{probe}-{name}", process::id()));
        fs::write(&path, file).unwrap();
        let mut packets: Vec<_> = ffprobe_packets(&path)
            .into_iter()
            .map(|packet| (packet.track_id, packet.size, packet.dts))
            .collect();
        fs::remove_file(&path).unwrap();
        packets.sort_unstable();
        packets
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe on them"]
    fn samples_muxed_into_another_file_structure_read_back_as_they_were_read() {
        let files: Vec<_> = conformance_files()
            .into_iter()
            .filter(|file| {
                [
                    "01_simple.mp4",
                    "a4-tone-fragmented.mp4",
                    "a7-tone-oddities.mp4",
                ]
                .contains(&file.name.as_str())
            })
            .collect();
        assert_eq!(files.len(), 3);
        for ConformanceFile { name, bytes, .. } in &files {
            let source = demux(bytes);
            let first = FileStructure::Fragmented {
                first_sequence_number: 1,
                random_access: false,
            };
            let conversions = if source.movie.mvex.is_some() {
                vec![
                    (
                        FileStructure::NonFragmented,
                        source.file_type.clone(),
                        source.movie.clone(),
                    ),
                    (
                        FileStructure::Segment {
                            first_sequence_number: 1,
                            segment_index: false,
                        },
                        Some(fragmented_brands()),
                        source.movie.clone(),
                    ),
                ]
            } else {
                vec![(first, Some(fragmented_brands()), source.movie.clone())]
            };
            for (file_structure, file_type, movie) in conversions {
                let context = format!("{name} into {file_structure:?}");
                let initialization = if matches!(file_structure, FileStructure::Segment { .. }) {
                    mux(first, file_type.clone(), movie.clone(), &[])
                } else {
                    Vec::new()
                };
                let muxed = mux(
                    file_structure,
                    file_type.clone(),
                    movie.clone(),
                    &source.samples,
                );
                let whole = [initialization.as_slice(), &muxed].concat();

                let read = demux(&whole);
                assert!(
                    read.samples == source.samples,
                    "{context}: the demuxer reads other samples"
                );
                assert_eq!(
                    ffprobe_read(&format!("muxed-{name}"), &whole),
                    ffprobe_read(&format!("source-{name}"), bytes),
                    "{context}: ffprobe reads other packets"
                );
                assert!(
                    mux(file_structure, file_type, movie, &read.samples) == muxed,
                    "{context}: muxing the samples read back writes another file"
                );
            }
        }
    }

    /// Returns the next event `demuxer` reads of `file`, taking the samples it gathers, or `None` at its end
    fn next_event(demuxer: &mut Demuxer, file: &[u8]) -> Option<DemuxEvent> {
        while let Some(wanted) = demuxer.wanted_input() {
            let event = match piece(file, wanted, usize::MAX) {
                [] => demuxer.finish(),
                input => demuxer.handle_input(wanted.offset(), input),
            };
            while demuxer.poll_sample().is_some() {}
            if let Some(event) = event.unwrap() {
                return Some(event);
            }
        }
        None
    }

    /// Returns the conformance files the muxer tests write again, by name
    fn muxed_files() -> Vec<ConformanceFile> {
        let files: Vec<_> = conformance_files()
            .into_iter()
            .filter(|file| {
                [
                    "01_simple.mp4",
                    "a4-tone-fragmented.mp4",
                    "a7-tone-oddities.mp4",
                ]
                .contains(&file.name.as_str())
            })
            .collect();
        assert_eq!(files.len(), 3);
        files
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe on them"]
    fn a_demuxer_resumes_at_a_movie_fragment_from_each_entry_of_the_mfra_a_muxer_writes() {
        for ConformanceFile { name, bytes, .. } in &muxed_files() {
            let source = demux(bytes);
            let file = mux(
                FileStructure::Fragmented {
                    first_sequence_number: 1,
                    random_access: true,
                },
                Some(fragmented_brands()),
                source.movie.clone(),
                &source.samples,
            );
            let read = demux(&file);
            let entries: Vec<u64> = read
                .movie_fragment_random_access
                .iter()
                .flat_map(|mfra| &mfra.tfra)
                .flat_map(|tfra| &tfra.entries)
                .map(|entry| entry.moof_offset)
                .collect();

            let mut demuxer = demuxer();
            while !matches!(
                next_event(&mut demuxer, &file),
                Some(DemuxEvent::Movie { .. })
            ) {}
            let resumed: Vec<Option<u64>> = entries
                .iter()
                .map(|&moof_offset| {
                    demuxer.resume_at(moof_offset).unwrap();
                    match next_event(&mut demuxer, &file) {
                        Some(DemuxEvent::MovieFragment { byte_range, .. }) => {
                            Some(byte_range.offset())
                        }
                        _ => None,
                    }
                })
                .collect();

            assert!(!entries.is_empty(), "{name}: the mfra lists no sync sample");
            assert_eq!(
                resumed,
                entries.iter().copied().map(Some).collect::<Vec<_>>(),
                "{name}: resuming at an entry of the mfra reads no movie fragment there"
            );
            assert!(
                read.samples == source.samples,
                "{name}: the demuxer reads other samples"
            );
            assert_eq!(
                ffprobe_read(&format!("random-access-{name}"), &file),
                ffprobe_read(&format!("source-{name}"), bytes),
                "{name}: ffprobe reads other packets"
            );
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe on them"]
    fn the_subsegments_of_the_sidx_a_muxer_writes_are_each_movie_fragment_and_its_media_data() {
        for ConformanceFile { name, bytes, .. } in &muxed_files() {
            let source = demux(bytes);
            let segment = mux(
                FileStructure::Segment {
                    first_sequence_number: 1,
                    segment_index: true,
                },
                Some(fragmented_brands()),
                source.movie.clone(),
                &source.samples,
            );
            let whole = [initialization(source.movie.clone()), segment].concat();
            let read = demux(&whole);
            let subsegments: Vec<_> = read
                .segment_indexes
                .iter()
                .flat_map(SegmentIndex::subsegments)
                .map(|subsegment| subsegment.byte_range())
                .collect();
            let movie_fragments: Vec<u64> = framed_whole(&whole)
                .into_iter()
                .filter(|(header, _)| header.box_type() == MovieFragmentBox::BOX_TYPE)
                .map(|(_, byte_range)| byte_range.offset())
                .collect();
            let ends = movie_fragments
                .iter()
                .skip(1)
                .copied()
                .chain([u64::try_from(whole.len()).unwrap()]);

            assert_eq!(
                subsegments,
                movie_fragments
                    .iter()
                    .zip(ends)
                    .map(|(&start, end)| start..end)
                    .collect::<Vec<_>>(),
                "{name}: the subsegments are not the movie fragments"
            );
            assert!(
                read.samples == source.samples,
                "{name}: the demuxer reads other samples"
            );
            assert_eq!(
                ffprobe_read(&format!("segment-index-{name}"), &whole),
                ffprobe_read(&format!("source-{name}"), bytes),
                "{name}: ffprobe reads other packets"
            );
        }
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe on them"]
    fn the_samples_of_a_fragment_laid_out_of_decoding_order_write_as_a_movie_file_in_decoding_order()
     {
        let file = conformance_files()
            .into_iter()
            .find(|file| file.name == "sg-tl-st.mp4")
            .unwrap();
        let source = demux(&file.bytes);
        let mut in_decoding_order = source.samples.clone();
        in_decoding_order.sort_by_key(|sample| {
            let properties = sample.properties();
            (properties.track_id, properties.decoding_time)
        });
        assert!(in_decoding_order != source.samples);

        let muxed = mux(
            FileStructure::NonFragmented,
            source.file_type.clone(),
            source.movie.clone(),
            &in_decoding_order,
        );

        assert!(
            demux(&muxed).samples == in_decoding_order,
            "the demuxer reads other samples"
        );
        assert_eq!(
            ffprobe_read("muxed-sg-tl-st.mp4", &muxed),
            ffprobe_read("source-sg-tl-st.mp4", &file.bytes)
        );
    }

    /// Returns the samples of `file`, a run for each movie fragment, in the order they come out of a demuxer
    fn fragment_runs(file: &[u8]) -> Vec<Vec<Sample>> {
        let mut demuxer = demuxer();
        let mut runs: Vec<Vec<Sample>> = Vec::new();
        while let Some(wanted) = demuxer.wanted_input() {
            let event = match piece(file, wanted, usize::MAX) {
                [] => demuxer.finish(),
                input => demuxer.handle_input(wanted.offset(), input),
            };
            if let Some(DemuxEvent::MovieFragment { .. }) = event.unwrap() {
                runs.push(Vec::new());
            }
            if let Some(run) = runs.last_mut() {
                run.extend(iter::from_fn(|| demuxer.poll_sample()));
            }
        }
        runs
    }

    /// Returns `file` written again as a segment indexed by a `sidx`, a movie fragment for each of its own, after the initialization segment of its movie
    fn indexed_again(file: &[u8]) -> Vec<u8> {
        let movie = demux(file).movie;
        let runs = fragment_runs(file);
        let segment = mux_runs(
            FileStructure::Segment {
                first_sequence_number: 1,
                segment_index: true,
            },
            Some(fragmented_brands()),
            movie.clone(),
            runs.iter().map(Vec::as_slice),
        );
        [initialization(movie), segment].concat()
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_sidx_of_a_segment_indexed_again_states_the_presentation_times_its_edit_list_maps_as_its_own_sidx_does()
     {
        let file = conformance_files()
            .into_iter()
            .find(|file| file.name == "21_segment.mp4")
            .unwrap();
        let source = demux(&file.bytes);
        let read = demux(&indexed_again(&file.bytes));

        type Times = (Range<u64>, bool, u8, u32);
        let times = |demuxed: &Demuxed| -> Vec<(u32, u32, Vec<Times>)> {
            demuxed
                .segment_indexes
                .iter()
                .map(|index| {
                    let subsegments = index
                        .subsegments()
                        .iter()
                        .map(|subsegment| {
                            (
                                subsegment.presentation_time(),
                                subsegment.starts_with_sap(),
                                subsegment.sap_type(),
                                subsegment.sap_delta_time(),
                            )
                        })
                        .collect();
                    (index.reference_id(), index.timescale(), subsegments)
                })
                .collect()
        };
        assert_eq!(times(&read), times(&source));
    }

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names"]
    fn the_subsegments_of_a_segment_indexed_again_one_per_movie_fragment_last_as_its_own_sidx_says()
    {
        let file = conformance_files()
            .into_iter()
            .find(|file| file.name == "FX-VY-9436R.3_qhd-variant.mp4")
            .unwrap();
        let source = demux(&file.bytes);
        let read = demux(&indexed_again(&file.bytes));

        let subsegment_durations = |demuxed: &Demuxed| -> Vec<(u32, u32, Vec<u64>)> {
            demuxed
                .segment_indexes
                .iter()
                .map(|index| {
                    let durations = index
                        .subsegments()
                        .iter()
                        .map(|subsegment| {
                            let time = subsegment.presentation_time();
                            time.end.checked_sub(time.start).unwrap()
                        })
                        .collect();
                    (index.reference_id(), index.timescale(), durations)
                })
                .collect()
        };
        assert_eq!(subsegment_durations(&read), subsegment_durations(&source));
        assert!(
            read.samples == source.samples,
            "the demuxer reads other samples"
        );
    }
}
