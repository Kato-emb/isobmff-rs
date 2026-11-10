//! The durations a `moov` declares, set from its sample tables and edit lists, and the movie extends box of its tracks

#[cfg(test)]
mod tests {
    use isobmff::structure_data::{movie_extends_box, set_durations};
    use isobmff_boxes::{
        CompositionOffsetBox, CompositionOffsetEntry, EditBox, EditListBox, EditListEntry,
        MovieBox, TrackBox,
    };
    use isobmff_test_support::fixtures::{movie, track};

    /// Track `track_id` of three samples lasting 1 each in `timescale`, whose composition offsets are `offsets` and whose edits are `(edit_duration, media_time)`
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
                    entry.sample_count = 1;
                    entry.sample_offset = offset;
                    entry
                })
                .collect();
            ctts
        });
        trak.edts = edits.map(|edits| {
            let mut elst = EditListBox::default();
            elst.entries = edits
                .iter()
                .map(|&(edit_duration, media_time)| {
                    let mut entry = EditListEntry::default();
                    entry.edit_duration = edit_duration;
                    entry.media_time = media_time;
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

    /// The movie of `trak` in `timescale`, its durations set
    fn with_durations(timescale: u32, trak: TrackBox) -> MovieBox {
        let mut moov = movie([trak]);
        moov.mvhd.timescale = timescale;
        set_durations(&mut moov);
        moov
    }

    /// The `(edit_duration, media_time)` of each edit of the first track of `moov`, the duration of the track and of its media
    fn edits(moov: &MovieBox) -> (Vec<(u64, i64)>, u64, u64) {
        let trak = moov.trak.first().unwrap();
        let entries = trak
            .edts
            .iter()
            .filter_map(|edts| edts.elst.as_ref())
            .flat_map(|elst| elst.entries.iter())
            .map(|entry| (entry.edit_duration, entry.media_time))
            .collect();
        (entries, trak.tkhd.duration, trak.mdia.mdhd.duration)
    }

    #[test]
    fn a_last_edit_of_duration_0_lasts_to_the_end_of_the_composition_of_the_samples() {
        // Composed at 1, 3 and 2 until 2, 4 and 3: the composition ends at 4,
        // past the 3 their durations add up to.
        let moov = with_durations(1000, timed_track(1, 1000, Some([1, 2, 0]), Some(&[(0, 1)])));
        assert_eq!(edits(&moov), (vec![(3, 1)], 3, 3));
    }

    #[test]
    fn a_last_edit_of_duration_0_without_composition_offsets_lasts_to_the_end_of_the_media() {
        let moov = with_durations(1000, timed_track(1, 1000, None, Some(&[(500, -1), (0, 1)])));
        assert_eq!(edits(&moov), (vec![(500, -1), (2, 1)], 502, 3));
    }

    #[test]
    fn a_closed_edit_is_counted_in_the_movie_timescale_rounded_up() {
        let moov = with_durations(1000, timed_track(1, 3, None, Some(&[(0, 0)])));
        assert_eq!(edits(&moov), (vec![(1000, 0)], 1000, 3));
        let moov = with_durations(1000, timed_track(1, 7, None, Some(&[(0, 0)])));
        assert_eq!(edits(&moov), (vec![(429, 0)], 429, 3));
    }

    #[test]
    fn edits_of_a_duration_and_tracks_without_an_edit_list_keep_their_durations() {
        let moov = with_durations(1000, timed_track(1, 1000, None, Some(&[(7, 1)])));
        assert_eq!(edits(&moov), (vec![(7, 1)], 7, 3));
        let moov = with_durations(1000, timed_track(1, 10, None, None));
        assert_eq!(edits(&moov), (vec![], 300, 3));
    }

    #[test]
    fn a_closed_edit_past_32_bits_is_written_in_an_edit_list_of_version_1() {
        let moov = with_durations(u32::MAX, timed_track(1, 1, None, Some(&[(0, 0)])));
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
    fn a_header_takes_version_1_where_its_duration_or_a_time_does_not_fit_32_bits() {
        let versions = |moov: &MovieBox| {
            let trak = moov.trak.first().unwrap();
            [moov.mvhd.version, trak.tkhd.version, trak.mdia.mdhd.version]
        };
        let mut trak = timed_track(1, 1000, None, None);
        trak.tkhd.creation_time = 1 << 32;
        trak.mdia.mdhd.modification_time = 1 << 32;
        let mut moov = movie([trak]);
        moov.mvhd.timescale = 1000;
        moov.mvhd.creation_time = 1 << 32;
        set_durations(&mut moov);
        assert_eq!(versions(&moov), [1, 1, 1]);
        assert_eq!(
            versions(&with_durations(1000, timed_track(1, 1000, None, None))),
            [0, 0, 0]
        );
        let moov = with_durations(1000, timed_track(1, 1, None, Some(&[(1 << 32, 0)])));
        assert_eq!(versions(&moov), [1, 1, 0]);
    }

    #[test]
    fn the_movie_lasts_as_long_as_its_longest_track() {
        let mut moov = movie([
            timed_track(1, 1000, None, None),
            timed_track(2, 1000, None, Some(&[(500, -1), (3, 0)])),
        ]);
        moov.mvhd.timescale = 1000;
        set_durations(&mut moov);
        assert_eq!(moov.mvhd.duration, 503);
    }

    #[test]
    fn the_movie_extends_box_has_a_track_extends_box_for_each_track_whose_default_sample_entry_is_the_first()
     {
        let mvex = movie_extends_box(&[track(3, 1, &[], &[], &[]), track(5, 1, &[], &[], &[])]);
        assert_eq!(
            mvex.trex
                .iter()
                .map(|trex| (trex.track_id, trex.default_sample_description_index))
                .collect::<Vec<_>>(),
            [(3, 1), (5, 1)]
        );
    }
}
