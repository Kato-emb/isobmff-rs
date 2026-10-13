//! [`isobmff::sample`]: [`SampleExtent`] and the [`SampleProperties`] it carries

use isobmff::sample;
use serde::Serialize;
use tsify::Tsify;

use crate::Range;
use crate::boxes::SampleFlags;

/// [`isobmff::sample::SampleProperties`]
#[derive(Debug, Clone, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SampleProperties {
    /// [`isobmff::sample::SampleProperties::track_id`]
    pub track_id: u32,
    /// [`isobmff::sample::SampleProperties::decode_time`]
    pub decode_time: u64,
    /// [`isobmff::sample::SampleProperties::sample_duration`]
    pub sample_duration: u32,
    /// [`isobmff::sample::SampleProperties::sample_composition_time_offset`]
    pub sample_composition_time_offset: i64,
    /// [`isobmff::sample::SampleProperties::sample_flags`]
    pub sample_flags: SampleFlags,
    /// [`isobmff::sample::SampleProperties::sample_description_index`]
    pub sample_description_index: u32,
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

/// [`isobmff::sample::SampleExtent`]
#[derive(Debug, Clone, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SampleExtent {
    /// [`isobmff::sample::SampleExtent::properties`]
    pub properties: SampleProperties,
    /// [`isobmff::sample::SampleExtent::data_reference_index`]
    pub data_reference_index: u16,
    /// [`isobmff::sample::SampleExtent::extent`]
    pub extent: Range,
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
