//! Segments written around samples, of several movie fragments, and indexed by a segment index with the `sidx` written over the room reserved for it, read back by this crate's demuxer and checked against ffprobe on the conformance files

#[cfg(test)]
#[path = "support/muxer.rs"]
mod support;

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use isobmff::demux::Event;
    use isobmff::mux::{
        Error, MovieParameters, SegmentIndex, SegmentIndexParameters, TrackParameters,
    };
    use isobmff::structure_data::subsegments;
    use isobmff::timeline::presentation;
    use isobmff::{Demuxer, DemuxerConfig, MuxerConfig, SampleProperties, SegmentMuxer};
    use isobmff_boxes::{SegmentIndexBox, SegmentTypeBox};
    use isobmff_core::FourCc;
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    use crate::support::{Read, mp4_file_type, one_sample_entry_track, packet_read, read, sample};

    /// What a demuxer reads of the subsegments of a file: each `sidx` with its anchor, and the file offset of each `moof` with the length of it and the `mdat` after it
    struct Indexed {
        segment_indexes: Vec<(u64, SegmentIndexBox)>,
        fragments: Vec<(u64, u32)>,
    }

    fn read_indexed(file: &[u8]) -> Indexed {
        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let mut segment_indexes = Vec::new();
        let mut fragments: Vec<(u64, u32)> = Vec::new();
        let mut position = 0;
        let mut after_moof = false;
        while let Some((header, _)) = file
            .get(position..)
            .and_then(|rest| isobmff_core::BoxHeader::decode(rest).ok())
        {
            let length = header.size().total_bytes().unwrap();
            let box_type = header.box_type();
            if box_type == isobmff_core::BoxType::compact(*b"moof") {
                fragments.push((
                    u64::try_from(position).unwrap(),
                    u32::try_from(length).unwrap(),
                ));
            } else if after_moof && box_type == isobmff_core::BoxType::compact(*b"mdat") {
                let last = fragments.last_mut().unwrap();
                last.1 = last.1.checked_add(u32::try_from(length).unwrap()).unwrap();
            }
            after_moof = box_type == isobmff_core::BoxType::compact(*b"moof");
            position = position
                .checked_add(usize::try_from(length).unwrap())
                .unwrap();
        }
        loop {
            while let Some(event) = demuxer.poll_event() {
                if let Event::SegmentIndex { anchor, sidx } = event {
                    segment_indexes.push((anchor, sidx));
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
        Indexed {
            segment_indexes,
            fragments,
        }
    }

    /// Each subsegment of each `sidx` of `indexed`: its offset, its length, its earliest presentation time, its duration and its stream access point
    fn indexed_subsegments(indexed: &Indexed) -> Vec<(u64, u32, u64, u32, bool, u8)> {
        indexed
            .segment_indexes
            .iter()
            .flat_map(|(anchor, sidx)| {
                subsegments(sidx, *anchor)
                    .unwrap()
                    .map(|subsegment| {
                        (
                            subsegment.offset,
                            subsegment.reference.referenced_size,
                            subsegment.earliest_presentation_time,
                            subsegment.reference.subsegment_duration,
                            subsegment.reference.starts_with_sap,
                            subsegment.reference.sap_type,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn segment_type() -> SegmentTypeBox {
        let mut styp = SegmentTypeBox::default();
        styp.major_brand = FourCc::new(*b"msdh");
        styp.compatible_brands = vec![FourCc::new(*b"msdh"), FourCc::new(*b"msix")];
        styp
    }

    /// A muxer of one track, of media segments of at most `subsegment_count` subsegments
    fn muxer(subsegment_count: u16) -> SegmentMuxer {
        let index = SegmentIndexParameters::new(1, NonZeroU16::new(subsegment_count).unwrap());
        let mut muxer = SegmentMuxer::new(
            MovieParameters::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
            segment_type(),
            Some(index),
        )
        .unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();
        muxer
    }

    /// Writes each `sidx` over the room reserved for it
    fn write_over(written: &mut [u8], segment_indexes: &[SegmentIndex]) {
        for sidx in segment_indexes {
            let start = usize::try_from(sidx.offset).unwrap();
            let end = start.checked_add(sidx.bytes.len()).unwrap();
            written
                .get_mut(start..end)
                .unwrap()
                .copy_from_slice(&sidx.bytes);
        }
    }

    fn not_sync(mut properties: SampleProperties) -> SampleProperties {
        properties.sample_flags.sample_is_non_sync_sample = true;
        properties
    }

    #[test]
    fn the_movie_fragments_ended_within_a_segment_follow_its_one_styp() {
        let mut muxer = SegmentMuxer::new(
            MovieParameters::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
            segment_type(),
            None,
        )
        .unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();
        let mut written = Vec::new();
        muxer.handle_sample(sample(0), b"abc").unwrap();
        muxer.finish_fragment().unwrap();
        muxer.handle_sample(sample(10), b"d").unwrap();
        muxer.finish_fragment().unwrap();
        muxer.handle_sample(sample(20), b"ef").unwrap();
        assert_eq!(muxer.finish_segment(), Ok(None));
        muxer.handle_sample(sample(30), b"g").unwrap();
        assert_eq!(muxer.finish(), Ok(None));
        muxer.poll_output(&mut written);

        let mut box_types = Vec::new();
        let mut position: usize = 0;
        while let Some((header, _)) = written
            .get(position..)
            .and_then(|rest| isobmff_core::BoxHeader::decode(rest).ok())
        {
            box_types.push(header.box_type().to_string());
            let length = usize::try_from(header.size().total_bytes().unwrap()).unwrap();
            position = position.checked_add(length).unwrap();
        }
        let read_back = read(&written).unwrap();

        assert_eq!(
            box_types,
            [
                "ftyp", "moov", "styp", "moof", "mdat", "moof", "mdat", "moof", "mdat", "styp",
                "moof", "mdat"
            ]
        );
        assert_eq!(
            read_back
                .samples
                .iter()
                .map(|(properties, bytes)| (properties.decoding_time, bytes.clone()))
                .collect::<Vec<_>>(),
            [
                (0, b"abc".to_vec()),
                (10, b"d".to_vec()),
                (20, b"ef".to_vec()),
                (30, b"g".to_vec())
            ]
        );
    }

    #[test]
    fn each_subsegment_a_segment_index_references_is_a_movie_fragment_of_the_segment() {
        let mut muxer = muxer(2);
        let mut written = Vec::new();
        let mut segment_indexes = Vec::new();
        muxer.handle_sample(sample(0), b"abc").unwrap();
        muxer.handle_sample(not_sync(sample(10)), b"d").unwrap();
        muxer.finish_fragment().unwrap();
        muxer.handle_sample(sample(20), b"ef").unwrap();
        segment_indexes.extend(muxer.finish_segment().unwrap());
        muxer.handle_sample(not_sync(sample(30)), b"g").unwrap();
        muxer.finish_fragment().unwrap();
        muxer.handle_sample(sample(40), b"hi").unwrap();
        segment_indexes.extend(muxer.finish().unwrap());
        muxer.poll_output(&mut written);
        write_over(&mut written, &segment_indexes);

        let indexed = read_indexed(&written);

        let fragment = |index: usize| indexed.fragments.get(index).copied().unwrap();
        assert_eq!(indexed.segment_indexes.len(), 2);
        assert_eq!(
            indexed_subsegments(&indexed),
            [
                (fragment(0).0, fragment(0).1, 0, 20, true, 1),
                (fragment(1).0, fragment(1).1, 20, 10, true, 1),
                (fragment(2).0, fragment(2).1, 30, 10, false, 0),
                (fragment(3).0, fragment(3).1, 40, 10, true, 1),
            ]
        );
    }

    #[test]
    fn a_segment_of_fewer_subsegments_than_reserved_steps_over_the_room_left() {
        let mut muxer = muxer(5);
        let mut written = Vec::new();
        muxer.handle_sample(sample(0), b"abc").unwrap();
        let segment_index = muxer.finish().unwrap().unwrap();
        muxer.poll_output(&mut written);
        write_over(&mut written, &[segment_index]);

        let indexed = read_indexed(&written);

        let (anchor, sidx) = indexed.segment_indexes.first().unwrap();
        assert_eq!(
            (anchor + sidx.first_offset, sidx.references.len()),
            (indexed.fragments.first().unwrap().0, 1)
        );
    }

    #[test]
    fn a_sample_past_the_subsegments_a_segment_holds_is_refused_and_opens_the_next_segment() {
        let mut muxer = muxer(1);
        muxer.handle_sample(sample(0), b"abc").unwrap();
        muxer.finish_fragment().unwrap();

        assert_eq!(
            muxer.handle_sample(sample(10), b"d"),
            Err(Error::SubsegmentCount {
                subsegment_count: 1
            })
        );
        assert!(muxer.finish_segment().unwrap().is_some());
        assert_eq!(muxer.handle_sample(sample(10), b"d"), Ok(()));
    }

    #[test]
    fn a_subsegment_presented_before_the_one_before_it_is_refused() {
        let mut muxer = muxer(2);
        muxer.handle_sample(sample(0), b"abc").unwrap();
        muxer.finish_fragment().unwrap();
        let mut early = sample(10);
        early.sample_composition_time_offset = -10;
        muxer.handle_sample(early, b"d").unwrap();

        assert_eq!(
            muxer.finish_fragment(),
            Err(Error::SubsegmentTime {
                expected: 0,
                found: 0
            })
        );
    }

    #[test]
    fn a_segment_that_does_not_start_where_the_one_before_it_ends_is_refused() {
        let mut muxer = muxer(2);
        let mut first = sample(0);
        first.sample_composition_time_offset = 5;
        muxer.handle_sample(first, b"abc").unwrap();
        muxer.finish_segment().unwrap();
        muxer.handle_sample(sample(10), b"d").unwrap();

        assert_eq!(
            muxer.finish_segment(),
            Err(Error::SubsegmentTime {
                expected: 15,
                found: 10
            })
        );
    }

    #[test]
    fn a_subsegment_of_no_sample_of_the_reference_track_is_refused() {
        let index = SegmentIndexParameters::new(1, NonZeroU16::new(2).unwrap());
        let mut muxer = SegmentMuxer::new(
            MovieParameters::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
            segment_type(),
            Some(index),
        )
        .unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();
        muxer.add_track(one_sample_entry_track(2)).unwrap();
        let mut other = sample(0);
        other.track_id = 2;
        muxer.handle_sample(other, b"abc").unwrap();

        assert_eq!(
            muxer.finish_fragment(),
            Err(Error::NoReferenceSample { track_id: 1 })
        );
    }

    #[test]
    fn a_sample_is_refused_until_the_reference_track_is_added() {
        let index = SegmentIndexParameters::new(7, NonZeroU16::new(2).unwrap());
        let mut muxer = SegmentMuxer::new(
            MovieParameters::new(1000),
            MuxerConfig::default(),
            mp4_file_type(),
            segment_type(),
            Some(index),
        )
        .unwrap();
        muxer.add_track(one_sample_entry_track(1)).unwrap();

        assert_eq!(
            muxer.handle_sample(sample(0), b"abc"),
            Err(Error::UnknownTrack { track_id: 7 })
        );
    }

    #[test]
    fn a_refused_subsegment_changes_nothing() {
        let mut muxer = muxer(2);
        let mut written = Vec::new();
        muxer.handle_sample(sample(0), b"abc").unwrap();
        muxer.finish_fragment().unwrap();
        muxer.poll_output(&mut written);
        let mut early = sample(10);
        early.sample_composition_time_offset = -10;
        muxer.handle_sample(early, b"d").unwrap();
        let before = muxer.clone();

        muxer.finish_fragment().unwrap_err();
        let (mut held, mut held_before) = (Vec::new(), Vec::new());
        muxer.clone().poll_output(&mut held);
        before.clone().poll_output(&mut held_before);

        assert_eq!(held, held_before);
        assert_eq!(
            format!("{:?}", muxer.finish_segment()),
            format!("{:?}", before.clone().finish_segment())
        );
    }

    /// Writes the samples of `source` in media segments of at most `subsegment_count` subsegments, each where a fragment of the source ends and the next sample of `reference_track_id` is a sync sample, and each `sidx` over the room reserved for it
    fn write_indexed(
        source: &Read,
        reference_track_id: u32,
        subsegment_count: u16,
    ) -> Result<Vec<u8>, Error> {
        let index = SegmentIndexParameters::new(
            reference_track_id,
            NonZeroU16::new(subsegment_count).unwrap(),
        );
        let mut muxer = SegmentMuxer::new(
            MovieParameters::from(&source.movie),
            MuxerConfig::default(),
            source.file_type.clone(),
            segment_type(),
            Some(index),
        )?;
        for trak in &source.movie.trak {
            muxer.add_track(TrackParameters::from(trak))?;
        }
        let (mut written, mut segment_indexes) = (Vec::new(), Vec::new());
        let mut referenced = false;
        for (index, (properties, bytes)) in source.samples.iter().enumerate() {
            // Why end a subsegment only where a fragment of the source ends
            // and the reference track goes on with a sync sample: a fragment
            // may start inside a group of pictures, whose samples the next
            // presents before this one ends, or hold samples of another track
            // alone, and subsegments are contiguous in presentation time,
            // each with a time of the reference track.
            let opens_with_sync = source
                .samples
                .iter()
                .skip(index)
                .find(|(next, _)| next.track_id == reference_track_id)
                .is_some_and(|(next, _)| !next.sample_flags.sample_is_non_sync_sample);
            if source.fragment_starts.contains(&index) && referenced && opens_with_sync {
                muxer.finish_fragment()?;
                referenced = false;
            }
            match muxer.handle_sample(*properties, bytes) {
                Err(Error::SubsegmentCount { .. }) => {
                    segment_indexes.extend(muxer.finish_segment()?);
                    muxer.handle_sample(*properties, bytes)?;
                }
                result => result?,
            }
            referenced |= properties.track_id == reference_track_id;
            muxer.poll_output(&mut written);
        }
        segment_indexes.extend(muxer.finish()?);
        muxer.poll_output(&mut written);
        write_over(&mut written, &segment_indexes);
        Ok(written)
    }

    /// The earliest presentation time of the samples of track `track_id` of each fragment `read`
    fn earliest_presentation_times(read: &Read, track_id: u32) -> Vec<Option<u64>> {
        let starts: Vec<usize> = read.fragment_starts.iter().copied().collect();
        let ends = starts.iter().skip(1).copied().chain([read.samples.len()]);
        starts
            .iter()
            .zip(ends)
            .map(|(&start, end)| {
                let samples = read
                    .samples
                    .get(start..end)
                    .unwrap()
                    .iter()
                    .map(|(properties, _)| *properties)
                    .filter(|properties| properties.track_id == track_id);
                presentation(&read.movie, track_id, samples)
                    .unwrap()
                    .map(|sample| sample.presentation_time)
                    .min()
            })
            .collect()
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR and ffprobe"]
    fn the_conformance_files_written_in_indexed_segments_index_each_fragment_and_read_as_the_same_packets()
     {
        let directory =
            std::env::temp_dir().join(format!("isobmff-indexed-segments-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let (mut differences, mut refused) = (Vec::new(), Vec::new());
        let (mut written_files, mut compared, mut subsegment_total) = (0, 0, 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            // Why only fragmented sources without errors: their samples are
            // written in movie fragments as they are, and a file the demuxer
            // reports an error in has none to write.
            if source.movie.mvex.is_none() || !source.errors.is_empty() || source.samples.is_empty()
            {
                continue;
            }
            let reference = source
                .movie
                .trak
                .iter()
                .find(|trak| trak.mdia.hdlr.handler_type == FourCc::new(*b"vide"))
                .or_else(|| source.movie.trak.first())
                .unwrap();
            let track_id = reference.tkhd.track_id;
            let written = match write_indexed(&source, track_id, 2) {
                Ok(written) => written,
                Err(error) => {
                    refused.push(format!("{}: {error:?}", file.name));
                    continue;
                }
            };
            written_files += 1;
            let indexed = read_indexed(&written);
            let ours: Vec<_> = indexed_subsegments(&indexed)
                .iter()
                .map(|subsegment| (subsegment.0, subsegment.1, Some(subsegment.2)))
                .collect();
            let read_back = read(&written).unwrap();
            let theirs: Vec<_> = indexed
                .fragments
                .iter()
                .zip(earliest_presentation_times(&read_back, track_id))
                .map(|(&(offset, length), time)| (offset, length, time))
                .collect();
            subsegment_total += ours.len();
            if ours != theirs {
                differences.push(format!(
                    "{}: subsegments {ours:?}, fragments {theirs:?}",
                    file.name
                ));
            }
            if read_back.samples != source.samples || !read_back.errors.is_empty() {
                differences.push(format!("{}: the samples read back differ", file.name));
            }
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let path = directory.join(&file.name);
            std::fs::write(&path, &written).unwrap();
            let packets = |path: &std::path::Path| {
                ffprobe_packets(&[], path)
                    .iter()
                    .map(packet_read)
                    .collect::<Vec<_>>()
            };
            compared += 1;
            if packets(&path) != packets(&file.path) {
                differences.push(format!("{}: ffprobe reads other packets", file.name));
            }
        }
        std::fs::remove_dir_all(&directory).unwrap();
        println!(
            "{written_files} files written, {subsegment_total} subsegments, {compared} compared with ffprobe; refused: {refused:?}"
        );
        assert_eq!(differences, Vec::<String>::new());
        assert!(written_files > 0);
    }
}
