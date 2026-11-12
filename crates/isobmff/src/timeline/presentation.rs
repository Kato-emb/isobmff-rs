//! [`presentation`], the samples of a track as its edit list presents them

use alloc::boxed::Box;
use alloc::vec::Vec;

use isobmff_boxes::{EditListEntry, MovieBox};

use super::Error;
use crate::SampleProperties;

/// A sample, or the part of it, that the edit list of its track presents
///
/// The presentation timeline, ISO/IEC 14496-12 §8.6.6, counted in the
/// timescale of the media of the track, §8.4.2. Where an edit starts or
/// ends inside a sample, the sample is cut, §8.6.6.1 NOTE 1: its
/// `composition_time` is past the start of the sample, or its `duration`
/// ends before the sample does.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresentedSample {
    /// Position of the sample among those given, from 0
    pub index: usize,
    /// Where its presented part starts on the presentation timeline
    pub presentation_time: u64,
    /// How long it is presented
    pub duration: u64,
    /// The composition time its presented part starts at, §8.6.1.3, or the
    /// one held where `dwell`
    pub composition_time: u64,
    /// Whether the edit holds the media at `composition_time` for
    /// `duration`, a `dwell` of `media_rate` 0, rather than plays it
    pub dwell: bool,
}

/// The samples of a track as its edit list presents them, in the order of the presentation timeline
///
/// Returned by [`presentation`].
#[derive(Debug, Clone)]
pub struct Presentation<'a> {
    edits: Edits<'a>,
    samples: Vec<SampleProperties>,
    /// Position of each sample among `samples`, in the order of their
    /// composition times
    order: Vec<usize>,
    /// The span being presented, and the position in `order` of the next
    /// sample to look at within it
    span: Option<(Span, usize)>,
}

/// The edits of a track, and where among them the presentation is
#[derive(Debug, Clone)]
enum Edits<'a> {
    /// No edit list: the composition timeline is presented as it is, as
    /// the span `presentation` opens with
    None,
    List(Box<List<'a>>),
}

#[derive(Debug, Clone)]
struct List<'a> {
    entries: &'a [EditListEntry],
    movie_timescale: u128,
    media_timescale: u128,
    /// Sum of the `edit_duration`s before each edit, and of all of them
    /// last, in the timescale of the movie
    starts: Vec<u128>,
    repeat: Repeat,
    /// Whether the movie is fragmented, §8.6.6.1
    fragmented: bool,
    /// Where the composition of the samples ends
    media_end: i128,
    repetition: u128,
    edit: usize,
    /// Whether a sample was presented in this repetition of the list
    presented: bool,
    /// Where the edits opened end, on the presentation timeline and in the
    /// media, for the implied edit that follows them
    explicit_end: Option<(i128, i128)>,
    implied: bool,
}

/// How often an edit list is presented, §8.6.6.1
#[derive(Debug, Clone, Copy)]
enum Repeat {
    Once,
    /// Until this time on the presentation timeline
    Until(i128),
    Forever,
}

/// A part of the presentation timeline, and what an edit presents over it
#[derive(Debug, Clone, Copy)]
struct Span {
    start: i128,
    /// `None` for a span that never ends
    end: Option<i128>,
    media_time: i128,
    dwell: bool,
}

/// Returns the samples of track `track_id` of `movie` as its edit list presents them, ISO/IEC 14496-12 §8.6.6
///
/// `samples` are those of the track, such as the
/// [`SampleLocation`](crate::SampleLocation)s a
/// [`Demuxer`](crate::Demuxer) reports for it, in any order: each is
/// presented over its composition time, its decoding time plus its
/// `sample_composition_time_offset`, §8.6.1.3, for its `sample_duration`.
/// The function holds a copy of them, and their order by composition time.
///
/// * Each edit maps its `edit_duration`, in the timescale of the movie
///   (§8.2.2), onto the media from its `media_time`, §8.6.6.3. The bounds of
///   the edits are the sums of the `edit_duration`s before them, written in
///   the timescale of the media, §8.4.2, to the nearest unit.
/// * An empty edit, of `media_time` −1, presents no sample. A `dwell`, of
///   `media_rate` 0, holds the media at its `media_time`. Every other
///   `media_rate` plays the media forward as 1.0 does.
/// * An edit cuts the samples it starts or ends inside, §8.6.6.1 NOTE 1.
///   Media an edit maps past the last sample presents nothing.
/// * With `RepeatEdits`, the edit list is repeated until the `duration` of
///   the `tkhd`, and without end where that is indefinite (all 1s), §8.6.6.1,
///   §8.3.2. A list that presents no sample is not repeated.
/// * In a fragmented movie, a last edit that is not empty and lasts 0 runs to
///   the end of the media, and the media after the last edit that is not
///   empty follows the edits, as the implied edit of §8.6.6.1 presents it.
/// * A track without an edit list, or with an empty one, presents the
///   composition timeline as it is; a sample whose composition time is
///   before 0 is presented from 0.
///
/// The samples are returned in the order of the presentation timeline; a
/// sample an edit list repeats is returned each time. The iterator ends
/// where a presentation time would pass what a `u64` counts.
///
/// # Errors
/// * [`Error::UnknownTrack`]: the movie has no track `track_id`.
/// * [`Error::OtherTrack`]: a sample is of another track.
/// * [`Error::ZeroTimescale`]: the track has an edit list, and the movie or
///   its media has a `timescale` of 0, which §8.2.2 and §8.4.2 do not allow:
///   its edits cannot be written in the timescale of the media.
///
/// ```
/// use isobmff::SampleProperties;
/// use isobmff::boxes::{EditBox, EditListBox, EditListEntry, MovieBox, SampleFlags, TrackBox};
/// use isobmff::timeline::presentation;
///
/// // A track of media of 1,000 units a second, in a movie of 1,000 units a
/// // second, whose edit list starts it 500 later and plays from 100 to 2,100
/// let edit = |edit_duration, media_time| {
///     let mut entry = EditListEntry::default();
///     (entry.edit_duration, entry.media_time, entry.media_rate_integer) = (edit_duration, media_time, 1);
///     entry
/// };
/// let mut elst = EditListBox::default();
/// elst.entries = vec![edit(500, -1), edit(2_000, 100)];
/// let mut edts = EditBox::default();
/// edts.elst = Some(elst);
/// let mut trak = TrackBox::default();
/// trak.tkhd.track_id = 1;
/// trak.mdia.mdhd.timescale = 1_000;
/// trak.edts = Some(edts);
/// let mut moov = MovieBox::default();
/// moov.mvhd.timescale = 1_000;
/// moov.trak = vec![trak];
///
/// // Three samples of 1,000 each
/// let samples = (0..3).map(|index| SampleProperties {
///     track_id: 1,
///     decoding_time: index * 1_000,
///     sample_duration: 1_000,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// });
///
/// // The first is cut at 100, the third ends at 2,100
/// let presented: Vec<_> = presentation(&moov, 1, samples)?
///     .map(|sample| (sample.index, sample.presentation_time, sample.duration, sample.composition_time))
///     .collect();
/// assert_eq!(presented, [(0, 500, 900, 100), (1, 1_400, 1_000, 1_000), (2, 2_400, 100, 2_000)]);
/// # Ok::<(), isobmff::timeline::Error>(())
/// ```
pub fn presentation(
    movie: &MovieBox,
    track_id: u32,
    samples: impl IntoIterator<Item = SampleProperties>,
) -> Result<Presentation<'_>, Error> {
    let track = movie
        .trak
        .iter()
        .find(|track| track.tkhd.track_id == track_id)
        .ok_or(Error::UnknownTrack { track_id })?;
    let samples = samples
        .into_iter()
        .map(|sample| {
            if sample.track_id == track_id {
                Ok(sample)
            } else {
                Err(Error::OtherTrack {
                    track_id,
                    found: sample.track_id,
                })
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut order: Vec<usize> = (0..samples.len()).collect();
    order.sort_by_key(|&index| samples.get(index).map_or(0, composition_start));
    let entries = track
        .edts
        .as_ref()
        .and_then(|edts| edts.elst.as_ref())
        .filter(|elst| !elst.entries.is_empty());
    let edits = match entries {
        None => Edits::None,
        Some(elst) => {
            let (movie_timescale, media_timescale) =
                (movie.mvhd.timescale, track.mdia.mdhd.timescale);
            if movie_timescale == 0 || media_timescale == 0 {
                return Err(Error::ZeroTimescale {
                    movie_timescale,
                    media_timescale,
                });
            }
            let mut starts = Vec::with_capacity(elst.entries.len().saturating_add(1));
            let mut sum = 0_u128;
            starts.push(sum);
            for entry in &elst.entries {
                sum = sum.saturating_add(u128::from(entry.edit_duration));
                starts.push(sum);
            }
            let (movie_timescale, media_timescale) =
                (u128::from(movie_timescale), u128::from(media_timescale));
            let duration = track.tkhd.duration;
            let indefinite = duration == u64::MAX
                || (track.tkhd.version == 0 && duration == u64::from(u32::MAX));
            let repeat = match (elst.repeat_edits && sum > 0, indefinite) {
                (false, _) => Repeat::Once,
                (true, true) => Repeat::Forever,
                (true, false) => Repeat::Until(in_media(
                    u128::from(duration),
                    movie_timescale,
                    media_timescale,
                )),
            };
            Edits::List(Box::new(List {
                entries: &elst.entries,
                movie_timescale,
                media_timescale,
                starts,
                repeat,
                fragmented: movie.mvex.is_some(),
                media_end: samples
                    .iter()
                    .map(composition_end_of)
                    .max()
                    .unwrap_or(0)
                    .max(0),
                repetition: 0,
                edit: 0,
                presented: false,
                explicit_end: None,
                implied: false,
            }))
        }
    };
    let span = matches!(edits, Edits::None).then_some((
        Span {
            start: 0,
            end: None,
            media_time: 0,
            dwell: false,
        },
        0,
    ));
    Ok(Presentation {
        edits,
        samples,
        order,
        span,
    })
}

impl Iterator for Presentation<'_> {
    type Item = PresentedSample;

    fn next(&mut self) -> Option<PresentedSample> {
        loop {
            let (span, position) = match self.span {
                Some(open) => open,
                None => {
                    let span = self.edits.next_span()?;
                    let position = self.first_at(span.media_time, span.dwell);
                    self.span = Some((span, position));
                    (span, position)
                }
            };
            match self.presented_at(span, position) {
                Some((presented, next)) => {
                    self.span = Some((span, next));
                    if let Some(presented) = presented {
                        if let Edits::List(list) = &mut self.edits {
                            list.presented = true;
                        }
                        return Some(presented);
                    }
                }
                None => self.span = None,
            }
        }
    }
}

impl Presentation<'_> {
    /// The position in `order` of the first sample a span from `media_time` may present
    fn first_at(&self, media_time: i128, dwell: bool) -> usize {
        let starts_before = self.order.partition_point(|&index| {
            self.samples
                .get(index)
                .is_some_and(|sample| composition_start(sample) <= media_time)
        });
        let before = starts_before.saturating_sub(1);
        // Why not start at the first sample that starts at `media_time` or
        // later: the one that starts before it may still be composed at
        // `media_time`, and is then the first presented.
        let covers = self
            .order
            .get(before)
            .and_then(|&index| self.samples.get(index))
            .is_some_and(|sample| composition_end_of(sample) > media_time);
        if covers || dwell {
            before
        } else {
            starts_before
        }
    }

    /// The sample at `position` in `order` as `span` presents it, if any, and the position to look at next; `None` once the span presents no more
    fn presented_at(
        &self,
        span: Span,
        position: usize,
    ) -> Option<(Option<PresentedSample>, usize)> {
        let &index = self.order.get(position)?;
        let sample = self.samples.get(index)?;
        let (start, end) = (composition_start(sample), composition_end_of(sample));
        let presented = |presentation_time: i128, duration: i128, composition_time: i128| {
            Some(PresentedSample {
                index,
                presentation_time: u64::try_from(presentation_time).ok()?,
                duration: u64::try_from(duration).unwrap_or(u64::MAX),
                composition_time: u64::try_from(composition_time).ok()?,
                dwell: span.dwell,
            })
        };
        if span.dwell {
            let held = start <= span.media_time && span.media_time < end;
            let duration = span.end?.saturating_sub(span.start);
            if !held || duration <= 0 {
                return None;
            }
            // Why not look past the sample: a dwell holds one point of the media.
            return Some((
                Some(presented(span.start, duration, span.media_time)?),
                self.order.len(),
            ));
        }
        let media_end = span.end.map(|end| {
            span.media_time
                .saturating_add(end.saturating_sub(span.start))
        });
        if media_end.is_some_and(|media_end| start >= media_end) {
            return None;
        }
        let shown_start = start.max(span.media_time);
        let shown_end = media_end.map_or(end, |media_end| end.min(media_end));
        let next = position.saturating_add(1);
        if shown_end <= shown_start {
            return Some((None, next));
        }
        let presentation_time = span
            .start
            .saturating_add(shown_start.saturating_sub(span.media_time));
        let item = presented(
            presentation_time,
            shown_end.saturating_sub(shown_start),
            shown_start,
        )?;
        Some((Some(item), next))
    }
}

impl Edits<'_> {
    /// The next span of the presentation timeline that presents media, or `None` once there is none
    fn next_span(&mut self) -> Option<Span> {
        match self {
            Self::None => None,
            Self::List(list) => list.next_span(),
        }
    }
}

impl List<'_> {
    fn next_span(&mut self) -> Option<Span> {
        loop {
            let Some(entry) = self.entries.get(self.edit) else {
                if matches!(self.repeat, Repeat::Once) {
                    return self.implied_span();
                }
                if !self.presented {
                    return None;
                }
                self.repetition = self.repetition.saturating_add(1);
                self.edit = 0;
                self.presented = false;
                continue;
            };
            let start = self.bound(self.edit)?;
            let mut end = self.bound(self.edit.saturating_add(1))?;
            let last = self.edit.saturating_add(1) == self.entries.len();
            self.edit = self.edit.saturating_add(1);
            let media_time = i128::from(entry.media_time);
            if let Repeat::Until(track_end) = self.repeat {
                if start >= track_end {
                    return None;
                }
                end = end.min(track_end);
            }
            if media_time < 0 {
                continue;
            }
            let runs_to_media_end = last
                && entry.edit_duration == 0
                && self.fragmented
                && matches!(self.repeat, Repeat::Once);
            if runs_to_media_end {
                end = start.saturating_add(self.media_end.saturating_sub(media_time).max(0));
            }
            let dwell = entry.media_rate_integer == 0 && entry.media_rate_fraction == 0;
            let media_reached = if dwell {
                media_time
            } else {
                media_time.saturating_add(end.saturating_sub(start))
            };
            self.explicit_end = Some((end, media_reached));
            if end <= start {
                continue;
            }
            return Some(Span {
                start,
                end: Some(end),
                media_time,
                dwell,
            });
        }
    }

    /// Where edit `edit` of the current repetition starts on the presentation timeline, `None` past what a `u64` counts
    fn bound(&self, edit: usize) -> Option<i128> {
        let total = self.starts.last().copied().unwrap_or(0);
        let before = self.starts.get(edit).copied().unwrap_or(total);
        let movie_units = self.repetition.saturating_mul(total).saturating_add(before);
        let media_units = in_media(movie_units, self.movie_timescale, self.media_timescale);
        (media_units <= i128::from(u64::MAX)).then_some(media_units)
    }

    /// The implied edit of a fragmented movie, §8.6.6.1: the media past the last edit that is not empty, after the edits
    fn implied_span(&mut self) -> Option<Span> {
        if self.implied || !self.fragmented {
            return None;
        }
        self.implied = true;
        let total = self.bound(self.entries.len())?;
        let (start, media_time) = match self.explicit_end {
            Some((end, media_reached)) => (end.max(total), media_reached),
            None => (total, 0),
        };
        let length = self.media_end.saturating_sub(media_time);
        (length > 0).then_some(Span {
            start,
            end: Some(start.saturating_add(length)),
            media_time,
            dwell: false,
        })
    }
}

/// `movie_units` of the timescale of the movie, in that of the media, to the nearest unit
fn in_media(movie_units: u128, movie_timescale: u128, media_timescale: u128) -> i128 {
    let scaled = movie_units
        .saturating_mul(media_timescale)
        .saturating_add(movie_timescale / 2)
        .checked_div(movie_timescale)
        .unwrap_or(0);
    i128::try_from(scaled).unwrap_or(i128::MAX)
}

/// Composition time of `sample`, §8.6.1.3
fn composition_start(sample: &SampleProperties) -> i128 {
    i128::from(sample.decoding_time)
        .saturating_add(i128::from(sample.sample_composition_time_offset))
}

/// Where the composition of `sample` ends
fn composition_end_of(sample: &SampleProperties) -> i128 {
    composition_start(sample).saturating_add(i128::from(sample.sample_duration))
}
