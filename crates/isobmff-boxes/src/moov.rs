//! [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, Mp4EpochSeconds, OtherBoxes, boxes,
};

use crate::data_types::{HeaderDuration, SampleFlags};
use crate::mvex::MovieExtendsBox;
use crate::mvhd::MovieHeaderBox;
use crate::trak::TrackBox;
use crate::trex::TrackExtendsBox;

/// Box that holds every declaration a presentation is made of
///
/// [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1. It carries the movie header,
/// one `trak` per track, and — when the presentation continues in fragments —
/// the `mvex` that says so.
///
/// §8.8.3 has one `trex` for each track of a fragmented movie, so this box
/// refuses a track left without one, in [`new`](Self::new) as in
/// [`decode_payload`](BoxDecode::decode_payload). A `track_id` identifies a
/// track over the whole presentation (§8.3.2.3), and that is held to in
/// [`new`](Self::new) alone: a decoded movie whose tracks collide on a
/// `track_id` reads as it came.
///
/// On encode the children are written in the order the spec lists them —
/// `mvhd`, then the tracks, then `mvex` — and then the children no field
/// claims, so a round-trip settles the order rather than preserving it.
#[doc(alias = "moov")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MovieBox {
    mvhd: MovieHeaderBox,
    trak: Vec<TrackBox>,
    mvex: Option<MovieExtendsBox>,
    other_boxes: OtherBoxes,
}

impl MovieBox {
    /// Creates the box from the movie header and the tracks it declares
    ///
    /// Returns `None` for an empty `trak`, which the spec does not allow: a
    /// presentation is made of at least one track.
    ///
    /// Returns `None` when two tracks declare the same `track_id`, and — where
    /// the movie is fragmented — when a track is left without its `trex`.
    #[must_use]
    pub fn new(
        mvhd: MovieHeaderBox,
        trak: Vec<TrackBox>,
        mvex: Option<MovieExtendsBox>,
    ) -> Option<Self> {
        if trak.is_empty() {
            return None;
        }

        let declared_track_ids: BTreeSet<u32> =
            trak.iter().map(|track| track.tkhd().track_id()).collect();
        if declared_track_ids.len() != trak.len() {
            return None;
        }

        if a_track_lacks_its_trex(&trak, mvex.as_ref()) {
            return None;
        }

        Some(Self {
            mvhd,
            trak,
            mvex,
            other_boxes: OtherBoxes::new(),
        })
    }

    /// Creates the box of a movie continued in fragments from its timescale and tracks
    ///
    /// [`new`](Self::new) states every field; `new_fragmented` states the ones
    /// a movie continued in fragments (ISO/IEC 14496-12 Annex A.8) varies and
    /// fills the rest:
    ///
    /// * `mvhd` (§8.2.2): a `creation_time`, `modification_time` and
    ///   `duration` of 0, a `next_track_id` one greater than the largest
    ///   `track_id` of `tracks` — left at all 1s when that `track_id` is itself
    ///   all 1s — and the template values of [`MovieHeaderBox::new`].
    /// * `mvex` (§8.8.1): one `trex` (§8.8.3) for each track, whose samples
    ///   default to the first sample description, a duration and a size of 0,
    ///   and [`SampleFlags::ZERO`].
    ///
    /// Returns `None` where [`new`](Self::new) would: for an empty `tracks`,
    /// and when two tracks declare the same `track_id`.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_boxes::{MovieBox, TrackBox};
    /// use isobmff_core::{AnyBox, BoxType};
    ///
    /// // One video track, its samples to come in fragments
    /// let entry = AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0, 0, 0, 0, 0, 0, 0, 1]);
    /// let track = TrackBox::new_video(1, 90_000, 1920, 1080, entry);
    /// let movie = MovieBox::new_fragmented(1_000, vec![track]).unwrap();
    /// assert_eq!(movie.mvhd().next_track_id(), 2);
    /// assert_eq!(movie.mvex().unwrap().trex()[0].track_id(), 1);
    ///
    /// // A movie is made of at least one track
    /// assert_eq!(MovieBox::new_fragmented(1_000, Vec::new()), None);
    /// ```
    #[must_use]
    pub fn new_fragmented(timescale: u32, tracks: Vec<TrackBox>) -> Option<Self> {
        let track_ids = tracks.iter().map(|track| track.tkhd().track_id());
        let next_track_id = track_ids.clone().max()?.saturating_add(1);
        let mvex = MovieExtendsBox::new(
            track_ids
                .map(|track_id| TrackExtendsBox::new(track_id, 1, 0, 0, SampleFlags::ZERO))
                .collect(),
        )?;
        let epoch = Mp4EpochSeconds::from_seconds(0);

        Self::new(
            MovieHeaderBox::new(epoch, epoch, timescale, HeaderDuration::ZERO, next_track_id),
            tracks,
            Some(mvex),
        )
    }

    /// Returns the declarations the presentation applies as a whole
    #[must_use]
    pub const fn mvhd(&self) -> &MovieHeaderBox {
        &self.mvhd
    }

    /// Returns the declarations the presentation applies as a whole, to be changed in place
    #[must_use]
    pub const fn mvhd_mut(&mut self) -> &mut MovieHeaderBox {
        &mut self.mvhd
    }

    /// Returns the tracks the presentation is made of
    #[must_use]
    pub fn trak(&self) -> &[TrackBox] {
        &self.trak
    }

    /// Returns the track `track_id` names, to be changed in place, or `None` for a track the movie does not declare
    ///
    /// What [`new`](Self::new) settled — distinct `track_id`s, and a `trex` for
    /// each track where the movie is fragmented — holds of the tracks as they
    /// were built; a change made through here that touches either is the
    /// caller's to keep to them. A decoded movie whose tracks collide on
    /// `track_id` yields the first of them, in the order they came.
    #[must_use]
    pub fn trak_mut(&mut self, track_id: u32) -> Option<&mut TrackBox> {
        self.trak
            .iter_mut()
            .find(|track| track.tkhd().track_id() == track_id)
    }

    /// States the duration of the movie, its tracks and their media from the tables the movie holds
    ///
    /// Every track, one with no samples as well, is stated afresh:
    ///
    /// * `mdhd` (ISO/IEC 14496-12 §8.4.2.3): as
    ///   [`MediaBox::state_duration`](crate::MediaBox::state_duration) states
    ///   it, the length of the media.
    /// * `tkhd` (§8.3.2.3): the sum of the `segment_duration` of the track's
    ///   edits, or, for a track with no edit list, the `mdhd` duration converted
    ///   to the movie's time scale and rounded up to the next whole unit.
    /// * `mvhd` (§8.2.2.3): the duration of the longest track.
    ///
    /// A duration cannot be determined — it is
    /// [`HeaderDuration::INDETERMINATE`](crate::HeaderDuration::INDETERMINATE)
    /// — where its sum or conversion reaches [`u64::MAX`], the media's time
    /// scale is 0, or, for a `tkhd` with no edit list, the `mdhd` duration
    /// cannot be determined; the movie's cannot be determined once any track's
    /// cannot.
    pub fn state_durations(&mut self) {
        let movie_timescale = self.mvhd.timescale();
        let mut longest = Some(0_u64);

        for track in &mut self.trak {
            let duration = track.state_duration(movie_timescale).get();
            longest = longest
                .zip(duration)
                .map(|(longest, duration)| longest.max(duration));
        }

        self.mvhd = self
            .mvhd
            .clone()
            .with_duration(HeaderDuration::from_derived(longest));
    }

    /// Returns the declaration that the movie continues in fragments, if it does
    #[must_use]
    pub const fn mvex(&self) -> Option<&MovieExtendsBox> {
        self.mvex.as_ref()
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

/// Returns whether a track of the movie is left without its own `trex`
fn a_track_lacks_its_trex(trak: &[TrackBox], mvex: Option<&MovieExtendsBox>) -> bool {
    let Some(mvex) = mvex else {
        return false;
    };

    // Why not scanning the `trex` per track: both counts follow from the payload
    // length, so the scan would cost the product of two figures an input settles.
    let extended_track_ids: BTreeSet<u32> =
        mvex.trex().iter().map(TrackExtendsBox::track_id).collect();

    trak.iter()
        .any(|track| !extended_track_ids.contains(&track.tkhd().track_id()))
}

impl BoxDefinition for MovieBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"moov");
}

impl BoxDecode for MovieBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `mvhd`,
    ///   no `trak` at all, or a track of a fragmented movie without its `trex`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `mvhd` or
    ///   `mvex`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: one of the
    ///   children does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut mvhd_boxes = ChildBoxes::new();
        let mut trak_boxes = ChildBoxes::new();
        let mut mvex_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == MovieHeaderBox::BOX_TYPE {
                mvhd_boxes.push(child);
            } else if box_type == TrackBox::BOX_TYPE {
                trak_boxes.push(child);
            } else if box_type == MovieExtendsBox::BOX_TYPE {
                mvex_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        let mvhd = mvhd_boxes.exactly_one()?;
        let trak = trak_boxes.one_or_more()?;
        let mvex = mvex_boxes.zero_or_one()?;

        if a_track_lacks_its_trex(&trak, mvex.as_ref()) {
            return Err(Error::missing_mandatory_box(TrackExtendsBox::BOX_TYPE));
        }

        Ok(Self {
            mvhd,
            trak,
            mvex,
            other_boxes,
        })
    }
}

impl BoxEncode for MovieBox {
    fn payload_len(&self) -> u64 {
        let tracks = self.trak.iter().fold(0_u64, |total, track| {
            total.saturating_add(track.encoded_len())
        });
        let extends = self.mvex.as_ref().map_or(0, |mvex| mvex.encoded_len());
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.mvhd
            .encoded_len()
            .saturating_add(tracks)
            .saturating_add(extends)
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.mvhd.encode(writer.take_remainder())?;
        for track in &self.trak {
            rest = track.encode(rest)?;
        }
        if let Some(mvex) = &self.mvex {
            rest = mvex.encode(rest)?;
        }
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{
        AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, LanguageCode, Mp4EpochSeconds,
    };

    use super::{
        HeaderDuration, MovieBox, MovieExtendsBox, MovieHeaderBox, TrackBox, TrackExtendsBox,
    };
    use crate::chunk_offset::ChunkOffsets;
    use crate::data_types::SampleFlags;
    use crate::edts::EditBox;
    use crate::edts::tests::edit;
    use crate::mdhd::MediaHeaderBox;
    use crate::mvex::tests::movie_extends;
    use crate::mvhd::tests::movie_header;
    use crate::sample_size::{SampleSizeBox, SampleSizes};
    use crate::stbl::SampleTableBox;
    use crate::stsc::SampleToChunkBox;
    use crate::stsd::SampleDescriptionBox;
    use crate::stts::TimeToSampleBox;
    use crate::trak::tests::{track, video_track};

    /// Movie with one track, as a progressive file declares it
    fn movie() -> MovieBox {
        MovieBox::new(movie_header(5_000), vec![track()], None).unwrap()
    }

    /// Video track `track_id` on the media time scale `timescale`, its samples lasting `deltas`
    ///
    /// Every duration its headers state is 0, as a track built before its
    /// samples states it.
    fn timed_track(
        track_id: u32,
        timescale: u32,
        deltas: impl IntoIterator<Item = u32>,
    ) -> TrackBox {
        let mut track = video_track(track_id);
        let epoch = Mp4EpochSeconds::from_seconds(0);
        *track.mdia_mut().mdhd_mut() = MediaHeaderBox::new(
            epoch,
            epoch,
            timescale,
            HeaderDuration::ZERO,
            LanguageCode::UND,
        );
        *track.mdia_mut().minf_mut().stbl_mut() = SampleTableBox::new(
            SampleDescriptionBox::new(Vec::new()),
            TimeToSampleBox::from_deltas(deltas),
            SampleToChunkBox::new(Vec::new()),
            SampleSizes::Stsz(SampleSizeBox::from_sizes([])),
            ChunkOffsets::from_offsets([]),
        );

        track
    }

    /// Duration of `value`, which is below all 1s
    fn duration(value: u64) -> HeaderDuration {
        HeaderDuration::new(value).unwrap()
    }

    /// The track with the durations of its media and of itself as given
    fn with_durations(
        mut track: TrackBox,
        media_duration: HeaderDuration,
        track_duration: HeaderDuration,
    ) -> TrackBox {
        *track.mdia_mut().mdhd_mut() = track.mdia().mdhd().clone().with_duration(media_duration);
        *track.tkhd_mut() = track.tkhd().clone().with_duration(track_duration);

        track
    }

    /// Extends box setting the defaults of a track the movie does not declare
    fn extends_of_another_track() -> MovieExtendsBox {
        MovieExtendsBox::new(vec![TrackExtendsBox::new(7, 1, 0, 0, SampleFlags::ZERO)]).unwrap()
    }

    /// Writes one child whole and returns the bytes it occupies
    fn encoded_child(child: &(impl BoxDefinition + BoxEncode)) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(child.encoded_len()).unwrap()];
        child.encode(&mut buffer).unwrap();

        buffer
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(movie: &MovieBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(movie.payload_len()).unwrap()];
        movie.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_movie_of_no_tracks_cannot_be_built() {
        assert_eq!(MovieBox::new(movie_header(0), Vec::new(), None), None);
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&movie());

        assert_eq!(MovieBox::decode_payload(&payload).unwrap(), movie());
    }

    #[test]
    fn a_fragmented_movie_reads_back_with_its_extends_box() {
        let fragmented =
            MovieBox::new(movie_header(0), vec![track()], Some(movie_extends())).unwrap();

        let payload = encoded_payload(&fragmented);

        assert_eq!(MovieBox::decode_payload(&payload).unwrap(), fragmented);
    }

    #[test]
    fn a_fragmented_movie_extends_each_track_and_numbers_the_next_past_the_largest() {
        let epoch = Mp4EpochSeconds::from_seconds(0);

        assert_eq!(
            MovieBox::new_fragmented(1_000, vec![video_track(9), video_track(3)]),
            MovieBox::new(
                MovieHeaderBox::new(epoch, epoch, 1_000, HeaderDuration::ZERO, 10),
                vec![video_track(9), video_track(3)],
                MovieExtendsBox::new(vec![
                    TrackExtendsBox::new(9, 1, 0, 0, SampleFlags::ZERO),
                    TrackExtendsBox::new(3, 1, 0, 0, SampleFlags::ZERO),
                ]),
            )
        );
    }

    #[test]
    fn a_fragmented_movie_reads_back_as_the_value_that_wrote_it() {
        let movie = MovieBox::new_fragmented(1_000, vec![video_track(9), video_track(3)]).unwrap();

        assert_eq!(
            MovieBox::decode_payload(&encoded_payload(&movie)).unwrap(),
            movie
        );
    }

    #[test]
    fn a_fragmented_movie_of_no_tracks_or_of_one_track_id_twice_cannot_be_built() {
        assert_eq!(MovieBox::new_fragmented(1_000, Vec::new()), None);
        assert_eq!(
            MovieBox::new_fragmented(1_000, vec![video_track(1), video_track(1)]),
            None
        );
    }

    #[test]
    fn a_fragmented_movie_holding_the_largest_track_id_leaves_the_next_at_all_ones() {
        let movie = MovieBox::new_fragmented(1_000, vec![video_track(u32::MAX)]).unwrap();

        assert_eq!(movie.mvhd().next_track_id(), u32::MAX);
    }

    #[test]
    fn a_movie_holding_no_track_is_rejected() {
        let whole = encoded_payload(&movie());
        let header_len = usize::try_from(movie().mvhd().encoded_len()).unwrap();

        assert_eq!(
            MovieBox::decode_payload(whole.get(..header_len).unwrap()),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"trak")))
        );
    }

    #[test]
    fn a_movie_declaring_one_track_id_twice_cannot_be_built() {
        assert_eq!(
            MovieBox::new(movie_header(0), vec![track(), track()], None),
            None
        );
    }

    #[test]
    fn a_fragmented_movie_leaving_a_track_without_its_extends_box_cannot_be_built() {
        assert_eq!(
            MovieBox::new(
                movie_header(0),
                vec![track()],
                Some(extends_of_another_track())
            ),
            None
        );
    }

    #[test]
    fn a_fragmented_movie_leaving_a_track_without_its_extends_box_is_rejected() {
        let payload = [
            encoded_payload(&movie()),
            encoded_child(&extends_of_another_track()),
        ]
        .concat();

        assert_eq!(
            MovieBox::decode_payload(&payload),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"trex")))
        );
    }

    #[test]
    fn a_decoded_movie_declaring_one_track_id_twice_keeps_both_tracks() {
        let payload = [encoded_payload(&movie()), encoded_child(&track())].concat();

        let decoded = MovieBox::decode_payload(&payload).unwrap();

        assert_eq!(decoded.trak(), [track(), track()]);
    }

    #[test]
    fn the_sample_tables_of_a_track_are_replaced_in_place_and_the_rest_of_the_movie_kept() {
        let unclaimed = vec![
            0, 0, 0, 0x0c, b'u', b'd', b't', b'a', 0x11, 0x11, 0x11, 0x11,
        ];
        let payload = [encoded_payload(&movie()), unclaimed].concat();
        let mut decoded = MovieBox::decode_payload(&payload).unwrap();
        let laid_out = SampleTableBox::new(
            SampleDescriptionBox::new(Vec::new()),
            TimeToSampleBox::from_deltas([3_000]),
            SampleToChunkBox::from_chunks([(1, 1)]).unwrap(),
            SampleSizes::Stsz(SampleSizeBox::from_sizes([4])),
            ChunkOffsets::from_offsets([1_000]),
        );

        *decoded
            .trak_mut(1)
            .unwrap()
            .mdia_mut()
            .minf_mut()
            .stbl_mut() = laid_out.clone();

        assert_eq!(
            decoded
                .trak()
                .first()
                .map(|track| track.mdia().minf().stbl()),
            Some(&laid_out)
        );
        assert_eq!(
            decoded.other_boxes(),
            [AnyBox::from_raw_bytes(
                BoxType::compact(*b"udta"),
                vec![0x11; 4]
            )]
        );
    }

    #[test]
    fn a_track_the_movie_does_not_declare_yields_nothing() {
        assert_eq!(movie().trak_mut(7), None);
    }

    #[test]
    fn a_duration_set_through_a_track_is_written_with_the_movie() {
        let mut movie = movie();

        let edited = movie.trak_mut(1).unwrap();
        *edited.tkhd_mut() = edited.tkhd().clone().with_duration(duration(7_000));

        assert_eq!(
            MovieBox::decode_payload(&encoded_payload(&movie)),
            Ok(MovieBox::new(
                movie_header(5_000),
                vec![TrackBox::new(
                    track().tkhd().clone().with_duration(duration(7_000)),
                    track().mdia().clone(),
                )],
                None,
            )
            .unwrap())
        );
    }

    #[test]
    fn a_track_converted_to_the_movie_time_scale_is_rounded_up_and_the_movie_lasts_the_longest() {
        let mut movie = MovieBox::new(
            movie_header(0),
            vec![
                timed_track(1, 3, [1, 1]),
                timed_track(2, 90_000, [3_000, 3_000]),
            ],
            None,
        )
        .unwrap();

        movie.state_durations();

        assert_eq!(
            MovieBox::new(
                movie_header(667),
                vec![
                    with_durations(timed_track(1, 3, [1, 1]), duration(2), duration(667)),
                    with_durations(
                        timed_track(2, 90_000, [3_000, 3_000]),
                        duration(6_000),
                        duration(67)
                    ),
                ],
                None,
            ),
            Some(movie)
        );
    }

    #[test]
    fn a_track_with_an_edit_list_lasts_its_edits_and_one_without_is_converted() {
        let edited = timed_track(1, 3, [1, 1]).with_edts(edit());
        let edit_box_alone = timed_track(2, 3, [1, 1]).with_edts(EditBox::new());
        let mut movie = MovieBox::new(
            movie_header(0),
            vec![edited.clone(), edit_box_alone.clone()],
            None,
        )
        .unwrap();

        movie.state_durations();

        assert_eq!(
            MovieBox::new(
                movie_header(3_010),
                vec![
                    with_durations(edited, duration(2), duration(3_010)),
                    with_durations(edit_box_alone, duration(2), duration(667)),
                ],
                None,
            ),
            Some(movie)
        );
    }

    #[test]
    fn a_track_whose_duration_overflows_leaves_it_and_the_movie_not_determined() {
        let epoch = Mp4EpochSeconds::from_seconds(0);
        let widest_scale = MovieHeaderBox::new(epoch, epoch, u32::MAX, HeaderDuration::ZERO, 3);
        let overflowing = timed_track(1, 1, [1 << 31; 4]);
        let fitting = timed_track(2, u32::MAX, [5]);
        let mut movie = MovieBox::new(
            widest_scale.clone(),
            vec![overflowing.clone(), fitting.clone()],
            None,
        )
        .unwrap();

        movie.state_durations();

        assert_eq!(
            MovieBox::new(
                widest_scale.with_duration(HeaderDuration::INDETERMINATE),
                vec![
                    with_durations(
                        overflowing,
                        duration(1 << 33),
                        HeaderDuration::INDETERMINATE
                    ),
                    with_durations(fitting, duration(5), duration(5)),
                ],
                None,
            ),
            Some(movie)
        );
    }

    #[test]
    fn a_track_with_no_samples_lasts_0() {
        let mut movie = MovieBox::new(
            movie_header(5_000),
            vec![with_durations(video_track(1), duration(9), duration(9))],
            None,
        )
        .unwrap();

        movie.state_durations();

        assert_eq!(
            MovieBox::new(movie_header(0), vec![video_track(1)], None),
            Some(movie)
        );
    }

    #[test]
    fn a_second_movie_header_is_rejected() {
        let payload = [encoded_payload(&movie()), encoded_payload(&movie())].concat();

        assert_eq!(
            MovieBox::decode_payload(&payload),
            Err(Error::duplicate_box(BoxType::compact(*b"mvhd")))
        );
    }
}
