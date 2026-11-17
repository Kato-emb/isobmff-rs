//! The segment indexes of a file resolved to where their subsegments lie and when they are presented, checked against the boxes and the samples of files FFmpeg and the conformance suite wrote

#[cfg(test)]
mod tests {
    use isobmff::demux::Event;
    use isobmff::structure_data::subsegments;
    use isobmff::timeline::presentation;
    use isobmff::{Demuxer, DemuxerConfig, SampleLocation};
    use isobmff_boxes::{MovieBox, SegmentIndexBox};
    use isobmff_core::{BoxHeader, BoxType};
    use isobmff_test_support::conformance_files;

    /// What a file holds: its `moov`, where each sample lies, each `sidx` with its anchor, and each box at file level by its offset, length and type
    struct Read {
        movie: MovieBox,
        locations: Vec<SampleLocation>,
        segment_indexes: Vec<(u64, SegmentIndexBox)>,
        boxes: Vec<(u64, u64, BoxType)>,
    }

    fn read(file: &[u8]) -> Option<Read> {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let (mut movie, mut locations, mut segment_indexes) = (None, Vec::new(), Vec::new());
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    Event::Movie(moov) => movie = Some(*moov),
                    Event::SampleLocation(location) => locations.push(location),
                    Event::SegmentIndex { anchor, sidx } => segment_indexes.push((anchor, sidx)),
                    Event::FileType(_)
                    | Event::SegmentType(_)
                    | Event::MovieFragment(_)
                    | Event::Sample(_)
                    | Event::Box { .. }
                    | Event::Error(_)
                    | _ => {}
                }
            }
            let Some(required) = demuxer.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
                _ => demuxer.finish(),
            }
        }
        let mut boxes = Vec::new();
        let mut offset: u64 = 0;
        while let Some((header, _)) = file
            .get(usize::try_from(offset).unwrap()..)
            .and_then(|rest| BoxHeader::decode(rest).ok())
        {
            let length = header.size().total_bytes()?;
            boxes.push((offset, length, header.box_type()));
            offset = offset.checked_add(length)?;
        }
        Some(Read {
            movie: movie?,
            locations,
            segment_indexes,
            boxes,
        })
    }

    /// What each subsegment of `read` is not: a run of whole boxes at file level, from where the subsegment starts, that holds a `moof`, or that is a `sidx` where it references one; and, where `timed`, one whose earliest presentation time is that of the samples of the reference track that lie in it
    fn differences(read: &Read, timed: bool) -> Vec<String> {
        let mut differences = Vec::new();
        for (anchor, sidx) in &read.segment_indexes {
            let track_id = sidx.reference_id;
            for subsegment in subsegments(sidx, *anchor).unwrap() {
                let start = subsegment.offset;
                let end = start
                    .checked_add(u64::from(subsegment.reference.referenced_size))
                    .unwrap();
                let run: Vec<BoxType> = read
                    .boxes
                    .iter()
                    .skip_while(|&&(offset, _, _)| offset != start)
                    .take_while(|&&(offset, _, _)| offset < end)
                    .map(|&(_, _, box_type)| box_type)
                    .collect();
                let run_end = read
                    .boxes
                    .iter()
                    .filter(|&&(offset, _, _)| (start..end).contains(&offset))
                    .map(|&(offset, length, _)| offset.checked_add(length).unwrap())
                    .max();
                let item = if subsegment.reference.reference_type {
                    run.first() == Some(&BoxType::compact(*b"sidx"))
                } else {
                    run.contains(&BoxType::compact(*b"moof"))
                };
                if run.is_empty() || run_end != Some(end) || !item {
                    differences.push(format!(
                        "subsegment {start}..{end} of the sidx of track {track_id} at {anchor} is the boxes {run:?}"
                    ));
                    continue;
                }
                if subsegment.reference.reference_type || !timed {
                    continue;
                }
                let samples = read
                    .locations
                    .iter()
                    .filter(|location| {
                        location.properties.track_id == track_id
                            && (start..end).contains(&location.offset)
                    })
                    .map(|location| location.properties);
                let earliest = presentation(&read.movie, track_id, samples)
                    .unwrap()
                    .map(|sample| sample.presentation_time)
                    .min();
                if earliest != Some(subsegment.earliest_presentation_time) {
                    differences.push(format!(
                        "subsegment {start}..{end} of track {track_id} is presented from {:?}, the sidx says {}",
                        earliest, subsegment.earliest_presentation_time
                    ));
                }
            }
        }
        differences
    }

    /// The number of items each `sidx` of `read` references
    fn reference_counts(read: &Read) -> Vec<usize> {
        read.segment_indexes
            .iter()
            .map(|(_, sidx)| sidx.references.len())
            .collect()
    }

    #[test]
    fn a_segment_index_of_every_fragment_references_each_moof_from_where_its_samples_are_presented()
    {
        let read = read(include_bytes!("fixtures/segment_index_global.mp4")).unwrap();

        assert_eq!(
            (reference_counts(&read), differences(&read, true)),
            (vec![3, 3], Vec::<String>::new())
        );
    }

    #[test]
    fn a_segment_index_before_each_fragment_references_it_from_where_its_samples_are_presented() {
        let read = read(include_bytes!("fixtures/segment_index_per_fragment.mp4")).unwrap();

        assert_eq!(
            (reference_counts(&read), differences(&read, true)),
            (vec![1; 6], Vec::<String>::new())
        );
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR"]
    fn the_segment_indexes_of_the_conformance_files_reference_runs_of_boxes_that_hold_a_moof() {
        let mut indexed = Vec::new();
        let mut differences_found = Vec::new();
        for file in conformance_files() {
            let Some(read) = read(&file.bytes) else {
                continue;
            };
            if read.segment_indexes.is_empty() {
                continue;
            }
            indexed.push((file.name.clone(), reference_counts(&read)));
            // Why not compare the times here: the `sidx` of
            // FX-VY-9436R.3_qhd-variant gives each subsegment the same
            // nominal duration, which the samples of its fragments do not
            // last; the times are compared on the files FFmpeg wrote.
            differences_found.extend(
                differences(&read, false)
                    .into_iter()
                    .map(|difference| format!("{}: {difference}", file.name)),
            );
        }
        println!("{indexed:?}");
        assert_eq!(differences_found, Vec::<String>::new());
        assert_eq!(indexed.len(), 3);
    }
}
