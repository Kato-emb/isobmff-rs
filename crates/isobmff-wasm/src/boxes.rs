//! [`isobmff::boxes`]: [`CONTAINER_BOXES`](isobmff::boxes::CONTAINER_BOXES)

use wasm_bindgen::prelude::wasm_bindgen;

use crate::framing::BoxType;

/// [`isobmff::boxes::CONTAINER_BOXES`], called as a function
#[wasm_bindgen(js_name = CONTAINER_BOXES)]
pub fn container_boxes() -> Vec<BoxType> {
    isobmff::boxes::CONTAINER_BOXES
        .iter()
        .copied()
        .map(BoxType)
        .collect()
}
