//! [`MovieDescription`] and [`TrackDescription`], what a muxer cannot derive of a movie and its tracks

use isobmff_boxes::{EditListBox, MovieBox, SampleDescriptionBox, TrackBox};
use isobmff_core::{FourCc, OtherBoxes, U16F16};

/// What a writer of a movie cannot derive from its samples: the timescale of the movie
///
/// The `movie header`, [`MovieHeaderBox`](isobmff_boxes::MovieHeaderBox)
/// (`mvhd`), ISO/IEC 14496-12 §8.2.2, apart from what the tracks give it: its
/// duration and the next `track_ID`.
///
/// A description holds no table, duration or movie extends box: a
/// [`Muxer`](super::Muxer) derives those. One is built from its timescale, or
/// taken from a `moov` read, to write the movie again:
///
/// ```
/// use isobmff::muxer::MovieDescription;
///
/// let mut movie = MovieDescription::new(1000);
/// movie.timescale = 600;
/// ```
///
/// A value a muxer derives cannot be set:
///
/// ```compile_fail
/// let mut movie = isobmff::muxer::MovieDescription::new(1000);
/// movie.duration = 60_000;
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct MovieDescription {
    /// `timescale` of the movie, the number of time units in a second
    pub timescale: u32,
    /// Children of the `moov` of a type the crate holds no field for, written
    /// after the tracks
    pub other_boxes: OtherBoxes,
}

impl MovieDescription {
    /// Creates the description of a movie of `timescale` time units a second
    #[must_use]
    pub fn new(timescale: u32) -> Self {
        Self {
            timescale,
            other_boxes: OtherBoxes::default(),
        }
    }
}

impl From<&MovieBox> for MovieDescription {
    fn from(movie: &MovieBox) -> Self {
        Self {
            timescale: movie.mvhd.timescale,
            other_boxes: movie.other_boxes.clone(),
        }
    }
}

/// What a writer of a movie cannot derive from the samples of a track: its `track_ID`, timescale, `handler_type`, sample entries and edit list
///
/// The `track` box, [`TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1, apart
/// from what its samples give it: its durations and its sample tables.
///
/// ```
/// use isobmff::muxer::TrackDescription;
/// use isobmff_boxes::SampleDescriptionBox;
/// use isobmff_core::FourCc;
///
/// let track = TrackDescription::new(1, 90_000, FourCc::new(*b"vide"), SampleDescriptionBox::default());
/// assert_eq!(track.edit_list, None);
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct TrackDescription {
    /// `track_ID` of the track, other than 0
    pub track_id: u32,
    /// `timescale` of the media of the track, §8.4.2
    pub timescale: u32,
    /// `handler_type` of the media, §8.4.3, such as `vide` or `soun`; it
    /// chooses the media header of the track, §12
    pub handler_type: FourCc,
    /// The sample entries, §8.5.2, each of whose `data_reference_index` is 1:
    /// the media data in the same file
    pub sample_entries: SampleDescriptionBox,
    /// The edit list of the track, §8.6.6, if it has one
    pub edit_list: Option<EditListBox>,
    /// `width` of the visual presentation of the track, §8.3.2; 0 for a
    /// track that has none
    pub width: U16F16,
    /// `height` of the visual presentation of the track, §8.3.2; 0 for a
    /// track that has none
    pub height: U16F16,
    /// Children of the `trak` of a type the crate holds no field for, written
    /// last
    pub other_boxes: OtherBoxes,
}

impl TrackDescription {
    /// Creates the description of track `track_id`, of media in `timescale` time units a second, that `handler_type` handles and `sample_entries` describe
    #[must_use]
    pub fn new(
        track_id: u32,
        timescale: u32,
        handler_type: FourCc,
        sample_entries: SampleDescriptionBox,
    ) -> Self {
        Self {
            track_id,
            timescale,
            handler_type,
            sample_entries,
            edit_list: None,
            width: U16F16::ZERO,
            height: U16F16::ZERO,
            other_boxes: OtherBoxes::default(),
        }
    }
}

impl From<&TrackBox> for TrackDescription {
    fn from(track: &TrackBox) -> Self {
        Self {
            track_id: track.tkhd.track_id,
            timescale: track.mdia.mdhd.timescale,
            handler_type: track.mdia.hdlr.handler_type,
            sample_entries: track.mdia.minf.stbl.stsd.clone(),
            edit_list: track.edts.as_ref().and_then(|edts| edts.elst.clone()),
            width: track.tkhd.width,
            height: track.tkhd.height,
            other_boxes: track.other_boxes.clone(),
        }
    }
}
