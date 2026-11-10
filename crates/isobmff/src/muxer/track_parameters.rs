//! [`TrackParameters`], what the samples of a track do not give a muxer

use alloc::vec::Vec;

use isobmff_boxes::{
    CompactSampleToGroupBox, EditBox, ExtendedLanguageBox, HintMediaHeaderBox, LanguageCode,
    Matrix, MediaInformationHeader, MetaBox, NullMediaHeaderBox, SampleDescriptionBox,
    SampleGroupDescriptionBox, SampleTableBox, SampleToGroupBox, ShadowSyncSampleBox,
    SoundMediaHeaderBox, SubSampleInformationBox, SubSampleReferenceTableBox,
    SubtitleMediaHeaderBox, TrackBox, TrackGroupBox, TrackReferenceBox, TrackTypeBox, UserDataBox,
    VideoMediaHeaderBox,
};
use isobmff_core::{FourCc, I8F8, OtherBoxes, U16F16, Utf8String};

use super::movie::data_reference;

/// What the samples of a track do not give a muxer: every field and box of its `track` box that is not derived from them or from where the muxer places them
///
/// The `track` box, [`TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1, apart
/// from what is derived: its durations and the versions of its headers
/// (§8.3.2, §8.4.2); the tables its samples declare (§8.6, §8.7), the
/// degradation priority and padding bits of each (§8.5.3, §8.7.6), which the
/// flags of its samples carry; the sample auxiliary information sizes and
/// offsets (§8.7.8, §8.7.9), which follow where the samples lie; and its
/// data references (§8.7.2), the media data being in the same file. A
/// field reads as the field of the box it names, which its doc cites.
///
/// [`new`](Self::new) fills in what the specification gives by default; a
/// track that differs sets the fields after it:
///
/// ```
/// use isobmff::boxes::SampleDescriptionBox;
/// use isobmff::muxer::TrackParameters;
/// use isobmff::FourCc;
///
/// let track = TrackParameters {
///     alternate_group: 1,
///     ..TrackParameters::new(1, 48_000, FourCc::new(*b"soun"), SampleDescriptionBox::default())
/// };
/// assert_eq!(track.volume, isobmff::I8F8::ONE);
/// ```
///
/// A value a muxer derives cannot be set:
///
/// ```compile_fail
/// # use isobmff::boxes::SampleDescriptionBox;
/// # use isobmff::FourCc;
/// let mut track = isobmff::muxer::TrackParameters::new(1, 1000, FourCc::new(*b"vide"), SampleDescriptionBox::default());
/// track.duration = 60_000;
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "the fields are the fields of the boxes a caller sets; a field the specification adds is a change the caller has to see"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct TrackParameters {
    /// `track_ID` of the track, other than 0, §8.3.2
    pub track_id: u32,
    /// Whether the track is enabled, flag `track_enabled` of the `tkhd`
    pub track_enabled: bool,
    /// Whether the track is used in the presentation, flag `track_in_movie`
    pub track_in_movie: bool,
    /// Whether the track is used in previews, flag `track_in_preview`
    pub track_in_preview: bool,
    /// Whether `width` and `height` are an aspect ratio, flag
    /// `track_size_is_aspect_ratio`
    pub track_size_is_aspect_ratio: bool,
    /// `creation_time` of the `tkhd`, in seconds since midnight, Jan. 1,
    /// 1904, in UTC
    pub creation_time: u64,
    /// `modification_time` of the `tkhd`, in seconds since midnight, Jan. 1,
    /// 1904, in UTC
    pub modification_time: u64,
    /// `layer` of the `tkhd`, the order of the visual tracks front to back
    pub layer: i16,
    /// `alternate_group` of the `tkhd`, 0 for a track of no group
    pub alternate_group: i16,
    /// `volume` of the `tkhd`
    pub volume: I8F8,
    /// `matrix` of the `tkhd`, the transformation of the video
    pub matrix: Matrix,
    /// `width` of the `tkhd`; 0 for a track that has no visual presentation
    pub width: U16F16,
    /// `height` of the `tkhd`; 0 for a track that has no visual presentation
    pub height: U16F16,
    /// The `ttyp` of the track, §8.3.5
    pub ttyp: Option<TrackTypeBox>,
    /// The `tref` of the track, §8.3.3
    pub tref: Option<TrackReferenceBox>,
    /// The `trgr` of the track, §8.3.4
    pub trgr: Option<TrackGroupBox>,
    /// The `edts` of the track, §8.6.5, with its edit list; the duration of a
    /// last edit of duration 0 is set from the samples, as
    /// [`MovieParameters::to_movie_box`](super::MovieParameters::to_movie_box)
    /// says
    pub edts: Option<EditBox>,
    /// The `meta` of the track, §8.11.1
    pub meta: Option<MetaBox>,
    /// The `udta` of the track, §8.10.1
    pub udta: Vec<UserDataBox>,
    /// `timescale` of the `mdhd`, §8.4.2, the number of time units of the
    /// media in a second
    pub timescale: u32,
    /// `creation_time` of the `mdhd`
    pub media_creation_time: u64,
    /// `modification_time` of the `mdhd`
    pub media_modification_time: u64,
    /// `language` of the `mdhd`
    pub language: LanguageCode,
    /// The `elng` of the media, §8.4.6
    pub elng: Option<ExtendedLanguageBox>,
    /// `handler_type` of the `hdlr`, §8.4.3, such as `vide` or `soun`
    pub handler_type: FourCc,
    /// `name` of the `hdlr`
    #[doc(alias = "name")]
    pub handler_name: Utf8String,
    /// The media header of the track, §8.4.5, §12
    pub media_information_header: Option<MediaInformationHeader>,
    /// The sample entries, §8.5.2, each of whose `data_reference_index` is 1:
    /// the media data in the same file
    pub stsd: SampleDescriptionBox,
    /// The `sbgp`s of the sample table, §8.9.2
    pub sbgp: Vec<SampleToGroupBox>,
    /// The `csgp`s of the sample table, §8.9.5
    pub csgp: Vec<CompactSampleToGroupBox>,
    /// The `sgpd`s of the sample table, §8.9.3
    pub sgpd: Vec<SampleGroupDescriptionBox>,
    /// The `subs`s of the sample table, §8.7.7
    pub subs: Vec<SubSampleInformationBox>,
    /// The `ssrt`s of the sample table, §8.7.10
    pub ssrt: Vec<SubSampleReferenceTableBox>,
    /// The `stsh` of the sample table, §8.6.3
    pub stsh: Option<ShadowSyncSampleBox>,
    /// Children of the `trak` of a type the crate holds no field for, written
    /// last
    pub other_boxes: OtherBoxes,
    /// Children of the `mdia`, §8.4.1, of a type the crate holds no field for
    pub media_other_boxes: OtherBoxes,
    /// Children of the `minf`, §8.4.4, of a type the crate holds no field for
    pub media_information_other_boxes: OtherBoxes,
    /// Children of the `dinf`, §8.7.1, of a type the crate holds no field for
    pub data_information_other_boxes: OtherBoxes,
    /// Children of the `stbl`, §8.5.1, of a type the crate holds no field for
    pub sample_table_other_boxes: OtherBoxes,
}

impl TrackParameters {
    /// Creates the parameters of track `track_id`, of media in `timescale` time units a second, that `handler_type` handles and `stsd` describes, every other field as the specification gives it by default
    ///
    /// The track is enabled, in the movie and in previews, created and
    /// modified at time 0, of language `und`, under the unity matrix, of a
    /// volume of 1.0 for `soun` and 0 for any other handler, and of the media
    /// header `handler_type` chooses (§12); it has no edit list and no other
    /// box.
    #[must_use]
    pub fn new(
        track_id: u32,
        timescale: u32,
        handler_type: FourCc,
        stsd: SampleDescriptionBox,
    ) -> Self {
        Self {
            track_id,
            track_enabled: true,
            track_in_movie: true,
            track_in_preview: true,
            track_size_is_aspect_ratio: false,
            creation_time: 0,
            modification_time: 0,
            layer: 0,
            alternate_group: 0,
            volume: if handler_type.as_bytes() == b"soun" {
                I8F8::ONE
            } else {
                I8F8::ZERO
            },
            matrix: Matrix::UNITY,
            width: U16F16::ZERO,
            height: U16F16::ZERO,
            ttyp: None,
            tref: None,
            trgr: None,
            edts: None,
            meta: None,
            udta: Vec::new(),
            timescale,
            media_creation_time: 0,
            media_modification_time: 0,
            language: LanguageCode::UND,
            elng: None,
            handler_type,
            handler_name: Utf8String::default(),
            media_information_header: Some(match handler_type.as_bytes() {
                b"vide" => MediaInformationHeader::Video(VideoMediaHeaderBox::default()),
                b"soun" => MediaInformationHeader::Sound(SoundMediaHeaderBox::default()),
                b"hint" => MediaInformationHeader::Hint(HintMediaHeaderBox::default()),
                b"subt" => MediaInformationHeader::Subtitle(SubtitleMediaHeaderBox::default()),
                _ => MediaInformationHeader::Null(NullMediaHeaderBox::default()),
            }),
            stsd,
            sbgp: Vec::new(),
            csgp: Vec::new(),
            sgpd: Vec::new(),
            subs: Vec::new(),
            ssrt: Vec::new(),
            stsh: None,
            other_boxes: OtherBoxes::default(),
            media_other_boxes: OtherBoxes::default(),
            media_information_other_boxes: OtherBoxes::default(),
            data_information_other_boxes: OtherBoxes::default(),
            sample_table_other_boxes: OtherBoxes::default(),
        }
    }

    /// Returns the `trak` of the track whose samples `stbl` declares
    ///
    /// The fields and boxes the parameters hold are written as they are:
    /// the sample entries and the boxes of the sample table the parameters
    /// hold replace those of `stbl`, and the data reference is one entry,
    /// the media data in the same file. The durations and the versions of
    /// the headers are left at 0, for
    /// [`MovieParameters::to_movie_box`](super::MovieParameters::to_movie_box)
    /// to set.
    ///
    /// ```
    /// use isobmff::boxes::{SampleDescriptionBox, SampleTableBox};
    /// use isobmff::muxer::TrackParameters;
    /// use isobmff::FourCc;
    ///
    /// let track = TrackParameters::new(1, 90_000, FourCc::new(*b"vide"), SampleDescriptionBox::default());
    /// let trak = track.to_track_box(SampleTableBox::default());
    /// assert_eq!((trak.tkhd.track_id, trak.mdia.mdhd.timescale), (1, 90_000));
    /// ```
    #[must_use]
    pub fn to_track_box(&self, stbl: SampleTableBox) -> TrackBox {
        let mut trak = TrackBox::default();
        let tkhd = &mut trak.tkhd;
        tkhd.track_enabled = self.track_enabled;
        tkhd.track_in_movie = self.track_in_movie;
        tkhd.track_in_preview = self.track_in_preview;
        tkhd.track_size_is_aspect_ratio = self.track_size_is_aspect_ratio;
        tkhd.creation_time = self.creation_time;
        tkhd.modification_time = self.modification_time;
        tkhd.track_id = self.track_id;
        tkhd.layer = self.layer;
        tkhd.alternate_group = self.alternate_group;
        tkhd.volume = self.volume;
        tkhd.matrix = self.matrix;
        tkhd.width = self.width;
        tkhd.height = self.height;
        trak.ttyp.clone_from(&self.ttyp);
        trak.tref.clone_from(&self.tref);
        trak.trgr.clone_from(&self.trgr);
        trak.edts.clone_from(&self.edts);
        trak.meta.clone_from(&self.meta);
        trak.udta.clone_from(&self.udta);
        trak.other_boxes.clone_from(&self.other_boxes);
        let mdia = &mut trak.mdia;
        mdia.mdhd.creation_time = self.media_creation_time;
        mdia.mdhd.modification_time = self.media_modification_time;
        mdia.mdhd.timescale = self.timescale;
        mdia.mdhd.language = self.language;
        mdia.elng.clone_from(&self.elng);
        mdia.hdlr.handler_type = self.handler_type;
        mdia.hdlr.name.clone_from(&self.handler_name);
        mdia.other_boxes.clone_from(&self.media_other_boxes);
        let minf = &mut mdia.minf;
        minf.media_information_header
            .clone_from(&self.media_information_header);
        minf.dinf.dref = data_reference();
        minf.dinf
            .other_boxes
            .clone_from(&self.data_information_other_boxes);
        minf.other_boxes
            .clone_from(&self.media_information_other_boxes);
        minf.stbl = stbl;
        let stbl = &mut minf.stbl;
        stbl.stsd.clone_from(&self.stsd);
        stbl.sbgp.clone_from(&self.sbgp);
        stbl.csgp.clone_from(&self.csgp);
        stbl.sgpd.clone_from(&self.sgpd);
        stbl.subs.clone_from(&self.subs);
        stbl.ssrt.clone_from(&self.ssrt);
        stbl.stsh.clone_from(&self.stsh);
        stbl.other_boxes.clone_from(&self.sample_table_other_boxes);

        trak
    }
}

impl From<&TrackBox> for TrackParameters {
    fn from(track: &TrackBox) -> Self {
        let (tkhd, mdia) = (&track.tkhd, &track.mdia);
        let stbl = &mdia.minf.stbl;
        Self {
            track_id: tkhd.track_id,
            track_enabled: tkhd.track_enabled,
            track_in_movie: tkhd.track_in_movie,
            track_in_preview: tkhd.track_in_preview,
            track_size_is_aspect_ratio: tkhd.track_size_is_aspect_ratio,
            creation_time: tkhd.creation_time,
            modification_time: tkhd.modification_time,
            layer: tkhd.layer,
            alternate_group: tkhd.alternate_group,
            volume: tkhd.volume,
            matrix: tkhd.matrix,
            width: tkhd.width,
            height: tkhd.height,
            ttyp: track.ttyp.clone(),
            tref: track.tref.clone(),
            trgr: track.trgr.clone(),
            edts: track.edts.clone(),
            meta: track.meta.clone(),
            udta: track.udta.clone(),
            timescale: mdia.mdhd.timescale,
            media_creation_time: mdia.mdhd.creation_time,
            media_modification_time: mdia.mdhd.modification_time,
            language: mdia.mdhd.language,
            elng: mdia.elng.clone(),
            handler_type: mdia.hdlr.handler_type,
            handler_name: mdia.hdlr.name.clone(),
            media_information_header: mdia.minf.media_information_header.clone(),
            stsd: stbl.stsd.clone(),
            sbgp: stbl.sbgp.clone(),
            csgp: stbl.csgp.clone(),
            sgpd: stbl.sgpd.clone(),
            subs: stbl.subs.clone(),
            ssrt: stbl.ssrt.clone(),
            stsh: stbl.stsh.clone(),
            other_boxes: track.other_boxes.clone(),
            media_other_boxes: mdia.other_boxes.clone(),
            media_information_other_boxes: mdia.minf.other_boxes.clone(),
            data_information_other_boxes: mdia.minf.dinf.other_boxes.clone(),
            sample_table_other_boxes: stbl.other_boxes.clone(),
        }
    }
}
