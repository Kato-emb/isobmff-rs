//! The samples a writer laid down as a non-fragmented movie file, read back off that file

// Why not inside `mod tests`: an inline `mod` adds its own name as a directory
// segment, so a nested one looks for `tests/tests/helpers/non_fragmented_reading.rs`.
// The `cfg` is what keeps `allow-unwrap-in-tests` reaching the helper from out here.
#[cfg(test)]
#[path = "helpers/non_fragmented_reading.rs"]
mod reading;

#[cfg(test)]
mod tests {
    use super::reading::samples_of;
    use isobmff_boxes::{
        EditBox, EditListBox, EditListEntry, FileTypeBox, HeaderDuration, MediaRate, MovieBox,
        MovieHeaderBox, SampleFlags,
    };
    use isobmff_core::{BoxType, FourCc, Mp4EpochSeconds};
    use isobmff_sample::{Sample, SampleProperties};
    use isobmff_sequence::BoxEvent;
    use isobmff_structure::{MovieDemuxFsm, NonFragmentedMuxFsm};
    use isobmff_test_support::{events_of, file_type, track};

    /// Ticks a second the media of the movie is timed in
    const TIMESCALE: u32 = 90_000;

    /// Movie of two tracks declaring no sample yet, the template the writer fills in
    fn movie() -> MovieBox {
        movie_timed_in(TIMESCALE)
    }

    /// Movie of [`movie`], the movie header counting in `timescale`
    fn movie_timed_in(timescale: u32) -> MovieBox {
        let epoch = Mp4EpochSeconds::from_seconds(0);

        MovieBox::new(
            MovieHeaderBox::new(epoch, epoch, timescale, HeaderDuration::ZERO, 3),
            vec![track(1), track(2)],
            None,
        )
        .unwrap()
    }

    /// The samples the two tracks carry, chunk by chunk, the tracks taking turns
    fn two_track_chunks() -> Vec<Vec<Sample>> {
        let video = |decode_time, data: &[u8]| {
            Sample::new(
                SampleProperties {
                    track_id: 1,
                    decode_time,
                    sample_duration: 3_000,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 1,
                },
                data.to_vec(),
            )
        };
        let audio = |decode_time, data: &[u8]| {
            Sample::new(
                SampleProperties {
                    track_id: 2,
                    decode_time,
                    sample_duration: 1_024,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 1,
                },
                data.to_vec(),
            )
        };

        vec![
            vec![video(0, b"VIDEO_01"), video(3_000, b"VIDEO_02")],
            vec![
                audio(0, b"AUD1"),
                audio(1_024, b"AUD2"),
                audio(2_048, b"AUD3"),
            ],
            vec![video(6_000, b"VIDEO_03")],
            vec![audio(3_072, b"AUD4")],
        ]
    }

    /// The file the chunks make: the brands if any are handed over, one `mdat` per chunk, then `movie`
    fn written_file(
        brands: Option<FileTypeBox>,
        movie: MovieBox,
        chunks: Vec<Vec<Sample>>,
    ) -> Vec<u8> {
        let mut mux_fsm = NonFragmentedMuxFsm::new();
        let mut file = Vec::new();

        if let Some(brands) = brands {
            mux_fsm.handle_file_type(brands).unwrap();
        }
        mux_fsm.handle_movie(movie).unwrap();
        for chunk in chunks {
            mux_fsm.begin_chunk().unwrap();
            for sample in chunk {
                mux_fsm.handle_sample(sample).unwrap();
            }
            mux_fsm.finish_chunk().unwrap();
        }
        mux_fsm.finish().unwrap();

        while let Some(written) = mux_fsm.poll_output() {
            file.extend_from_slice(&written);
        }

        file
    }

    #[test]
    fn the_samples_are_read_back_as_they_were_handed_over_chunk_by_chunk() {
        let file = written_file(Some(file_type()), movie(), two_track_chunks());

        assert_eq!(samples_of(&file, file.len()), two_track_chunks().concat());
    }

    #[test]
    fn each_chunk_is_laid_down_as_its_own_media_data_box_and_the_movie_comes_last() {
        let file = written_file(Some(file_type()), movie(), two_track_chunks());

        let top_level: Vec<BoxType> = events_of(&file, file.len())
            .unwrap()
            .into_iter()
            .filter_map(|(_extent, event)| {
                if let BoxEvent::Header(header) = event {
                    Some(header.box_type())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            top_level,
            [b"ftyp", b"mdat", b"mdat", b"mdat", b"mdat", b"moov"]
                .map(|fourcc| BoxType::compact(*fourcc))
        );
    }

    #[test]
    fn a_file_handed_no_brands_is_read_back_declaring_the_brand_its_layout_requires() {
        let file = written_file(None, movie(), two_track_chunks());

        let mut demux_fsm = MovieDemuxFsm::new();
        demux_fsm.handle_input(0, &file).unwrap();

        assert_eq!(
            demux_fsm.file_type(),
            Some(&FileTypeBox::new(
                FourCc::new(*b"iso4"),
                0,
                vec![FourCc::new(*b"iso4")]
            ))
        );
    }

    #[test]
    fn the_durations_are_stated_from_the_samples_each_track_was_handed() {
        let file = written_file(None, movie_timed_in(1_000), two_track_chunks());

        let mut demux_fsm = MovieDemuxFsm::new();
        demux_fsm.handle_input(0, &file).unwrap();
        let read = demux_fsm.movie().unwrap();

        let mut expected = read.clone();
        let duration = |value| HeaderDuration::new(value).unwrap();
        *expected.mvhd_mut() = read.mvhd().clone().with_duration(duration(100));
        for (track_id, media_duration, track_duration) in [(1, 9_000, 100), (2, 4_096, 46)] {
            let track = expected.trak_by_id_mut(track_id).unwrap();
            *track.tkhd_mut() = track.tkhd().clone().with_duration(duration(track_duration));
            *track.mdia_mut().mdhd_mut() = track
                .mdia()
                .mdhd()
                .clone()
                .with_duration(duration(media_duration));
        }
        assert_eq!(read, &expected);
    }

    #[test]
    fn a_last_edit_of_length_0_is_laid_down_lasting_until_the_last_composition_ends() {
        let edits = |segment_duration| {
            EditBox::new().with_elst(EditListBox::new(vec![
                EditListEntry::new(500, None, MediaRate::NORMAL),
                EditListEntry::new(segment_duration, Some(3_000), MediaRate::NORMAL),
            ]))
        };
        let mut movie = movie_timed_in(1_000);
        let track = movie.trak_by_id_mut(1).unwrap();
        *track = track.clone().with_edts(edits(0));
        let composed_late = |decode_time, sample_composition_time_offset| {
            Sample::new(
                SampleProperties {
                    track_id: 1,
                    decode_time,
                    sample_duration: 3_000,
                    sample_composition_time_offset,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 1,
                },
                b"VIDEO".to_vec(),
            )
        };
        let chunks = vec![vec![
            composed_late(0, 3_000),
            composed_late(3_000, 6_000),
            composed_late(6_000, 0),
        ]];
        let file = written_file(None, movie, chunks);

        let mut demux_fsm = MovieDemuxFsm::new();
        demux_fsm.handle_input(0, &file).unwrap();
        let read = demux_fsm.movie().unwrap().trak_by_id(1).unwrap();

        let mut expected = read.clone().with_edts(edits(100));
        *expected.tkhd_mut() = read
            .tkhd()
            .clone()
            .with_duration(HeaderDuration::new(600).unwrap());
        assert_eq!(read, &expected);
    }
}
