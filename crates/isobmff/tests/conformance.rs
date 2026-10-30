//! Reading and writing of the boxes of a file as its bytes arrive, checked on the files of MPEG's `FileFormatConformance`
//!
//! What the reader frames out of a file cut into pieces is checked against
//! what [`Boxes`] frames out of the file whole, and what the writer writes
//! from the reader's events against the file itself. The samples gathered
//! out of the media data of a file cut into pieces are checked against the
//! bytes their locations name in the file whole. The samples of a file,
//! muxed into another file structure, read back the same through the
//! demuxer and `ffprobe`.
//! `scripts/fetch-conformance.sh` fetches the files into a directory, which
//! `ISOBMFF_CONFORMANCE_DIR` names to these tests.

#[cfg(test)]
mod tests {
    use std::{env, fs, iter, process};

    use isobmff::boxes::{
        ChunkOffsetBox, ChunkOffsetVariant, FileTypeBox, MovieBox, MovieExtendsBox, SampleSizeBox,
        SampleSizeVariant, SampleToChunkBox, TimeToSampleBox, TrackExtendsBox,
    };
    use isobmff::framing::{BoxEvent, BoxReader, BoxWriter};
    use isobmff::{
        BoxHeader, Boxes, ByteRange, DemuxEvent, DemuxLimits, Demuxer, FileStructure, FourCc,
        Muxer, Sample, SampleLocation,
    };
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

    /// Returns a demuxer that holds every sample of a file
    fn demuxer() -> Demuxer {
        Demuxer::with_limits(
            DemuxLimits::new()
                .with_held_location_count(u64::MAX)
                .with_held_bytes(u64::MAX),
        )
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
                let expected: Vec<_> = gathered
                    .locations
                    .iter()
                    .map(|location| {
                        let byte_range = location.byte_range();
                        let data = bytes
                            .get(
                                usize::try_from(byte_range.start).unwrap()
                                    ..usize::try_from(byte_range.end).unwrap(),
                            )
                            .unwrap();
                        Sample::new(*location.properties(), data.to_vec())
                    })
                    .collect();
                assert!(!expected.is_empty(), "{name} declares no sample");
                assert!(
                    gathered.samples == expected,
                    "{name} in pieces of {piece_len} gathers other samples"
                );
            }
        }
    }

    /// What a demuxer read of a file: its brands, its movie and its samples, in the order they came out
    #[derive(PartialEq, Debug)]
    struct Demuxed {
        file_type: Option<FileTypeBox>,
        movie: MovieBox,
        samples: Vec<Sample>,
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
        let mut muxer = Muxer::new(file_structure);
        if let Some(file_type) = file_type {
            muxer.handle_file_type(file_type).unwrap();
        }
        muxer.handle_movie(movie).unwrap();
        for samples in samples.chunks(SAMPLES_PER_CALL) {
            muxer.handle_samples(samples.iter().cloned()).unwrap();
        }
        muxer.finish().unwrap();
        iter::from_fn(|| muxer.poll_output()).flatten().collect()
    }

    /// Returns `movie` with sample tables that declare no sample, as a movie whose samples are muxed takes
    fn without_samples(mut movie: MovieBox) -> MovieBox {
        for trak in &mut movie.trak {
            let stbl = &mut trak.mdia.minf.stbl;
            stbl.stts = Some(TimeToSampleBox::default());
            stbl.ctts = None;
            stbl.stsc = Some(SampleToChunkBox::default());
            stbl.sample_size = Some(SampleSizeVariant::SampleSize(SampleSizeBox::default()));
            stbl.chunk_offset = Some(ChunkOffsetVariant::ChunkOffset(ChunkOffsetBox::default()));
            stbl.stss = None;
            stbl.sdtp = None;
            stbl.padb = None;
            stbl.stdp = None;
        }
        movie
    }

    /// Returns `movie` extended for movie fragments, each track by a `trex` whose samples sample entry 1 describes
    fn fragmented(movie: MovieBox) -> MovieBox {
        let mut movie = without_samples(movie);
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = movie
            .trak
            .iter()
            .map(|trak| {
                let mut trex = TrackExtendsBox::default();
                trex.track_id = trak.tkhd.track_id;
                trex.default_sample_description_index = 1;
                trex
            })
            .collect();
        movie.mvex = Some(mvex);
        movie
    }

    /// Returns `movie` without its `mvex`, as a movie that is not fragmented
    fn not_fragmented(movie: MovieBox) -> MovieBox {
        let mut movie = without_samples(movie);
        movie.mvex = None;
        movie
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
        let path = env::temp_dir().join(format!("isobmff-mux-{}-{name}", process::id()));
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
            };
            let conversions = if source.movie.mvex.is_some() {
                vec![
                    (
                        FileStructure::NonFragmented,
                        source.file_type.clone(),
                        not_fragmented(source.movie.clone()),
                    ),
                    (
                        FileStructure::Segment {
                            first_sequence_number: 1,
                        },
                        Some(fragmented_brands()),
                        fragmented(source.movie.clone()),
                    ),
                ]
            } else {
                vec![(
                    first,
                    Some(fragmented_brands()),
                    fragmented(source.movie.clone()),
                )]
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
}
