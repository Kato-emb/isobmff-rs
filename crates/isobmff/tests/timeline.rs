//! How long the media, the tracks and the movie of a `moov` last, as their sample tables and edit lists give it

#[cfg(test)]
mod tests {
    use isobmff::timeline::{Error, declare_durations, move_media_times};
    use isobmff_boxes::{
        CompositionOffsetBox, CompositionOffsetEntry, EditBox, EditListBox, EditListEntry,
        MovieBox, TrackBox,
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
}
