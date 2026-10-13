//! [`isobmff::boxes`]: [`CONTAINER_BOXES`](isobmff::boxes::CONTAINER_BOXES), the boxes from the `moov` down to the `stsd`, [`HeaderDuration`] and [`SampleFlags`]

use isobmff::boxes;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::framing::{AnyBox, BoxType, FourCc};

/// [`isobmff::boxes::CONTAINER_BOXES`], called as a function
#[wasm_bindgen(js_name = CONTAINER_BOXES)]
pub fn container_boxes() -> Vec<BoxType> {
    boxes::CONTAINER_BOXES
        .iter()
        .copied()
        .map(BoxType)
        .collect()
}

/// [`isobmff::boxes::HeaderDuration`]
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct HeaderDuration(boxes::HeaderDuration);

#[wasm_bindgen]
impl HeaderDuration {
    /// [`isobmff::boxes::HeaderDuration::get`]
    pub fn get(&self) -> Option<u64> {
        self.0.get()
    }
}

/// [`isobmff::boxes::SampleFlags`]
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct SampleFlags(pub(crate) boxes::SampleFlags);

#[wasm_bindgen]
impl SampleFlags {
    /// [`isobmff::boxes::SampleFlags::sample_is_non_sync_sample`]
    pub fn sample_is_non_sync_sample(&self) -> bool {
        self.0.sample_is_non_sync_sample()
    }
}

/// [`isobmff::boxes::MovieBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct MovieBox(pub(crate) boxes::MovieBox);

#[wasm_bindgen]
impl MovieBox {
    /// [`isobmff::boxes::MovieBox::trak`], copied
    pub fn trak(&self) -> Vec<TrackBox> {
        self.0.trak().iter().cloned().map(TrackBox).collect()
    }
}

/// [`isobmff::boxes::TrackBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct TrackBox(boxes::TrackBox);

#[wasm_bindgen]
impl TrackBox {
    /// [`isobmff::boxes::TrackBox::tkhd`], copied
    pub fn tkhd(&self) -> TrackHeaderBox {
        TrackHeaderBox(self.0.tkhd().clone())
    }

    /// [`isobmff::boxes::TrackBox::mdia`], copied
    pub fn mdia(&self) -> MediaBox {
        MediaBox(self.0.mdia().clone())
    }
}

/// [`isobmff::boxes::TrackHeaderBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct TrackHeaderBox(boxes::TrackHeaderBox);

#[wasm_bindgen]
impl TrackHeaderBox {
    /// [`isobmff::boxes::TrackHeaderBox::track_id`]
    pub fn track_id(&self) -> u32 {
        self.0.track_id()
    }
}

/// [`isobmff::boxes::MediaBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct MediaBox(boxes::MediaBox);

#[wasm_bindgen]
impl MediaBox {
    /// [`isobmff::boxes::MediaBox::mdhd`], copied
    pub fn mdhd(&self) -> MediaHeaderBox {
        MediaHeaderBox(self.0.mdhd().clone())
    }

    /// [`isobmff::boxes::MediaBox::hdlr`], copied
    pub fn hdlr(&self) -> HandlerBox {
        HandlerBox(self.0.hdlr().clone())
    }

    /// [`isobmff::boxes::MediaBox::minf`], copied
    pub fn minf(&self) -> MediaInformationBox {
        MediaInformationBox(self.0.minf().clone())
    }
}

/// [`isobmff::boxes::MediaHeaderBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct MediaHeaderBox(boxes::MediaHeaderBox);

#[wasm_bindgen]
impl MediaHeaderBox {
    /// [`isobmff::boxes::MediaHeaderBox::timescale`]
    pub fn timescale(&self) -> u32 {
        self.0.timescale()
    }

    /// [`isobmff::boxes::MediaHeaderBox::duration`]
    pub fn duration(&self) -> HeaderDuration {
        HeaderDuration(self.0.duration())
    }
}

/// [`isobmff::boxes::HandlerBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct HandlerBox(boxes::HandlerBox);

#[wasm_bindgen]
impl HandlerBox {
    /// [`isobmff::boxes::HandlerBox::handler_type`]
    pub fn handler_type(&self) -> FourCc {
        FourCc(self.0.handler_type())
    }
}

/// [`isobmff::boxes::MediaInformationBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct MediaInformationBox(boxes::MediaInformationBox);

#[wasm_bindgen]
impl MediaInformationBox {
    /// [`isobmff::boxes::MediaInformationBox::stbl`], copied
    pub fn stbl(&self) -> SampleTableBox {
        SampleTableBox(self.0.stbl().clone())
    }
}

/// [`isobmff::boxes::SampleTableBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct SampleTableBox(boxes::SampleTableBox);

#[wasm_bindgen]
impl SampleTableBox {
    /// [`isobmff::boxes::SampleTableBox::stsd`], copied
    pub fn stsd(&self) -> SampleDescriptionBox {
        SampleDescriptionBox(self.0.stsd().clone())
    }
}

/// [`isobmff::boxes::SampleDescriptionBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct SampleDescriptionBox(boxes::SampleDescriptionBox);

#[wasm_bindgen]
impl SampleDescriptionBox {
    /// [`isobmff::boxes::SampleDescriptionBox::entries`], copied
    pub fn entries(&self) -> Vec<AnyBox> {
        self.0.entries().iter().cloned().map(AnyBox).collect()
    }
}
