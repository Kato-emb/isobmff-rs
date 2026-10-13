//! [`isobmff::sample`]: [`SampleExtent`] and the [`SampleProperties`] it carries

use isobmff::sample;
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::Range;
use crate::boxes::SampleFlags;

/// [`isobmff::sample::SampleProperties`]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleProperties {
    /// [`isobmff::sample::SampleProperties::track_id`]
    track_id: u32,
    /// [`isobmff::sample::SampleProperties::decode_time`]
    decode_time: u64,
    /// [`isobmff::sample::SampleProperties::sample_duration`]
    sample_duration: u32,
    /// [`isobmff::sample::SampleProperties::sample_composition_time_offset`]
    sample_composition_time_offset: i64,
    /// [`isobmff::sample::SampleProperties::sample_flags`]
    sample_flags: SampleFlags,
    /// [`isobmff::sample::SampleProperties::sample_description_index`]
    sample_description_index: u32,
}

impl From<sample::SampleProperties> for SampleProperties {
    fn from(properties: sample::SampleProperties) -> Self {
        Self {
            track_id: properties.track_id,
            decode_time: properties.decode_time,
            sample_duration: properties.sample_duration,
            sample_composition_time_offset: properties.sample_composition_time_offset,
            sample_flags: properties.sample_flags.into(),
            sample_description_index: properties.sample_description_index,
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const SAMPLE_PROPERTIES: &str = r#"
/** `isobmff::sample::SampleProperties` */
export interface SampleProperties {
    trackId: number;
    decodeTime: bigint;
    sampleDuration: number;
    sampleCompositionTimeOffset: bigint;
    sampleFlags: SampleFlags;
    sampleDescriptionIndex: number;
}
"#;

/// [`isobmff::sample::SampleExtent`]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleExtent {
    /// [`isobmff::sample::SampleExtent::properties`]
    properties: SampleProperties,
    /// [`isobmff::sample::SampleExtent::data_reference_index`]
    data_reference_index: u16,
    /// [`isobmff::sample::SampleExtent::extent`]
    extent: Range,
}

impl From<sample::SampleExtent> for SampleExtent {
    fn from(extent: sample::SampleExtent) -> Self {
        Self {
            properties: (*extent.properties()).into(),
            data_reference_index: extent.data_reference_index(),
            extent: extent.extent().into(),
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const SAMPLE_EXTENT: &str = r#"
/** `isobmff::sample::SampleExtent` */
export interface SampleExtent {
    properties: SampleProperties;
    dataReferenceIndex: number;
    extent: Range;
}
"#;
