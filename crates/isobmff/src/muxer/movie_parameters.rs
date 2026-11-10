//! [`MovieParameters`], what the samples of a movie do not give a muxer

use alloc::vec::Vec;

use isobmff_boxes::{
    Matrix, MetaBox, MovieBox, MovieExtendsBox, TrackBox, TrackExtendsBox, UserDataBox,
};
use isobmff_core::{I8F8, I16F16, OtherBoxes};

use super::TrackParameters;
use super::movie::{data_reference, set_durations};
use crate::structure_data::{SampleTableWriter, TrackSampleDescription};

/// What the samples of a movie do not give a muxer: every field and box of its `moov` that is not derived from its tracks
///
/// The `movie` box, [`MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1, apart
/// from its tracks and what is derived from them: the duration and the
/// next `track_ID` of the `movie header` (§8.2.2), its version, and the
/// `mvex` (§8.8.1). A field reads as the field of the box it names, which
/// its doc cites.
///
/// [`new`](Self::new) fills in what the specification gives by default; a
/// movie that differs sets the fields after it:
///
/// ```
/// use isobmff::muxer::MovieParameters;
///
/// let movie = MovieParameters {
///     creation_time: 3_786_825_600,
///     ..MovieParameters::new(1000)
/// };
/// assert_eq!(movie.rate, isobmff::I16F16::ONE);
/// ```
///
/// A value a muxer derives cannot be set:
///
/// ```compile_fail
/// let mut movie = isobmff::muxer::MovieParameters::new(1000);
/// movie.duration = 60_000;
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "the fields are the fields of the boxes a caller sets; a field the specification adds is a change the caller has to see"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct MovieParameters {
    /// `timescale` of the `mvhd`, §8.2.2, the number of time units of the
    /// movie in a second
    pub timescale: u32,
    /// `creation_time` of the `mvhd`, in seconds since midnight, Jan. 1,
    /// 1904, in UTC
    pub creation_time: u64,
    /// `modification_time` of the `mvhd`, in seconds since midnight, Jan. 1,
    /// 1904, in UTC
    pub modification_time: u64,
    /// `rate` of the `mvhd`, the preferred rate to play the movie at
    pub rate: I16F16,
    /// `volume` of the `mvhd`, the preferred volume to play the movie at
    pub volume: I8F8,
    /// `matrix` of the `mvhd`, the transformation of the video
    pub matrix: Matrix,
    /// The `meta` of the movie, §8.11.1
    pub meta: Option<MetaBox>,
    /// The `udta` of the movie, §8.10.1
    pub udta: Vec<UserDataBox>,
    /// Children of the `moov` of a type the crate holds no field for, written
    /// after the tracks
    pub other_boxes: OtherBoxes,
}

impl MovieParameters {
    /// Creates the parameters of a movie of `timescale` time units a second, every other field as the specification gives it by default
    ///
    /// The movie is created and modified at time 0, to be played at a rate
    /// and a volume of 1.0, under the unity matrix; it has no other box.
    #[must_use]
    pub fn new(timescale: u32) -> Self {
        Self {
            timescale,
            creation_time: 0,
            modification_time: 0,
            rate: I16F16::ONE,
            volume: I8F8::ONE,
            matrix: Matrix::UNITY,
            meta: None,
            udta: Vec::new(),
            other_boxes: OtherBoxes::default(),
        }
    }

    /// Returns the `moov` of the movie whose tracks are `trak`, its durations those the tracks give and its next `track_ID` past theirs
    ///
    /// * The duration of the media of a track is
    ///   [`media_duration`](crate::structure_data::media_duration), ISO/IEC
    ///   14496-12 §8.4.2.
    /// * The duration of a track is the sum of the durations of its edits,
    ///   or, without an edit list, the duration of its media in the
    ///   timescale of the movie, rounded up, §8.3.2.
    /// * Where the last edit of a track is not empty and lasts 0, as the last
    ///   edit of a fragmented movie may to run to the end of its media
    ///   (§8.6.6.1), it is closed at the end of the composition of the
    ///   samples: it lasts from its `media_time` to the
    ///   [`composition_end`](crate::structure_data::composition_end) of the
    ///   track, in the timescale of the movie, rounded up. The `elst` takes
    ///   version 1 where that duration does not fit 32 bits.
    /// * The duration of the movie is that of its longest track, §8.2.2, and
    ///   its next `track_ID` one more than the largest of the tracks.
    /// * The `mvhd`, each `tkhd` and each `mdhd` are of version 0 where their
    ///   duration, `creation_time` and `modification_time` fit 32 bits, and
    ///   of version 1 otherwise.
    ///
    /// ```
    /// use isobmff::boxes::{SampleDescriptionBox, SampleTableBox};
    /// use isobmff::muxer::{MovieParameters, TrackParameters};
    /// use isobmff::FourCc;
    ///
    /// let track = TrackParameters::new(1, 48_000, FourCc::new(*b"soun"), SampleDescriptionBox::default());
    /// let movie = MovieParameters::new(1000);
    /// let moov = movie.to_movie_box(vec![track.to_track_box(SampleTableBox::default())]);
    /// assert_eq!(moov.mvhd.next_track_id, 2);
    /// ```
    #[must_use]
    pub fn to_movie_box(&self, trak: Vec<TrackBox>) -> MovieBox {
        let mut moov = MovieBox::default();
        let mvhd = &mut moov.mvhd;
        mvhd.timescale = self.timescale;
        mvhd.creation_time = self.creation_time;
        mvhd.modification_time = self.modification_time;
        mvhd.rate = self.rate;
        mvhd.volume = self.volume;
        mvhd.matrix = self.matrix;
        mvhd.next_track_id = trak
            .iter()
            .map(|trak| trak.tkhd.track_id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        moov.trak = trak;
        moov.meta.clone_from(&self.meta);
        moov.udta.clone_from(&self.udta);
        moov.other_boxes.clone_from(&self.other_boxes);
        set_durations(&mut moov);
        moov
    }

    /// Returns the `moov` of the movie whose tracks `tracks` describe, which declares no sample and extends each track into movie fragments
    ///
    /// Each track has a sample table of no sample, and the `mvex`, §8.8.1, a
    /// `trex`, §8.8.3, for each track, whose default sample entry is the
    /// first. A last edit of duration 0 is kept: in a fragmented movie, it
    /// runs to the end of the media the movie fragments add, §8.6.6.1.
    ///
    /// ```
    /// use isobmff::boxes::SampleDescriptionBox;
    /// use isobmff::muxer::{MovieParameters, TrackParameters};
    /// use isobmff::FourCc;
    ///
    /// let track = TrackParameters::new(1, 90_000, FourCc::new(*b"vide"), SampleDescriptionBox::default());
    /// let moov = MovieParameters::new(1000).to_fragmented_movie_box(&[track]);
    /// assert_eq!(moov.mvex.map(|mvex| mvex.trex.len()), Some(1));
    /// ```
    #[must_use]
    pub fn to_fragmented_movie_box(&self, tracks: &[TrackParameters]) -> MovieBox {
        let dref = data_reference();
        let trak = tracks
            .iter()
            .map(|track| {
                let stbl = SampleTableWriter::new(
                    &TrackSampleDescription {
                        track_id: track.track_id,
                        stsd: &track.stsd,
                        dref: &dref,
                    },
                    0,
                )
                .finish();
                track.to_track_box(stbl)
            })
            .collect();
        let mut moov = self.to_movie_box(trak);
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = tracks
            .iter()
            .map(|track| {
                let mut trex = TrackExtendsBox::default();
                trex.track_id = track.track_id;
                trex.default_sample_description_index = 1;
                trex
            })
            .collect();
        moov.mvex = Some(mvex);
        moov
    }
}

impl From<&MovieBox> for MovieParameters {
    fn from(movie: &MovieBox) -> Self {
        let mvhd = &movie.mvhd;
        Self {
            timescale: mvhd.timescale,
            creation_time: mvhd.creation_time,
            modification_time: mvhd.modification_time,
            rate: mvhd.rate,
            volume: mvhd.volume,
            matrix: mvhd.matrix,
            meta: movie.meta.clone(),
            udta: movie.udta.clone(),
            other_boxes: movie.other_boxes.clone(),
        }
    }
}
