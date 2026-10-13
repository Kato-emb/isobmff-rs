//! [`isobmff::sample`]: [`SampleExtent`] and the [`SampleProperties`] it carries

use isobmff::sample;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::Range;
use crate::boxes::SampleFlags;

/// [`isobmff::sample::SampleProperties`], its fields read as properties
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct SampleProperties(sample::SampleProperties);

#[wasm_bindgen]
impl SampleProperties {
    /// [`isobmff::sample::SampleProperties::track_id`]
    #[wasm_bindgen(getter)]
    pub fn track_id(&self) -> u32 {
        self.0.track_id
    }

    /// [`isobmff::sample::SampleProperties::decode_time`]
    #[wasm_bindgen(getter)]
    pub fn decode_time(&self) -> u64 {
        self.0.decode_time
    }

    /// [`isobmff::sample::SampleProperties::sample_duration`]
    #[wasm_bindgen(getter)]
    pub fn sample_duration(&self) -> u32 {
        self.0.sample_duration
    }

    /// [`isobmff::sample::SampleProperties::sample_composition_time_offset`]
    #[wasm_bindgen(getter)]
    pub fn sample_composition_time_offset(&self) -> i64 {
        self.0.sample_composition_time_offset
    }

    /// [`isobmff::sample::SampleProperties::sample_flags`]
    #[wasm_bindgen(getter)]
    pub fn sample_flags(&self) -> SampleFlags {
        SampleFlags(self.0.sample_flags)
    }

    /// [`isobmff::sample::SampleProperties::sample_description_index`]
    #[wasm_bindgen(getter)]
    pub fn sample_description_index(&self) -> u32 {
        self.0.sample_description_index
    }
}

/// [`isobmff::sample::SampleExtent`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct SampleExtent(pub(crate) sample::SampleExtent);

#[wasm_bindgen]
impl SampleExtent {
    /// [`isobmff::sample::SampleExtent::properties`], copied
    pub fn properties(&self) -> SampleProperties {
        SampleProperties(*self.0.properties())
    }

    /// [`isobmff::sample::SampleExtent::extent`]
    pub fn extent(&self) -> Range {
        Range::from(self.0.extent())
    }
}
