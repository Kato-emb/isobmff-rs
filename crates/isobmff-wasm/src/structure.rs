//! [`isobmff::structure`]: [`MovieSampleExtentFsm`] and the [`WantedInput`] it states

use isobmff::structure;
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::boxes::MovieBox;
use crate::sample::SampleExtent;
use crate::{Failure, to_plain};

#[wasm_bindgen]
extern "C" {
    /// [`isobmff::sample::SampleExtent`] as the plain object `SampleExtent`
    #[wasm_bindgen(typescript_type = "SampleExtent")]
    pub type JsSampleExtent;

    /// [`isobmff::structure::WantedInput`] as the plain object `WantedInput`
    #[wasm_bindgen(typescript_type = "WantedInput")]
    pub type JsWantedInput;
}

/// [`isobmff::structure::WantedInput`]
#[derive(Debug, Clone, Copy, Serialize)]
struct WantedInput {
    /// [`isobmff::structure::WantedInput::offset`]
    offset: u64,
    /// [`isobmff::structure::WantedInput::length`]
    length: Option<u64>,
}

impl From<structure::WantedInput> for WantedInput {
    fn from(wanted: structure::WantedInput) -> Self {
        Self {
            offset: wanted.offset(),
            length: wanted.length(),
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const WANTED_INPUT: &str = r#"
/** `isobmff::structure::WantedInput` */
export interface WantedInput {
    offset: bigint;
    length: bigint | undefined;
}
"#;

/// [`isobmff::structure::MovieSampleExtentFsm`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MovieSampleExtentFsm(structure::MovieSampleExtentFsm);

#[wasm_bindgen]
impl MovieSampleExtentFsm {
    /// [`isobmff::structure::MovieSampleExtentFsm::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(structure::MovieSampleExtentFsm::new())
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::handle_input`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::MovieSampleExtentFsm::handle_input`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleInput)]
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), Failure> {
        Ok(self.0.handle_input(offset, input)?)
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::poll_sample_extent`]
    ///
    /// # Errors
    ///
    /// `"binding"`: the value did not convert to a plain object.
    #[wasm_bindgen(js_name = pollSampleExtent)]
    pub fn poll_sample_extent(&mut self) -> Result<Option<JsSampleExtent>, Failure> {
        self.0
            .poll_sample_extent()
            .map(|extent| to_plain(&SampleExtent::from(extent)))
            .transpose()
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::wanted_input`]
    ///
    /// # Errors
    ///
    /// `"binding"`: the value did not convert to a plain object.
    #[wasm_bindgen(js_name = wantedInput)]
    pub fn wanted_input(&self) -> Result<Option<JsWantedInput>, Failure> {
        self.0
            .wanted_input()
            .map(|wanted| to_plain(&WantedInput::from(wanted)))
            .transpose()
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::movie`], copied into a root of its own
    pub fn movie(&self) -> Option<MovieBox> {
        self.0.movie().cloned().map(MovieBox::new)
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::MovieSampleExtentFsm::finish`], of the
    /// kind of the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for MovieSampleExtentFsm {
    fn default() -> Self {
        Self::new()
    }
}
