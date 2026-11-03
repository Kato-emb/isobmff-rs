//! [`Timeline`], which presents the samples of a track where the edit list of the track maps their composition times on the presentation timeline
//!
//! A track's media lies on its composition timeline: a sample is composed
//! at its decoding time plus its composition time offset (§8.6.1.3), for
//! its duration. The edit list of the track maps parts of that timeline
//! onto the presentation timeline, which starts at 0 (§8.6.5, §8.6.6). A
//! [`Timeline`] returns, for a sample, the [`Presentation`]s of it that the
//! edits make: none where no edit presents it, one each time an edit does.
//!
//! The demuxer returns samples at their decoding times; a caller that
//! presents them, or states their presentation times, maps them here.
//!
//! ```
//! use isobmff::boxes::{EditBox, EditListBox, EditListEntry, MovieBox, TrackBox};
//! use isobmff::boxes::SampleFlags;
//! use isobmff::timeline::Timeline;
//! use isobmff::SampleProperties;
//!
//! // A track of timescale 1000 in a movie of timescale 600, presented from
//! // media time 500 after one second with nothing
//! let mut movie = MovieBox::default();
//! movie.mvhd.timescale = 600;
//! let mut trak = TrackBox::default();
//! trak.tkhd.track_id = 1;
//! trak.mdia.mdhd.timescale = 1000;
//! let mut empty = EditListEntry::default();
//! (empty.edit_duration, empty.media_time, empty.media_rate_integer) = (600, -1, 1);
//! let mut edit = EditListEntry::default();
//! (edit.edit_duration, edit.media_time, edit.media_rate_integer) = (1200, 500, 1);
//! let mut elst = EditListBox::default();
//! elst.entries = vec![empty, edit];
//! let mut edts = EditBox::default();
//! edts.elst = Some(elst);
//! trak.edts = Some(edts);
//! movie.trak.push(trak);
//!
//! let timeline = Timeline::new(&movie, 1)?;
//! let sample = |decoding_time| SampleProperties {
//!     track_id: 1,
//!     decoding_time,
//!     sample_duration: 400,
//!     sample_composition_time_offset: 0,
//!     sample_flags: SampleFlags::default(),
//!     sample_description_index: 1,
//! };
//! // Before the edit, the sample is not presented
//! assert_eq!(timeline.present(&sample(0)).count(), 0);
//! // The edit enters this sample 100 into it, and presents the rest of it
//! let presented: Vec<_> = timeline.present(&sample(400)).collect();
//! let times: Vec<_> = presented.iter().map(|p| (p.presentation_time(), p.duration(), p.media_time())).collect();
//! assert_eq!(times, [(1000, 300, 500)]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

use core::iter::FusedIterator;
use core::ops::{ControlFlow, Range};

use isobmff_boxes::{EditListEntry, MovieBox, TrackBox};

use crate::error::TimelineError;
use crate::sample::SampleProperties;

/// Mapping of the composition timeline of a track onto the presentation timeline, by the edit list of the track, ISO/IEC 14496-12 §8.6.6
///
/// The times of edits count in the timescale of the movie, the times a
/// timeline returns in the timescale of the media of the track. Each
/// boundary between edits is the sum of the edit durations before it, in
/// the timescale of the movie, rounded to the nearest tick of the media,
/// half a tick up.
///
/// * An empty edit presents nothing for its duration.
/// * An edit at `media_rate` 0, a dwell, presents the sample composed at
///   its `media_time` for its duration.
/// * Any other edit presents the composition timeline from its
///   `media_time` for its duration. A sample it enters or leaves partway is
///   presented for the part within it (§8.6.6 NOTE 1). Its rate is taken as
///   1, as FFmpeg and GPAC take it.
/// * Where `RepeatEdits` is set, the edit list repeats until the duration
///   of the `tkhd`, or without end where that is indefinite.
/// * A track with no edit list, or none with an entry, maps its composition
///   timeline one to one: a sample composed before time 0 is presented
///   from 0, for its part after it, or not at all.
/// * In a fragmented movie, one with an `mvex`, a last edit of duration 0
///   that is not empty lasts to the end of the media, and an edit after the
///   edit list presents the media past the last edit's from where that
///   edit ends (§8.6.6.1).
#[derive(Clone, Debug)]
pub struct Timeline<'a> {
    edits: &'a [EditListEntry],
    movie_timescale: u32,
    media_timescale: u32,
    repeat: Repeat,
    open_last_edit: bool,
    /// Composition time the edit after the edit list presents from, where there is one
    implied_media_time: Option<i128>,
}

/// How often the edit list of a track is presented
#[derive(Clone, Copy, Debug)]
enum Repeat {
    Once,
    /// Until the movie time the `tkhd` states as the duration of the track
    Until(u128),
    Indefinitely,
}

impl<'a> Timeline<'a> {
    /// Returns the timeline of the track of `track_id` of `movie`
    ///
    /// # Errors
    ///
    /// * [`UnknownTrackId`](TimelineError::UnknownTrackId): `movie` has no
    ///   track of `track_id`.
    /// * [`InvalidMediaTime`](TimelineError::InvalidMediaTime): an edit is at
    ///   a negative `media_time` other than the −1 of an empty edit.
    /// * [`MissingMovieTimescale`](TimelineError::MissingMovieTimescale): the
    ///   track has an edit list, and the `mvhd` a timescale of 0.
    pub fn new(movie: &'a MovieBox, track_id: u32) -> Result<Self, TimelineError> {
        let trak = movie
            .trak
            .iter()
            .find(|trak| trak.tkhd.track_id == track_id)
            .ok_or(TimelineError::UnknownTrackId { track_id })?;
        let edits = edits(trak);
        if let Some(edit) = edits.iter().find(|edit| edit.media_time < -1) {
            return Err(TimelineError::InvalidMediaTime {
                track_id,
                media_time: edit.media_time,
            });
        }
        let movie_timescale = movie.mvhd.timescale;
        if !edits.is_empty() && movie_timescale == 0 {
            return Err(TimelineError::MissingMovieTimescale { track_id });
        }
        let fragmented = movie.mvex.is_some();
        let indefinite = trak.tkhd.duration == u64::MAX
            || (trak.tkhd.version == 0 && trak.tkhd.duration == u64::from(u32::MAX));
        let repeat_edits = trak
            .edts
            .as_ref()
            .and_then(|edts| edts.elst.as_ref())
            .is_some_and(|elst| elst.repeat_edits);
        let repeat = match (repeat_edits, indefinite) {
            (false, _) => Repeat::Once,
            (true, true) => Repeat::Indefinitely,
            (true, false) => Repeat::Until(u128::from(trak.tkhd.duration)),
        };
        let open_last_edit = fragmented
            && edits
                .last()
                .is_some_and(|edit| edit.edit_duration == 0 && edit.media_time >= 0);

        let mut timeline = Self {
            edits,
            movie_timescale,
            media_timescale: trak.mdia.mdhd.timescale,
            repeat,
            open_last_edit,
            implied_media_time: None,
        };
        if edits.is_empty() || (fragmented && !matches!(repeat, Repeat::Indefinitely)) {
            let mut start = 0_u128;
            let mut media_end = 0_i128;
            for edit in edits {
                let end = start.saturating_add(u128::from(edit.edit_duration));
                if edit.media_time >= 0 {
                    let width = match timeline.to_media(end).zip(timeline.to_media(start)) {
                        Some((end, start)) if !is_dwell(edit) => end.saturating_sub(start),
                        _ => 0,
                    };
                    media_end = i128::from(edit.media_time)
                        .saturating_add(i128::try_from(width).unwrap_or(i128::MAX));
                }
                start = end;
            }
            timeline.implied_media_time = Some(media_end);
        }

        Ok(timeline)
    }

    /// Returns the presentations of the sample of `properties`, in the order of their presentation times
    ///
    /// The sample is composed from its decoding time plus its composition
    /// time offset, for its duration. A sample of duration 0 is presented,
    /// for no time, by an edit whose part of the composition timeline holds
    /// its composition time. A presentation whose time would pass 64 bits is
    /// not returned, nor any after it.
    ///
    /// The presentations come as they are taken. Where the edit list repeats
    /// without end and presents the sample, they go on until their times
    /// pass 64 bits: a caller takes those up to the presentation time it
    /// wants, as with [`Iterator::take_while`].
    #[must_use]
    pub fn present(&self, properties: &SampleProperties) -> Presentations<'_> {
        let start = composition_time(properties);
        Presentations {
            timeline: self,
            composition: start..start.saturating_add(i128::from(properties.sample_duration)),
            edit: 0,
            edit_start: 0,
            pass_start: 0,
            presented_in_pass: false,
            done: false,
        }
    }

    /// Returns `movie_time`, in the timescale of the movie, in that of the media, rounded to the nearest tick
    fn to_media(&self, movie_time: u128) -> Option<u128> {
        // Why not divide at 0 too: a track without an edit list maps one to one from movie time 0, whatever the timescale of the movie.
        if movie_time == 0 {
            return Some(0);
        }
        let movie_timescale = u128::from(self.movie_timescale);
        movie_time
            .checked_mul(u128::from(self.media_timescale))?
            .checked_add(movie_timescale / 2)?
            .checked_div(movie_timescale)
    }

    /// Returns the presentation of the part of `composition` that an edit from `media_time`, a dwell where `dwell`, presents from movie time `start` to `end`, `None` for no end, or breaks where its times pass 64 bits
    fn presentation(
        &self,
        media_time: i128,
        dwell: bool,
        (start, end): (u128, Option<u128>),
        composition: &Range<i128>,
    ) -> ControlFlow<(), Option<Presentation>> {
        let past_range = ControlFlow::Break(());
        let Some(presentation_start) = self.to_media(start) else {
            return past_range;
        };
        let presentation_end = match end.map(|end| self.to_media(end)) {
            Some(None) => return past_range,
            Some(Some(end)) => Some(end),
            None => None,
        };
        let width = presentation_end.map(|end| end.saturating_sub(presentation_start));
        let media_end = width
            .map(|width| media_time.saturating_add(i128::try_from(width).unwrap_or(i128::MAX)));
        let presented = if dwell {
            let holds = composition.contains(&media_time)
                || (composition.is_empty() && composition.start == media_time);
            holds.then(|| (presentation_start, width.unwrap_or_default(), media_time))
        } else {
            let from = composition.start.max(media_time);
            let to = media_end.map_or(composition.end, |end| composition.end.min(end));
            let instant = composition.is_empty()
                && from == composition.start
                && media_end.is_none_or(|end| from < end);
            (from < to || instant).then(|| {
                let into_edit = u128::try_from(from.saturating_sub(media_time)).unwrap_or_default();
                let duration = u128::try_from(to.saturating_sub(from)).unwrap_or_default();
                (presentation_start.saturating_add(into_edit), duration, from)
            })
        };
        let Some((presentation_time, duration, media_time)) = presented else {
            return ControlFlow::Continue(None);
        };
        match (u64::try_from(presentation_time), u64::try_from(media_time)) {
            (Ok(presentation_time), Ok(media_time)) => ControlFlow::Continue(Some(Presentation {
                presentation_time,
                duration: u64::try_from(duration).unwrap_or(u64::MAX),
                media_time,
            })),
            _ => past_range,
        }
    }
}

/// Returns the entries of the edit list of `trak`, none where it has no edit list
pub(crate) fn edits(trak: &TrackBox) -> &[EditListEntry] {
    trak.edts
        .as_ref()
        .and_then(|edts| edts.elst.as_ref())
        .map_or(&[][..], |elst| elst.entries.as_slice())
}

/// Returns the composition time of the sample of `properties`: its decoding time plus its composition time offset (§8.6.1.3)
pub(crate) fn composition_time(properties: &SampleProperties) -> i128 {
    i128::from(properties.decoding_time)
        .saturating_add(i128::from(properties.sample_composition_time_offset))
}

/// Returns whether `edit` is a dwell: its `media_rate` is 0
const fn is_dwell(edit: &EditListEntry) -> bool {
    edit.media_rate_integer == 0 && edit.media_rate_fraction == 0
}

/// Presentations of a sample, in the order of their presentation times, that [`Timeline::present`] returns
#[derive(Clone, Debug)]
pub struct Presentations<'t> {
    timeline: &'t Timeline<'t>,
    composition: Range<i128>,
    /// Index of the next edit of the edit list
    edit: usize,
    /// Movie time the next edit starts at
    edit_start: u128,
    /// Movie time the current pass over the edit list started at
    pass_start: u128,
    presented_in_pass: bool,
    done: bool,
}

impl Iterator for Presentations<'_> {
    type Item = Presentation;

    fn next(&mut self) -> Option<Presentation> {
        let timeline = self.timeline;
        while !self.done {
            let Some(edit) = timeline.edits.get(self.edit) else {
                let repeats = !matches!(timeline.repeat, Repeat::Once);
                if repeats && self.presented_in_pass && self.edit_start > self.pass_start {
                    self.pass_start = self.edit_start;
                    self.edit = 0;
                    self.presented_in_pass = false;
                    continue;
                }
                return self.implied_edit();
            };
            self.edit = self.edit.saturating_add(1);
            let start = self.edit_start;
            let last = self.edit == timeline.edits.len();
            let mut end = if last && timeline.open_last_edit {
                None
            } else {
                Some(start.saturating_add(u128::from(edit.edit_duration)))
            };
            if let Repeat::Until(until) = timeline.repeat {
                if start >= until {
                    self.edit_start = until;
                    return self.implied_edit();
                }
                end = Some(end.map_or(until, |end| end.min(until)));
            }
            self.edit_start = end.unwrap_or(u128::MAX);
            if edit.media_time == -1 {
                continue;
            }
            let media_time = i128::from(edit.media_time);
            match timeline.presentation(media_time, is_dwell(edit), (start, end), &self.composition)
            {
                ControlFlow::Break(()) => self.done = true,
                ControlFlow::Continue(None) => {}
                ControlFlow::Continue(Some(presentation)) => {
                    self.presented_in_pass = true;
                    return Some(presentation);
                }
            }
        }
        None
    }
}

impl Presentations<'_> {
    /// Returns the presentation of the edit after the edit list, which presents the media past the last edit from the end of the edit list, and ends the presentations
    fn implied_edit(&mut self) -> Option<Presentation> {
        self.done = true;
        let media_time = self.timeline.implied_media_time?;
        let presented = self.timeline.presentation(
            media_time,
            false,
            (self.edit_start, None),
            &self.composition,
        );
        match presented {
            ControlFlow::Continue(presentation) => presentation,
            ControlFlow::Break(()) => None,
        }
    }
}

impl FusedIterator for Presentations<'_> {}

/// Part of the presentation timeline over which a sample is presented, in the timescale of its media
///
/// The media presented first is at [`media_time`](Self::media_time) of
/// the composition timeline: the composition time of the sample, or later
/// where an edit enters the sample partway.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Presentation {
    presentation_time: u64,
    duration: u64,
    media_time: u64,
}

impl Presentation {
    /// Creates the presentation from `presentation_time` for `duration`, of the media from composition time `media_time`
    #[must_use]
    pub const fn new(presentation_time: u64, duration: u64, media_time: u64) -> Self {
        Self {
            presentation_time,
            duration,
            media_time,
        }
    }

    /// Returns the presentation time the sample is presented from
    #[must_use]
    pub const fn presentation_time(&self) -> u64 {
        self.presentation_time
    }

    /// Returns how long the sample is presented
    #[must_use]
    pub const fn duration(&self) -> u64 {
        self.duration
    }

    /// Returns the composition time of the media presented first
    #[must_use]
    pub const fn media_time(&self) -> u64 {
        self.media_time
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_boxes::{EditBox, EditListBox, EditListEntry, MovieBox, MovieExtendsBox, TrackBox};

    use super::{Presentation, Timeline};
    use crate::error::TimelineError;
    use crate::structure_data::fixtures::sample;

    /// Returns a movie of timescale 600 with track 1 of media timescale 1000, its edit list `edits` (edit duration, media time, rate) where there are any
    fn movie(edits: &[(u64, i64, i16)]) -> MovieBox {
        let mut movie = MovieBox::default();
        movie.mvhd.timescale = 600;
        let mut trak = TrackBox::default();
        trak.tkhd.track_id = 1;
        trak.mdia.mdhd.timescale = 1000;
        if !edits.is_empty() {
            let mut elst = EditListBox::default();
            elst.entries = edits
                .iter()
                .map(|&(edit_duration, media_time, media_rate_integer)| {
                    let mut entry = EditListEntry::default();
                    entry.edit_duration = edit_duration;
                    entry.media_time = media_time;
                    entry.media_rate_integer = media_rate_integer;
                    entry
                })
                .collect();
            let mut edts = EditBox::default();
            edts.elst = Some(elst);
            trak.edts = Some(edts);
        }
        movie.trak.push(trak);
        movie
    }

    /// Returns the presentations, on `movie`, of a sample of track 1 composed from `composition_time` for `sample_duration`
    fn present(movie: &MovieBox, composition_time: u64, sample_duration: u32) -> Vec<Presentation> {
        let sample = sample(1, composition_time, sample_duration, 0, true);
        Timeline::new(movie, 1)
            .unwrap()
            .present(sample.properties())
            .collect()
    }

    #[test]
    fn a_track_without_an_edit_list_presents_its_samples_at_their_composition_times() {
        let movie = movie(&[]);

        assert_eq!(
            present(&movie, 500, 100),
            [Presentation::new(500, 100, 500)]
        );
    }

    #[test]
    fn a_track_without_an_edit_list_maps_one_to_one_in_a_movie_of_timescale_0() {
        let mut movie = movie(&[]);
        movie.mvhd.timescale = 0;

        assert_eq!(
            present(&movie, 500, 100),
            [Presentation::new(500, 100, 500)]
        );
    }

    #[test]
    fn a_sample_composed_before_time_0_with_no_edit_list_is_presented_from_0() {
        let movie = movie(&[]);
        let sample = sample(1, 0, 100, -40, true);
        let presented: Vec<_> = Timeline::new(&movie, 1)
            .unwrap()
            .present(sample.properties())
            .collect();

        assert_eq!(presented, [Presentation::new(0, 60, 0)]);
    }

    #[test]
    fn an_empty_edit_first_starts_the_presentation_of_the_track_after_its_duration() {
        // 300 ticks of the movie are 500 of the media.
        let movie = movie(&[(300, -1, 1), (600, 0, 1)]);

        assert_eq!(
            [
                present(&movie, 0, 100),
                present(&movie, 900, 100),
                present(&movie, 1000, 100)
            ],
            [
                Vec::from([Presentation::new(500, 100, 0)]),
                Vec::from([Presentation::new(1400, 100, 900)]),
                Vec::new(),
            ]
        );
    }

    #[test]
    fn a_sample_an_edit_enters_or_leaves_partway_is_presented_for_its_part_within_the_edit() {
        // From media time 250 for 1000 ticks of the media
        let movie = movie(&[(600, 250, 1)]);

        assert_eq!(
            [present(&movie, 200, 100), present(&movie, 1200, 100)],
            [
                Vec::from([Presentation::new(0, 50, 250)]),
                Vec::from([Presentation::new(950, 50, 1200)]),
            ]
        );
    }

    #[test]
    fn a_sample_two_edits_present_is_presented_once_by_each() {
        let movie = movie(&[(60, 0, 1), (60, 0, 1)]);

        assert_eq!(
            present(&movie, 0, 100),
            [Presentation::new(0, 100, 0), Presentation::new(100, 100, 0)]
        );
    }

    #[test]
    fn a_dwell_presents_the_sample_composed_at_its_media_time_for_its_duration() {
        let movie = movie(&[(120, -1, 1), (600, 150, 0)]);

        assert_eq!(
            [present(&movie, 100, 100), present(&movie, 200, 100)],
            [Vec::from([Presentation::new(200, 1000, 150)]), Vec::new(),]
        );
    }

    #[test]
    fn edit_boundaries_round_the_sum_of_the_edit_durations_before_them_to_the_nearest_tick_of_the_media()
     {
        // 1 tick of the movie is 5/3 of the media: 1.67 rounds to 2, 3.33 to 3, 5 to 5.
        let movie = movie(&[(1, 0, 1), (1, 0, 1), (1, 0, 1)]);

        assert_eq!(
            present(&movie, 0, 10),
            [
                Presentation::new(0, 2, 0),
                Presentation::new(2, 1, 0),
                Presentation::new(3, 2, 0),
            ]
        );
    }

    #[test]
    fn a_repeated_edit_list_repeats_until_the_duration_of_the_track() {
        let mut movie = movie(&[(60, 0, 1)]);
        let trak = movie.trak.first_mut().unwrap();
        trak.tkhd.duration = 150;
        if let Some(elst) = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut()) {
            elst.repeat_edits = true;
        }

        // The third repetition ends at 150 ticks of the movie, 250 of the media.
        assert_eq!(
            present(&movie, 0, 100),
            [
                Presentation::new(0, 100, 0),
                Presentation::new(100, 100, 0),
                Presentation::new(200, 50, 0),
            ]
        );
    }

    #[test]
    fn an_edit_list_repeated_indefinitely_presents_a_sample_again_and_again() {
        let mut movie = movie(&[(60, 0, 1)]);
        let trak = movie.trak.first_mut().unwrap();
        trak.tkhd.duration = u64::MAX;
        if let Some(elst) = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut()) {
            elst.repeat_edits = true;
        }
        let sample = sample(1, 0, 100, 0, true);
        let timeline = Timeline::new(&movie, 1).unwrap();

        assert_eq!(
            timeline.present(sample.properties()).nth(1000),
            Some(Presentation::new(100_000, 100, 0))
        );
        assert_eq!(present(&movie, 100, 100), []);
    }

    #[test]
    fn a_fragmented_movie_presents_the_media_past_its_edit_list_after_it() {
        let mut movie = movie(&[(300, -1, 1), (600, 100, 1)]);
        movie.mvex = Some(MovieExtendsBox::default());

        // The edit presents media time 100 to 1100 from 500, the media after it follows from 1500.
        assert_eq!(
            [present(&movie, 1000, 100), present(&movie, 5000, 100)],
            [
                Vec::from([Presentation::new(1400, 100, 1000)]),
                Vec::from([Presentation::new(5400, 100, 5000)]),
            ]
        );
    }

    #[test]
    fn a_last_edit_of_duration_0_in_a_fragmented_movie_lasts_to_the_end_of_the_media() {
        let mut movie = movie(&[(0, 8, 1)]);
        movie.mvex = Some(MovieExtendsBox::default());

        assert_eq!(
            [present(&movie, 0, 10), present(&movie, 1_000_000, 10)],
            [
                Vec::from([Presentation::new(0, 2, 8)]),
                Vec::from([Presentation::new(999_992, 10, 1_000_000)]),
            ]
        );
    }

    #[test]
    fn an_edit_at_another_rate_than_0_presents_its_media_at_rate_1() {
        let movie = movie(&[(600, 0, 2)]);

        assert_eq!(
            present(&movie, 900, 100),
            [Presentation::new(900, 100, 900)]
        );
    }

    #[test]
    fn a_sample_of_duration_0_is_presented_for_no_time_by_an_edit_that_holds_its_composition_time()
    {
        let movie = movie(&[(600, 0, 1)]);

        assert_eq!(
            [present(&movie, 500, 0), present(&movie, 1000, 0)],
            [Vec::from([Presentation::new(500, 0, 500)]), Vec::new()]
        );
    }

    #[test]
    fn an_edit_list_that_does_not_map_a_timeline_is_refused() {
        let mut no_timescale = movie(&[(600, 0, 1)]);
        no_timescale.mvhd.timescale = 0;

        assert_eq!(
            [
                Timeline::new(&movie(&[]), 2).map(|_| ()),
                Timeline::new(&movie(&[(600, -2, 1)]), 1).map(|_| ()),
                Timeline::new(&no_timescale, 1).map(|_| ()),
            ],
            [
                Err(TimelineError::UnknownTrackId { track_id: 2 }),
                Err(TimelineError::InvalidMediaTime {
                    track_id: 1,
                    media_time: -2
                }),
                Err(TimelineError::MissingMovieTimescale { track_id: 1 }),
            ]
        );
    }
}
