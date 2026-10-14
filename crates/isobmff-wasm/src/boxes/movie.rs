//! The boxes of ISO/IEC 14496-12 §8.2.1 (`moov`) to §8.4.4 (`minf`), each a root of its own or a handle into one

use alloc::rc::Rc;
use core::cell::RefCell;

use isobmff::boxes;
use wasm_bindgen::prelude::wasm_bindgen;

use super::Place;
use super::sample_table::SampleTableBox;
use crate::Failure;

/// [`isobmff::boxes::MovieBox`] (`moov`), ISO/IEC 14496-12 §8.2.1, the root its handles share
#[doc(alias = "moov")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MovieBox(Rc<RefCell<boxes::MovieBox>>);

impl MovieBox {
    /// Makes `movie` the root of the handles reached from it
    pub(crate) fn new_root(movie: boxes::MovieBox) -> Self {
        Self(Rc::new(RefCell::new(movie)))
    }

    /// Returns a copy of the box, as the library takes it
    pub(crate) fn value(&self) -> boxes::MovieBox {
        self.0.borrow().clone()
    }

    /// Returns the place of the box, the root of its handles
    fn place(&self) -> Place<boxes::MovieBox> {
        Place::shared(Rc::clone(&self.0))
    }
}

#[wasm_bindgen]
impl MovieBox {
    /// [`isobmff::boxes::MovieBox::new`] with no `mvex`, taking the handles of `trak`, a root of its own, or none where the library refuses the tracks
    ///
    /// # Errors
    ///
    /// `"handle"`: the path of `mvhd` or of a track leads to no box.
    pub fn new(mvhd: &MovieHeaderBox, trak: Vec<TrackBox>) -> Result<Option<Self>, Failure> {
        Ok(
            boxes::MovieBox::new(mvhd.0.read(Clone::clone)?, TrackBox::values(&trak)?, None)
                .map(Self::new_root),
        )
    }

    /// [`isobmff::boxes::MovieBox::new_fragmented`], taking the handles of `tracks`, a root of its own, or none where the library refuses the tracks
    ///
    /// # Errors
    ///
    /// `"handle"`: the path of a track leads to no box.
    #[wasm_bindgen(js_name = newFragmented)]
    pub fn new_fragmented(timescale: u32, tracks: Vec<TrackBox>) -> Result<Option<Self>, Failure> {
        Ok(
            boxes::MovieBox::new_fragmented(timescale, TrackBox::values(&tracks)?)
                .map(Self::new_root),
        )
    }

    /// [`isobmff::boxes::MovieBox::mvhd`], a handle
    pub fn mvhd(&self) -> MovieHeaderBox {
        MovieHeaderBox(self.place().child(|movie| Some(movie.mvhd_mut())))
    }

    /// [`isobmff::boxes::MovieBox::trak`], a handle to each
    pub fn trak(&self) -> Vec<TrackBox> {
        let place = self.place();
        (0..self.0.borrow().trak().len())
            .map(|position| TrackBox(place.child(move |movie| movie.trak_mut().get_mut(position))))
            .collect()
    }

    /// [`isobmff::boxes::MovieBox::trak_by_id`], a handle following the `track_ID`
    #[wasm_bindgen(js_name = trakById)]
    pub fn trak_by_id(&self, track_id: u32) -> Option<TrackBox> {
        let declared = self.0.borrow().trak_by_id(track_id).is_some();
        declared.then(|| {
            TrackBox(
                self.place()
                    .child(move |movie| movie.trak_by_id_mut(track_id)),
            )
        })
    }
}

/// [`isobmff::boxes::MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2, a handle or a root of its own
#[doc(alias = "mvhd")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MovieHeaderBox(Place<boxes::MovieHeaderBox>);

#[wasm_bindgen]
impl MovieHeaderBox {
    /// [`isobmff::boxes::MovieHeaderBox::timescale`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    pub fn timescale(&self) -> Result<u32, Failure> {
        self.0.read(boxes::MovieHeaderBox::timescale)
    }

    /// [`isobmff::boxes::MovieHeaderBox::with_modification_time`] at the seconds since 1904, a root of its own
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    #[wasm_bindgen(js_name = withModificationTime)]
    pub fn with_modification_time(&self, modification_time: u64) -> Result<Self, Failure> {
        let header = self.0.read(|header| {
            header
                .clone()
                .with_modification_time(isobmff::Mp4EpochSeconds::from_seconds(modification_time))
        })?;
        Ok(Self(Place::root(header)))
    }
}

/// [`isobmff::boxes::TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1, a handle into its root
#[doc(alias = "trak")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct TrackBox(Place<boxes::TrackBox>);

impl TrackBox {
    /// Returns copies of the boxes `tracks` lead to, as the library takes them
    ///
    /// # Errors
    ///
    /// `"handle"`: the path of a track leads to no box.
    fn values(tracks: &[Self]) -> Result<Vec<boxes::TrackBox>, Failure> {
        tracks
            .iter()
            .map(|track| track.0.read(Clone::clone))
            .collect()
    }
}

#[wasm_bindgen]
impl TrackBox {
    /// [`isobmff::boxes::TrackBox::tkhd`], a handle
    pub fn tkhd(&self) -> TrackHeaderBox {
        TrackHeaderBox(self.0.child(|track| Some(track.tkhd_mut())))
    }

    /// [`isobmff::boxes::TrackBox::tkhd_mut`], set to a copy of `tkhd`
    ///
    /// # Errors
    ///
    /// `"handle"`: the path of either box leads to no box.
    #[wasm_bindgen(js_name = setTkhd)]
    pub fn set_tkhd(&self, tkhd: &TrackHeaderBox) -> Result<(), Failure> {
        let tkhd = tkhd.0.read(Clone::clone)?;
        self.0.write(|track| *track.tkhd_mut() = tkhd)
    }

    /// [`isobmff::boxes::TrackBox::mdia`], a handle
    pub fn mdia(&self) -> MediaBox {
        MediaBox(self.0.child(|track| Some(track.mdia_mut())))
    }
}

/// [`isobmff::boxes::TrackHeaderBox`] (`tkhd`), ISO/IEC 14496-12 §8.3.2, a handle or a root of its own
#[doc(alias = "tkhd")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct TrackHeaderBox(Place<boxes::TrackHeaderBox>);

#[wasm_bindgen]
impl TrackHeaderBox {
    /// [`isobmff::boxes::TrackHeaderBox::track_id`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    #[wasm_bindgen(js_name = trackId)]
    pub fn track_id(&self) -> Result<u32, Failure> {
        self.0.read(boxes::TrackHeaderBox::track_id)
    }

    /// [`isobmff::boxes::TrackHeaderBox::with_modification_time`] at the seconds since 1904, a root of its own
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    #[wasm_bindgen(js_name = withModificationTime)]
    pub fn with_modification_time(&self, modification_time: u64) -> Result<Self, Failure> {
        let header = self.0.read(|header| {
            header
                .clone()
                .with_modification_time(isobmff::Mp4EpochSeconds::from_seconds(modification_time))
        })?;
        Ok(Self(Place::root(header)))
    }
}

/// [`isobmff::boxes::MediaBox`] (`mdia`), ISO/IEC 14496-12 §8.4.1, a handle into its root
#[doc(alias = "mdia")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MediaBox(Place<boxes::MediaBox>);

#[wasm_bindgen]
impl MediaBox {
    /// [`isobmff::boxes::MediaBox::mdhd`], a handle
    pub fn mdhd(&self) -> MediaHeaderBox {
        MediaHeaderBox(self.0.child(|media| Some(media.mdhd_mut())))
    }

    /// [`isobmff::boxes::MediaBox::mdhd_mut`], set to a copy of `mdhd`
    ///
    /// # Errors
    ///
    /// `"handle"`: the path of either box leads to no box.
    #[wasm_bindgen(js_name = setMdhd)]
    pub fn set_mdhd(&self, mdhd: &MediaHeaderBox) -> Result<(), Failure> {
        let mdhd = mdhd.0.read(Clone::clone)?;
        self.0.write(|media| *media.mdhd_mut() = mdhd)
    }

    /// [`isobmff::boxes::MediaBox::hdlr`], a handle
    pub fn hdlr(&self) -> HandlerBox {
        HandlerBox(self.0.clone())
    }

    /// [`isobmff::boxes::MediaBox::minf`], a handle
    pub fn minf(&self) -> MediaInformationBox {
        MediaInformationBox(self.0.child(|media| Some(media.minf_mut())))
    }
}

/// [`isobmff::boxes::MediaHeaderBox`] (`mdhd`), ISO/IEC 14496-12 §8.4.2, a handle or a root of its own
#[doc(alias = "mdhd")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MediaHeaderBox(Place<boxes::MediaHeaderBox>);

#[wasm_bindgen]
impl MediaHeaderBox {
    /// [`isobmff::boxes::MediaHeaderBox::timescale`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    pub fn timescale(&self) -> Result<u32, Failure> {
        self.0.read(boxes::MediaHeaderBox::timescale)
    }

    /// [`isobmff::boxes::MediaHeaderBox::duration`], none where it is indeterminate
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    pub fn duration(&self) -> Result<Option<u64>, Failure> {
        self.0.read(|header| header.duration().get())
    }

    /// [`isobmff::boxes::MediaHeaderBox::with_modification_time`] at the seconds since 1904, a root of its own
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    #[wasm_bindgen(js_name = withModificationTime)]
    pub fn with_modification_time(&self, modification_time: u64) -> Result<Self, Failure> {
        let header = self.0.read(|header| {
            header
                .clone()
                .with_modification_time(isobmff::Mp4EpochSeconds::from_seconds(modification_time))
        })?;
        Ok(Self(Place::root(header)))
    }
}

/// [`isobmff::boxes::HandlerBox`] (`hdlr`), ISO/IEC 14496-12 §8.4.3, a handle into its root
#[doc(alias = "hdlr")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct HandlerBox(Place<boxes::MediaBox>);

#[wasm_bindgen]
impl HandlerBox {
    /// [`isobmff::boxes::HandlerBox::handler_type`], the four characters as [`isobmff::FourCc`] displays them
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    #[wasm_bindgen(js_name = handlerType)]
    pub fn handler_type(&self) -> Result<String, Failure> {
        self.0.read(|media| media.hdlr().handler_type().to_string())
    }
}

/// [`isobmff::boxes::MediaInformationBox`] (`minf`), ISO/IEC 14496-12 §8.4.4, a handle into its root
#[doc(alias = "minf")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MediaInformationBox(Place<boxes::MediaInformationBox>);

#[wasm_bindgen]
impl MediaInformationBox {
    /// [`isobmff::boxes::MediaInformationBox::stbl`], a handle
    pub fn stbl(&self) -> SampleTableBox {
        SampleTableBox(self.0.child(|media| Some(media.stbl_mut())))
    }

    /// [`isobmff::boxes::MediaInformationBox::stbl_mut`], set to a copy of `stbl`
    ///
    /// # Errors
    ///
    /// `"handle"`: the path of either box leads to no box.
    #[wasm_bindgen(js_name = setStbl)]
    pub fn set_stbl(&self, stbl: &SampleTableBox) -> Result<(), Failure> {
        let stbl = stbl.0.read(Clone::clone)?;
        self.0.write(|media| *media.stbl_mut() = stbl)
    }
}
