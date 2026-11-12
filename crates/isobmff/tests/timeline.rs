//! How long the media, the tracks and the movie of a `moov` last, as their sample tables and edit lists give it

#[cfg(test)]
mod tests {
    use isobmff::timeline::{Error, declare_durations, move_media_times, presentation};
    use isobmff::{SampleLocation, SampleProperties};
    use isobmff_boxes::{
        CompositionOffsetBox, CompositionOffsetEntry, EditBox, EditListBox, EditListEntry,
        MovieBox, MovieExtendsBox, SampleFlags, TrackBox,
    };
    use isobmff_test_support::fixtures::{movie, track};

    /// Track `track_id` of three samples lasting 1 each in media of `timescale`, composed at the offsets `offsets`, whose edits are `(edit_duration, media_time)` (no edit list where `None`)
    fn timed_track(
        track_id: u32,
        timescale: u32,
        offsets: Option<[i64; 3]>,
        edits: Option<&[(u64, i64)]>,
    ) -> TrackBox {
        let mut trak = track(track_id, 1, &[4, 4, 4], &[(1, 3)], &[100]);
        trak.mdia.mdhd.timescale = timescale;
        trak.mdia.minf.stbl.ctts = offsets.map(|offsets| {
            let mut ctts = CompositionOffsetBox::default();
            ctts.entries = offsets
                .into_iter()
                .map(|offset| {
                    let mut entry = CompositionOffsetEntry::default();
                    (entry.sample_count, entry.sample_offset) = (1, offset);
                    entry
                })
                .collect();
            ctts
        });
        trak.edts = edits.map(|edits| {
            let mut elst = EditListBox::default();
            elst.version = u8::from(
                edits
                    .iter()
                    .any(|&(edit_duration, _)| edit_duration > u64::from(u32::MAX)),
            );
            elst.entries = edits
                .iter()
                .map(|&(edit_duration, media_time)| {
                    let mut entry = EditListEntry::default();
                    (entry.edit_duration, entry.media_time) = (edit_duration, media_time);
                    entry.media_rate_integer = 1;
                    entry
                })
                .collect();
            let mut edts = EditBox::default();
            edts.elst = Some(elst);
            edts
        });
        trak
    }

    /// The movie of `tracks` in `timescale`, its durations set
    fn with_durations(timescale: u32, tracks: impl IntoIterator<Item = TrackBox>) -> MovieBox {
        let mut moov = movie(tracks);
        moov.mvhd.timescale = timescale;
        declare_durations(&mut moov);
        moov
    }

    /// The `(edit_duration, media_time)` of each edit of the first track of `moov`, the duration of the track and of its media
    fn durations(moov: &MovieBox) -> (Vec<(u64, i64)>, u64, u64) {
        let trak = moov.trak.first().unwrap();
        let edits = trak
            .edts
            .iter()
            .filter_map(|edts| edts.elst.as_ref())
            .flat_map(|elst| elst.entries.iter())
            .map(|entry| (entry.edit_duration, entry.media_time))
            .collect();
        (edits, trak.tkhd.duration, trak.mdia.mdhd.duration)
    }

    /// Three samples lasting 1 each, composed at 1, 3 and 2 until 2, 4 and 3, end their composition at 4: an edit from 1 lasts 3, past the 2 their durations leave it
    #[test]
    fn a_last_edit_of_duration_0_lasts_to_the_end_of_the_composition_of_the_samples() {
        assert_eq!(
            [
                durations(&with_durations(
                    1000,
                    [timed_track(1, 1000, Some([1, 2, 0]), Some(&[(0, 1)]))]
                )),
                durations(&with_durations(
                    1000,
                    [timed_track(1, 1000, None, Some(&[(500, -1), (0, 1)]))]
                )),
            ],
            [(vec![(3, 1)], 3, 3), (vec![(500, -1), (2, 1)], 502, 3)]
        );
    }

    #[test]
    fn a_closed_edit_and_a_track_without_edits_are_counted_in_the_movie_timescale_rounded_up() {
        assert_eq!(
            [
                durations(&with_durations(
                    1000,
                    [timed_track(1, 3, None, Some(&[(0, 0)]))]
                )),
                durations(&with_durations(
                    1000,
                    [timed_track(1, 7, None, Some(&[(0, 0)]))]
                )),
                durations(&with_durations(1000, [timed_track(1, 10, None, None)])),
            ],
            [
                (vec![(1000, 0)], 1000, 3),
                (vec![(429, 0)], 429, 3),
                (vec![], 300, 3),
            ]
        );
    }

    #[test]
    fn edits_of_a_duration_keep_it() {
        let moov = with_durations(1000, [timed_track(1, 1000, None, Some(&[(7, 1)]))]);
        assert_eq!(durations(&moov), (vec![(7, 1)], 7, 3));
    }

    #[test]
    fn media_of_a_timescale_of_0_counts_no_time() {
        assert_eq!(
            [
                durations(&with_durations(
                    1000,
                    [timed_track(1, 0, None, Some(&[(0, 0)]))]
                )),
                durations(&with_durations(1000, [timed_track(1, 0, None, None)])),
            ],
            [(vec![(0, 0)], 0, 3), (vec![], 0, 3)]
        );
    }

    #[test]
    fn a_closed_edit_past_32_bits_is_written_in_an_edit_list_of_version_1() {
        let moov = with_durations(u32::MAX, [timed_track(1, 1, None, Some(&[(0, 0)]))]);
        let elst = moov
            .trak
            .first()
            .and_then(|trak| trak.edts.as_ref())
            .and_then(|edts| edts.elst.as_ref())
            .unwrap();
        assert_eq!(
            (
                elst.version,
                elst.entries.first().map(|entry| entry.edit_duration)
            ),
            (1, Some(3 * u64::from(u32::MAX)))
        );
    }

    #[test]
    fn edits_that_last_longer_than_64_bits_count_together_give_the_longest_duration_there_can_be() {
        let moov = with_durations(
            1000,
            [timed_track(
                1,
                1000,
                None,
                Some(&[(1 << 63, 0), (1 << 63, 0)]),
            )],
        );
        assert_eq!(
            (moov.trak.first().unwrap().tkhd.duration, moov.mvhd.duration),
            (u64::MAX, u64::MAX)
        );
    }

    #[test]
    fn a_header_takes_version_1_where_its_duration_or_a_time_does_not_fit_32_bits() {
        let versions = |moov: &MovieBox| {
            let trak = moov.trak.first().unwrap();
            [moov.mvhd.version, trak.tkhd.version, trak.mdia.mdhd.version]
        };
        let mut timed = timed_track(1, 1000, None, None);
        timed.tkhd.creation_time = 1 << 32;
        timed.mdia.mdhd.modification_time = 1 << 32;
        let mut moov = movie([timed]);
        moov.mvhd.timescale = 1000;
        moov.mvhd.creation_time = 1 << 32;
        declare_durations(&mut moov);
        assert_eq!(
            [
                versions(&with_durations(1000, [timed_track(1, 1000, None, None)])),
                versions(&with_durations(
                    1000,
                    [timed_track(1, 1, None, Some(&[(1 << 32, 0)]))]
                )),
                versions(&moov),
            ],
            [[0, 0, 0], [1, 1, 0], [1, 1, 1]]
        );
    }

    #[test]
    fn the_movie_lasts_as_long_as_its_longest_track_and_keeps_its_next_track_id() {
        let mut moov = movie([
            timed_track(1, 1000, None, None),
            timed_track(2, 1000, None, Some(&[(500, -1), (3, 0)])),
        ]);
        (moov.mvhd.timescale, moov.mvhd.next_track_id) = (1000, 9);
        declare_durations(&mut moov);
        assert_eq!((moov.mvhd.duration, moov.mvhd.next_track_id), (503, 9));
    }

    /// The edit list of the edits `(edit_duration, media_time)`
    fn edit_list(edits: &[(u64, i64)]) -> EditListBox {
        let mut elst = EditListBox::default();
        elst.entries = edits
            .iter()
            .map(|&(edit_duration, media_time)| {
                let mut entry = EditListEntry::default();
                (entry.edit_duration, entry.media_time) = (edit_duration, media_time);
                entry
            })
            .collect();
        elst
    }

    #[test]
    fn edits_that_are_not_empty_start_as_much_sooner_as_the_media_does() {
        let mut elst = edit_list(&[(500, -1), (100, 1_000), (0, 1_200)]);
        move_media_times(&mut elst, 1_000, 300).unwrap();
        assert_eq!(elst, edit_list(&[(500, -1), (100, 0), (0, 200)]));
    }

    #[test]
    fn edits_of_media_that_starts_where_it_did_are_left_as_they_are() {
        let mut elst = edit_list(&[(500, -1), (100, 7)]);
        move_media_times(&mut elst, 0, 10).unwrap();
        assert_eq!(elst, edit_list(&[(500, -1), (100, 7)]));
    }

    #[test]
    fn an_edit_that_would_start_before_the_media_or_where_its_composition_has_ended_is_refused_and_no_edit_moves()
     {
        let before = edit_list(&[(100, 1_000), (100, 999)]);
        let after = edit_list(&[(100, 1_000), (100, 1_300)]);
        let mut moved = [before.clone(), after.clone()];
        let refusals = [
            move_media_times(&mut moved[0], 1_000, 300),
            move_media_times(&mut moved[1], 1_000, 300),
        ];
        assert_eq!(
            (refusals, moved),
            (
                [
                    Err(Error::EditOutsideMedia {
                        media_time: 999,
                        media_start: 1_000,
                        media_end: 1_300
                    }),
                    Err(Error::EditOutsideMedia {
                        media_time: 1_300,
                        media_start: 1_000,
                        media_end: 1_300
                    }),
                ],
                [before, after]
            )
        );
    }

    /// A movie of 1,000 units a second, of track 1 of media of `media_timescale` lasting `duration` in the `tkhd`, whose edits are `(edit_duration, media_time, media_rate_integer)` (no edit list where `None`), repeated where `repeat_edits`
    fn edited_movie(
        media_timescale: u32,
        edits: Option<&[(u64, i64, i16)]>,
        repeat_edits: bool,
        duration: u64,
    ) -> MovieBox {
        let mut trak = track(1, 1, &[], &[], &[]);
        trak.mdia.mdhd.timescale = media_timescale;
        trak.tkhd.duration = duration;
        trak.edts = edits.map(|edits| {
            let mut elst = EditListBox::default();
            elst.repeat_edits = repeat_edits;
            elst.entries = edits
                .iter()
                .map(|&(edit_duration, media_time, media_rate_integer)| {
                    let mut entry = EditListEntry::default();
                    (entry.edit_duration, entry.media_time) = (edit_duration, media_time);
                    entry.media_rate_integer = media_rate_integer;
                    entry
                })
                .collect();
            let mut edts = EditBox::default();
            edts.elst = Some(elst);
            edts
        });
        let mut moov = movie([trak]);
        moov.mvhd.timescale = 1_000;
        moov
    }

    /// Samples of track 1, each `(decoding_time, sample_duration, sample_composition_time_offset)`
    fn samples(times: &[(u64, u32, i64)]) -> Vec<SampleProperties> {
        times
            .iter()
            .map(
                |&(decoding_time, sample_duration, sample_composition_time_offset)| {
                    SampleProperties {
                        track_id: 1,
                        decoding_time,
                        sample_duration,
                        sample_composition_time_offset,
                        sample_flags: SampleFlags::default(),
                        sample_description_index: 1,
                    }
                },
            )
            .collect()
    }

    /// `(index, presentation_time, duration, composition_time, dwell)` of each sample of track 1 of `movie` as its edits present `samples`
    type Presented = Vec<(usize, u64, u64, u64, bool)>;

    fn presented(movie: &MovieBox, samples: Vec<SampleProperties>) -> Result<Presented, Error> {
        Ok(presentation(movie, 1, samples)?
            .map(|sample| {
                (
                    sample.index,
                    sample.presentation_time,
                    sample.duration,
                    sample.composition_time,
                    sample.dwell,
                )
            })
            .collect())
    }

    #[test]
    fn without_an_edit_list_the_composition_timeline_is_presented_from_0() {
        assert_eq!(
            presented(
                &edited_movie(1_000, None, false, 0),
                samples(&[(10, 10, 0), (0, 10, -5)])
            ),
            Ok(vec![(1, 0, 5, 0, false), (0, 10, 10, 10, false)])
        );
    }

    #[test]
    fn an_empty_edit_presents_nothing_and_an_edit_cuts_the_samples_it_starts_and_ends_in() {
        let movie = edited_movie(1_000, Some(&[(500, -1, 1), (150, 50, 1)]), false, 0);
        assert_eq!(
            presented(
                &movie,
                samples(&[(0, 100, 0), (100, 100, 0), (200, 100, 0)])
            ),
            Ok(vec![(0, 500, 50, 50, false), (1, 550, 100, 100, false)])
        );
    }

    #[test]
    fn media_an_edit_maps_past_the_last_sample_presents_nothing() {
        let movie = edited_movie(1_000, Some(&[(1_000, 0, 1)]), false, 0);
        assert_eq!(
            presented(&movie, samples(&[(0, 100, 0), (100, 100, 0)])),
            Ok(vec![(0, 0, 100, 0, false), (1, 100, 100, 100, false)])
        );
    }

    #[test]
    fn a_dwell_holds_the_sample_composed_at_its_media_time() {
        let movie = edited_movie(1_000, Some(&[(500, 150, 0), (100, 0, 2)]), false, 0);
        assert_eq!(
            presented(&movie, samples(&[(0, 100, 0), (100, 100, 0)])),
            Ok(vec![(1, 0, 500, 150, true), (0, 500, 100, 0, false)])
        );
    }

    #[test]
    fn repeated_edits_are_presented_until_the_duration_of_the_track_and_cut_there() {
        let movie = edited_movie(1_000, Some(&[(200, 0, 1)]), true, 450);
        assert_eq!(
            presented(&movie, samples(&[(0, 100, 0), (100, 100, 0)])),
            Ok(vec![
                (0, 0, 100, 0, false),
                (1, 100, 100, 100, false),
                (0, 200, 100, 0, false),
                (1, 300, 100, 100, false),
                (0, 400, 50, 0, false),
            ])
        );
    }

    #[test]
    fn repeated_edits_of_a_track_of_indefinite_duration_are_presented_without_end() {
        let movie = edited_movie(1_000, Some(&[(100, 0, 1)]), true, u64::MAX);
        let presented: Vec<_> = presentation(&movie, 1, samples(&[(0, 100, 0)]))
            .unwrap()
            .take(3)
            .map(|sample| sample.presentation_time)
            .collect();
        assert_eq!(presented, [0, 100, 200]);
    }

    #[test]
    fn an_edit_list_that_presents_no_sample_is_not_repeated() {
        let movie = edited_movie(1_000, Some(&[(100, -1, 1), (100, 500, 1)]), true, u64::MAX);
        assert_eq!(presented(&movie, samples(&[(0, 100, 0)])), Ok(vec![]));
    }

    #[test]
    fn the_bounds_of_the_edits_are_their_summed_durations_in_the_media_timescale_to_the_nearest_unit()
     {
        // Why 3 edits of 1 in a movie of 1,000 into media of 1,500: each lasts
        // 1.5 media units, so the sums 1.5, 3 and 4.5 round to 2, 3 and 5, where
        // each edit rounded alone would end at 2, 4 and 6.
        let movie = edited_movie(1_500, Some(&[(1, 0, 1), (1, 0, 1), (1, 0, 1)]), false, 0);
        assert_eq!(
            presented(&movie, samples(&[(0, 10, 0)])),
            Ok(vec![
                (0, 0, 2, 0, false),
                (0, 2, 1, 0, false),
                (0, 3, 2, 0, false),
            ])
        );
    }

    #[test]
    fn in_a_fragmented_movie_a_last_edit_of_duration_0_runs_to_the_end_of_the_media() {
        let mut movie = edited_movie(1_000, Some(&[(0, 10, 1)]), false, 0);
        movie.mvex = Some(MovieExtendsBox::default());
        assert_eq!(
            presented(&movie, samples(&[(0, 10, 0), (10, 10, 0), (20, 10, 0)])),
            Ok(vec![(1, 0, 10, 10, false), (2, 10, 10, 20, false)])
        );
    }

    #[test]
    fn in_a_fragmented_movie_the_media_past_the_edits_follows_them() {
        let mut movie = edited_movie(1_000, Some(&[(5, -1, 1), (10, 0, 1)]), false, 0);
        movie.mvex = Some(MovieExtendsBox::default());
        assert_eq!(
            presented(&movie, samples(&[(0, 10, 0), (10, 10, 0), (20, 10, 0)])),
            Ok(vec![
                (0, 5, 10, 0, false),
                (1, 15, 10, 10, false),
                (2, 25, 10, 20, false),
            ])
        );
    }

    #[test]
    fn a_presentation_past_what_a_u64_counts_ends() {
        let movie = edited_movie(1_000, Some(&[(u64::MAX, -1, 1), (100, 0, 1)]), false, 0);
        assert_eq!(presented(&movie, samples(&[(0, 100, 0)])), Ok(vec![]));
    }

    #[test]
    fn a_presentation_is_of_a_track_of_the_movie_and_of_its_samples_alone() {
        let movie = edited_movie(1_000, None, false, 0);
        let mut other = samples(&[(0, 10, 0)]);
        other.first_mut().unwrap().track_id = 2;
        let mut zero = edited_movie(0, Some(&[(100, 0, 1)]), false, 0);
        zero.mvhd.timescale = 1_000;
        assert_eq!(
            [
                presented(&movie, other).map(|_| ()),
                presentation(&movie, 3, Vec::new()).map(|_| ()),
                presented(&zero, Vec::new()).map(|_| ()),
            ],
            [
                Err(Error::OtherTrack {
                    track_id: 1,
                    found: 2
                }),
                Err(Error::UnknownTrack { track_id: 3 }),
                Err(Error::ZeroTimescale {
                    movie_timescale: 1_000,
                    media_timescale: 0
                }),
            ]
        );
    }

    /// The `moov` of `file` and the location of each of its samples and of those of its movie fragments, as a demuxer reports them
    fn structure_of(file: &[u8]) -> (Option<MovieBox>, Vec<SampleLocation>) {
        use isobmff::demuxer::Event;
        use isobmff::{Demuxer, DemuxerConfig};

        let mut demuxer = Demuxer::new(DemuxerConfig::default());
        let (mut movie, mut locations) = (None, Vec::new());
        loop {
            while let Some(event) = demuxer.poll_event() {
                if let Event::Movie(moov) = event {
                    movie = Some(*moov);
                } else if let Event::SampleLocation(location) = event {
                    locations.push(location);
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
        (movie, locations)
    }

    /// `(index, presentation_time, duration, composition_time, dwell)` of each sample of the video track, track 1, of `file` as its edits present it, at most `most`
    fn presented_video(file: &[u8], most: usize) -> Presented {
        let (movie, locations) = structure_of(file);
        let movie = movie.unwrap();
        let video = locations
            .iter()
            .filter(|location| location.properties.track_id == 1)
            .map(|location| location.properties);
        presentation(&movie, 1, video)
            .unwrap()
            .take(most)
            .map(|sample| {
                (
                    sample.index,
                    sample.presentation_time,
                    sample.duration,
                    sample.composition_time,
                    sample.dwell,
                )
            })
            .collect()
    }

    /// The video of the fixtures FFmpeg wrote, of samples of 1024 in media of 10,240 units a second, in a movie of 48,000, with edits xtask changed
    ///
    /// ffprobe reads each otherwise: it discards the samples an edit cuts,
    /// and presents from the first whole sample at 0; it plays a dwell, of
    /// `media_rate` 0, forward from the first whole sample after its
    /// `media_time`; and it presents an edit list of `RepeatEdits` once.
    #[test]
    fn the_edits_of_the_fixtures_cut_hold_repeat_and_delay_their_video() {
        assert_eq!(
            [
                presented_video(include_bytes!("fixtures/edit_cut.mp4"), 32),
                presented_video(include_bytes!("fixtures/edit_dwell.mp4"), 32),
                presented_video(include_bytes!("fixtures/edit_repeat.mp4"), 32),
            ],
            [
                vec![
                    (1, 0, 512, 1_536, false),
                    (2, 512, 1_024, 2_048, false),
                    (3, 1_536, 1_024, 3_072, false),
                    (4, 2_560, 1_024, 4_096, false),
                    (5, 3_584, 1_024, 5_120, false),
                    (6, 4_608, 512, 6_144, false),
                ],
                vec![(2, 0, 10_240, 2_560, true)],
                vec![
                    (0, 0, 1_024, 0, false),
                    (1, 1_024, 1_024, 1_024, false),
                    (0, 2_048, 1_024, 0, false),
                    (1, 3_072, 1_024, 1_024, false),
                    (0, 4_096, 1_024, 0, false),
                ],
            ]
        );
    }

    /// The segments FFmpeg wrote, whose video starts after an empty edit of 1,026 of the movie timescale, 218.9 of the media, and runs to the end of the media in an edit of duration 0
    #[test]
    fn an_empty_edit_delays_the_video_of_the_segments_by_its_duration_to_the_nearest_unit() {
        let expected: Presented = (0..12_u64)
            .map(|sample| {
                (
                    usize::try_from(sample).unwrap(),
                    219 + 1_024 * sample,
                    1_024,
                    1_024 * sample,
                    false,
                )
            })
            .collect();
        assert_eq!(
            presented_video(include_bytes!("fixtures/segment.mp4"), 32),
            expected
        );
    }

    /// How the presentation times of a track differ from the `pts` ffprobe reports for the packets it does not discard: by how much ffprobe's are later, and the file offsets of the samples only one of them presents
    #[derive(Debug, PartialEq, Eq)]
    struct Difference {
        later: i128,
        only_presented: Vec<u64>,
        only_ffprobe: Vec<u64>,
    }

    /// The presentation times of each track with an edit list of each conformance file, as they differ from ffprobe's
    ///
    /// Two files differ, each by a rule ffprobe reads otherwise:
    ///
    /// * `a8-foreman_QCIF_edit.mp4`, video: its edit starts inside a sample.
    ///   The edit cuts it and presents the rest (§8.6.6.1 NOTE 1); FFmpeg
    ///   discards the sample and starts at the next, 15 units of media
    ///   later.
    /// * `21_segment.mp4`, fragmented: its last edit lasts 0 from
    ///   `media_time` 8, and runs to the end of the media (§8.6.6.1); ffprobe
    ///   reports the composition times without it, 8 units later, and
    ///   presents the 2 samples whose composition ends by 8.
    #[test]
    #[ignore = "needs the conformance files and ffprobe"]
    fn the_conformance_files_with_edit_lists_present_their_samples_at_the_times_ffmpeg_reads() {
        use isobmff_test_support::{conformance_files, ffprobe_packets};
        use std::collections::BTreeMap;

        let mut found = BTreeMap::new();
        for file in conformance_files() {
            let (Some(movie), locations) = structure_of(&file.bytes) else {
                continue;
            };
            let edited: Vec<_> = movie
                .trak
                .iter()
                .filter(|trak| {
                    trak.edts
                        .as_ref()
                        .and_then(|edts| edts.elst.as_ref())
                        .is_some()
                })
                .collect();
            if edited.is_empty() {
                continue;
            }
            let packets = ffprobe_packets(&[], &file.path);
            for trak in edited {
                let track_id = trak.tkhd.track_id;
                let samples: Vec<_> = locations
                    .iter()
                    .filter(|location| location.properties.track_id == track_id)
                    .copied()
                    .collect();
                let mut presented = BTreeMap::new();
                for sample in presentation(
                    &movie,
                    track_id,
                    samples.iter().map(|location| location.properties),
                )
                .unwrap()
                .take(samples.len().saturating_mul(4))
                {
                    let location = samples.get(sample.index).unwrap();
                    let composed = i128::from(location.properties.decoding_time)
                        + i128::from(location.properties.sample_composition_time_offset);
                    let uncut = i128::from(sample.presentation_time)
                        - (i128::from(sample.composition_time) - composed);
                    presented.entry(location.offset).or_insert(uncut);
                }
                let ffprobe: BTreeMap<_, _> = packets
                    .iter()
                    .filter(|packet| packet.track_id == track_id && !packet.discard)
                    .filter_map(|packet| Some((packet.pos, i128::from(packet.pts?))))
                    .collect();
                let mut later: Vec<_> = presented
                    .iter()
                    .filter_map(|(pos, time)| Some(ffprobe.get(pos)? - time))
                    .collect();
                later.dedup();
                let difference = Difference {
                    later: match later.as_slice() {
                        [] => 0,
                        [later] => *later,
                        _ => i128::MAX,
                    },
                    only_presented: presented
                        .keys()
                        .filter(|pos| !ffprobe.contains_key(pos))
                        .copied()
                        .collect(),
                    only_ffprobe: ffprobe
                        .keys()
                        .filter(|pos| !presented.contains_key(pos))
                        .copied()
                        .collect(),
                };
                found.insert((file.name.clone(), track_id), difference);
            }
        }
        let same = || Difference {
            later: 0,
            only_presented: vec![],
            only_ffprobe: vec![],
        };
        let expected = BTreeMap::from([
            (("17_negative_ctso.mp4".to_owned(), 1), same()),
            (
                ("21_segment.mp4".to_owned(), 1),
                Difference {
                    later: 8,
                    only_presented: vec![],
                    only_ffprobe: vec![11956, 15237],
                },
            ),
            (
                ("a8-foreman_QCIF_edit.mp4".to_owned(), 1),
                Difference {
                    later: -15,
                    only_presented: vec![156765],
                    only_ffprobe: vec![],
                },
            ),
            (("a9-aac-samplegroups-edit.mp4".to_owned(), 1), same()),
            (("restricted.mp4".to_owned(), 1), same()),
        ]);
        assert_eq!(found, expected);
    }
}
