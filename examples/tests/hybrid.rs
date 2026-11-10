//! Movies the hybrid writer writes, read while they are written as fragmented movie files and once they end as movie files, by this crate's demuxer and by ffprobe

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use isobmff::framing::{BoxReader, Event, FramerConfig};
    use isobmff::structure_data;
    use isobmff::{MuxerConfig, SampleProperties};
    use isobmff_examples::demux::{Read, read};
    use isobmff_examples::hybrid::{Error, HybridWriter};
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    /// The fixtures of the reading tests, two movie files and two of movie fragments
    const FIXTURES: [&[u8]; 4] = [
        include_bytes!("../../crates/isobmff/tests/fixtures/movie_moov_first.mp4"),
        include_bytes!("../../crates/isobmff/tests/fixtures/movie_moov_last.mp4"),
        include_bytes!("../../crates/isobmff/tests/fixtures/fragmented.mp4"),
        include_bytes!("../../crates/isobmff/tests/fixtures/segment.mp4"),
    ];

    /// What a hybrid writer writes of `source`, a fragment for every `samples_per_fragment` samples: the bytes before it finishes, and the file once it has
    fn write_hybrid(
        source: &Read,
        samples_per_fragment: usize,
    ) -> Result<(Vec<u8>, Vec<u8>), Error> {
        let mut written = Vec::new();
        let mut writer = HybridWriter::new(
            &source.file_type,
            source.movie.clone(),
            MuxerConfig::default().sample_count_limit,
            &mut written,
        )?;
        for (index, (properties, bytes)) in source.samples.iter().enumerate() {
            if index > 0 && index.checked_rem(samples_per_fragment) == Some(0) {
                writer.finish_fragment(&mut written)?;
            }
            writer.handle_sample(*properties, bytes)?;
        }
        writer.finish_fragment(&mut written)?;
        let fragmented = written.clone();
        let header = writer.finish(&mut written)?;
        let free = usize::try_from(header.offset).unwrap();
        written
            .get_mut(free..)
            .and_then(|rest| rest.get_mut(..header.bytes.len()))
            .unwrap()
            .copy_from_slice(&header.bytes);
        Ok((fragmented, written))
    }

    /// The samples of `source`, the decoding times of each track counted from that of its first sample, as a sample table declares them
    fn counted_from_the_first(
        samples: &[(SampleProperties, Vec<u8>)],
    ) -> Vec<(SampleProperties, Vec<u8>)> {
        let mut firsts = BTreeMap::new();
        samples
            .iter()
            .map(|(properties, bytes)| {
                let first = *firsts
                    .entry(properties.track_id)
                    .or_insert(properties.decoding_time);
                let properties = SampleProperties {
                    decoding_time: properties.decoding_time.checked_sub(first).unwrap(),
                    ..*properties
                };
                (properties, bytes.clone())
            })
            .collect()
    }

    /// The type of each box at file level of `file`
    fn top_level_box_types(file: &[u8]) -> Vec<String> {
        let mut config = FramerConfig::new(0);
        config.gathered_box_types.clear();
        let mut reader = BoxReader::new(config);
        let mut types = Vec::new();
        loop {
            while let Some(event) = reader.poll_event() {
                if let Event::Box { header, .. } = event {
                    types.push(header.box_type().to_string());
                }
            }
            let Some(required) = reader.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => reader.handle_input(required.position, bytes),
                _ => reader.finish(),
            }
        }
        types
    }

    #[test]
    fn a_movie_reads_as_fragmented_while_it_is_written_and_as_a_movie_file_once_it_ends() {
        for file in FIXTURES {
            let source = read(file).unwrap();
            let (fragmented, finished) = write_hybrid(&source, 16).unwrap();

            let while_written = read(&fragmented).unwrap();
            assert_eq!(
                (
                    while_written.samples,
                    while_written.movie.mvex.is_some(),
                    while_written.errors
                ),
                (source.samples.clone(), true, Vec::new())
            );

            let once_ended = read(&finished).unwrap();
            assert_eq!(
                (
                    once_ended.samples,
                    once_ended.movie.mvex.is_some(),
                    once_ended.errors,
                    top_level_box_types(&finished)
                ),
                (
                    counted_from_the_first(&source.samples),
                    false,
                    Vec::new(),
                    ["ftyp", "mdat", "moov"].map(String::from).to_vec()
                )
            );
        }
    }

    /// What ffprobe reads of each packet of `path`, ignoring edit lists: its track, its size, the offset of its composition time from its decoding time, and whether it is a key frame
    fn packets(path: &Path) -> Vec<(u32, u64, Option<i64>, bool)> {
        ffprobe_packets(&["-ignore_editlist", "1"], path)
            .iter()
            .map(|packet| {
                (
                    packet.track_id,
                    packet.size,
                    packet
                        .pts
                        .zip(packet.dts)
                        .map(|(pts, dts)| pts.checked_sub(dts).unwrap()),
                    packet.key,
                )
            })
            .collect()
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_written_as_hybrid_movies_read_as_the_same_samples_and_packets() {
        let directory = std::env::temp_dir().join(format!("isobmff-hybrid-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut written_files, mut compared, mut refused) = (Vec::new(), 0, 0, 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            // Why skip these: a file the demuxer reports an error in, such as
            // one whose samples lie in another file, has none to write.
            if !source.errors.is_empty() || source.samples.is_empty() {
                continue;
            }
            let (fragmented, finished) = match write_hybrid(&source, 30) {
                Ok(written) => written,
                // Why accept this refusal: a `tfdt` is the sum of the
                // durations of the samples before it, §8.8.12, and the source
                // jumps.
                Err(Error::StructureData(structure_data::Error::DecodingTimeGap { .. })) => {
                    refused += 1;
                    continue;
                }
                Err(error) => {
                    differences.push(format!("{}: {error:?}", file.name));
                    continue;
                }
            };
            written_files += 1;
            let while_written = read(&fragmented).unwrap();
            let once_ended = read(&finished).unwrap();
            if (&while_written.samples, &while_written.errors) != (&source.samples, &Vec::new()) {
                differences.push(format!(
                    "{}: the samples read while written differ",
                    file.name
                ));
            }
            if (&once_ended.samples, &once_ended.errors)
                != (&counted_from_the_first(&source.samples), &Vec::new())
            {
                differences.push(format!("{}: the samples read once ended differ", file.name));
            }
            // Why compare these with this crate's demuxer alone: ffprobe
            // refuses to read the source.
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let presented = |path: &Path| ffprobe_packets(&[], path).len();
            let written_path = directory.join(format!("presented-{}", file.name));
            std::fs::write(&written_path, &finished).unwrap();
            if presented(&written_path) != presented(&file.path) {
                differences.push(format!(
                    "{} once ended: ffprobe presents {} packets through the edit lists, {} of the source",
                    file.name,
                    presented(&written_path),
                    presented(&file.path)
                ));
            }
            let theirs = packets(&file.path);
            for (stage, bytes) in [("while written", &fragmented), ("once ended", &finished)] {
                let path = directory.join(format!("{stage}-{}", file.name));
                std::fs::write(&path, bytes).unwrap();
                let ours = packets(&path);
                compared += 1;
                if ours != theirs {
                    differences.push(format!(
                        "{} {stage}: ffprobe reads {} packets, {} from the source; first differing {:?}",
                        file.name,
                        ours.len(),
                        theirs.len(),
                        ours.iter().zip(&theirs).find(|(ours, theirs)| ours != theirs)
                    ));
                }
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{written_files} files written, {compared} compared with ffprobe, {refused} refused for a gap in decoding times"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(written_files > 0);
    }
}
