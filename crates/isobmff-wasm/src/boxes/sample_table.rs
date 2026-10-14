//! The `stbl` of ISO/IEC 14496-12 §8.5.1, a handle or a root of its own, its `stsd` (§8.5.2) and the entries of that `stsd` as handles, the `stss` (§8.6.2) it may hold, and the tables of §8.6.1.2 to §8.7.5 a `stbl` is built from, each a root of its own

use isobmff::boxes;
use wasm_bindgen::prelude::wasm_bindgen;

use super::Place;
use crate::framing::{BoxType, JsBoxType};
use crate::{Failure, to_plain};

#[wasm_bindgen]
extern "C" {
    /// The chunks [`isobmff::boxes::SampleToChunkBox::from_chunks`] takes, as a plain array of pairs
    #[wasm_bindgen(typescript_type = "[bigint, number][]")]
    pub type JsSampleChunks;
}

/// [`isobmff::boxes::SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1, a handle or a root of its own
#[doc(alias = "stbl")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct SampleTableBox(pub(super) Place<boxes::SampleTableBox>);

#[wasm_bindgen]
impl SampleTableBox {
    /// [`isobmff::boxes::SampleTableBox::new`] from copies of the tables, a root of its own
    ///
    /// # Errors
    ///
    /// `"handle"`: the path of `stsd` leads to no box.
    pub fn new(
        stsd: &SampleDescriptionBox,
        stts: &TimeToSampleBox,
        stsc: &SampleToChunkBox,
        sample_sizes: &SampleSizes,
        chunk_offsets: &ChunkOffsets,
    ) -> Result<Self, Failure> {
        let stsd = stsd.0.read(|table| table.stsd().clone())?;
        Ok(Self(Place::root(boxes::SampleTableBox::new(
            stsd,
            stts.0.clone(),
            stsc.0.clone(),
            sample_sizes.0.clone(),
            chunk_offsets.0.clone(),
        ))))
    }

    /// [`isobmff::boxes::SampleTableBox::stsd`], a handle
    pub fn stsd(&self) -> SampleDescriptionBox {
        SampleDescriptionBox(self.0.clone())
    }

    /// [`isobmff::boxes::SampleTableBox::stss`], or none where the table holds no `stss`
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    pub fn stss(&self) -> Result<Option<SyncSampleBox>, Failure> {
        Ok(self
            .0
            .read(|table| table.stss().is_some())?
            .then_some(SyncSampleBox {}))
    }
}

/// [`isobmff::boxes::SampleDescriptionBox`] (`stsd`), ISO/IEC 14496-12 §8.5.2, a handle into its root
#[doc(alias = "stsd")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct SampleDescriptionBox(Place<boxes::SampleTableBox>);

#[wasm_bindgen]
impl SampleDescriptionBox {
    /// [`isobmff::boxes::SampleDescriptionBox::entries`], a handle to each
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box.
    pub fn entries(&self) -> Result<Vec<AnyBox>, Failure> {
        let count = self.0.read(|table| table.stsd().entries().len())?;
        Ok((0..count)
            .map(|entry| AnyBox {
                table: self.0.clone(),
                entry,
            })
            .collect())
    }
}

/// [`isobmff::AnyBox`], an entry of a `stsd`, a handle into its root
#[wasm_bindgen]
#[derive(Debug)]
pub struct AnyBox {
    /// The `stbl` whose `stsd` holds the entry
    table: Place<boxes::SampleTableBox>,
    /// The position of the entry in the `stsd`
    entry: usize,
}

#[wasm_bindgen]
impl AnyBox {
    /// [`isobmff::AnyBox::box_type`]
    ///
    /// # Errors
    ///
    /// `"handle"`: the path leads to no box; `"binding"`: the value did not
    /// convert to a plain object.
    #[wasm_bindgen(js_name = boxType)]
    pub fn box_type(&self) -> Result<JsBoxType, Failure> {
        let box_type = self
            .table
            .read(|table| {
                table
                    .stsd()
                    .entries()
                    .get(self.entry)
                    .map(isobmff::AnyBox::box_type)
            })?
            .ok_or_else(Failure::handle)?;
        to_plain(&BoxType::from(box_type))
    }
}

/// [`isobmff::boxes::SyncSampleBox`] (`stss`), ISO/IEC 14496-12 §8.6.2, the `stss` a `stbl` holds, of which nothing is read
#[doc(alias = "stss")]
#[wasm_bindgen]
#[derive(Debug)]
#[non_exhaustive]
pub struct SyncSampleBox {}

/// [`isobmff::boxes::TimeToSampleBox`] (`stts`), ISO/IEC 14496-12 §8.6.1.2, a root of its own
#[doc(alias = "stts")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct TimeToSampleBox(boxes::TimeToSampleBox);

#[wasm_bindgen]
impl TimeToSampleBox {
    /// [`isobmff::boxes::TimeToSampleBox::from_deltas`]
    #[wasm_bindgen(js_name = fromDeltas)]
    pub fn from_deltas(deltas: Vec<u32>) -> Self {
        Self(boxes::TimeToSampleBox::from_deltas(deltas))
    }
}

/// [`isobmff::boxes::SampleToChunkBox`] (`stsc`), ISO/IEC 14496-12 §8.7.4, a root of its own
#[doc(alias = "stsc")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct SampleToChunkBox(boxes::SampleToChunkBox);

#[wasm_bindgen]
impl SampleToChunkBox {
    /// [`isobmff::boxes::SampleToChunkBox::from_chunks`], each chunk the count of its samples and the index of their sample description
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::boxes::SampleToChunkBox::from_chunks`], of kind
    /// `"box"`; `"binding"`: `chunks` is not an array of such pairs.
    #[wasm_bindgen(js_name = fromChunks)]
    pub fn from_chunks(chunks: JsSampleChunks) -> Result<Self, Failure> {
        let chunks: Vec<(u64, u32)> = serde_wasm_bindgen::from_value(chunks.into())?;
        Ok(Self(boxes::SampleToChunkBox::from_chunks(chunks)?))
    }
}

/// [`isobmff::boxes::SampleSizeBox`] (`stsz`), ISO/IEC 14496-12 §8.7.3.2, a root of its own
#[doc(alias = "stsz")]
#[wasm_bindgen]
#[derive(Debug)]
pub struct SampleSizeBox(boxes::SampleSizeBox);

#[wasm_bindgen]
impl SampleSizeBox {
    /// [`isobmff::boxes::SampleSizeBox::from_sizes`]
    #[wasm_bindgen(js_name = fromSizes)]
    pub fn from_sizes(sizes: Vec<u32>) -> Self {
        Self(boxes::SampleSizeBox::from_sizes(sizes))
    }
}

/// [`isobmff::boxes::SampleSizes`], ISO/IEC 14496-12 §8.7.3, a root of its own
#[wasm_bindgen]
#[derive(Debug)]
pub struct SampleSizes(boxes::SampleSizes);

#[wasm_bindgen]
impl SampleSizes {
    /// [`isobmff::boxes::SampleSizes::Stsz`] of a copy of `stsz`
    pub fn stsz(stsz: &SampleSizeBox) -> Self {
        Self(boxes::SampleSizes::Stsz(stsz.0.clone()))
    }
}

/// [`isobmff::boxes::ChunkOffsets`], ISO/IEC 14496-12 §8.7.5, a root of its own
#[wasm_bindgen]
#[derive(Debug)]
pub struct ChunkOffsets(boxes::ChunkOffsets);

#[wasm_bindgen]
impl ChunkOffsets {
    /// [`isobmff::boxes::ChunkOffsets::from_offsets`]
    #[wasm_bindgen(js_name = fromOffsets)]
    pub fn from_offsets(offsets: Vec<u64>) -> Self {
        Self(boxes::ChunkOffsets::from_offsets(offsets))
    }
}
