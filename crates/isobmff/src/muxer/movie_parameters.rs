//! [`MovieParameters`], what the samples of a movie do not give a muxer

use alloc::vec::Vec;

use isobmff_boxes::{Matrix, MetaBox, MovieBox, TrackBox, UserDataBox};
use isobmff_core::{I8F8, I16F16, OtherBoxes};

use super::TrackParameters;
use super::movie::data_reference;
use crate::structure_data::{
    SampleTableWriter, TrackSampleDescription, movie_extends_box, set_durations,
};

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
    /// The durations and versions are set as
    /// [`set_durations`](crate::structure_data::set_durations) sets them,
    /// and the next `track_ID` is one more than the largest of the tracks,
    /// §8.2.2.
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
    /// Each track has a sample table of no sample, and the `mvex` is the one
    /// [`movie_extends_box`](crate::structure_data::movie_extends_box)
    /// returns. A last edit of duration 0 is kept: in a fragmented movie, it
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
        moov.mvex = Some(movie_extends_box(&moov.trak));
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
