//! [`isobmff::structure`]: [`MovieSampleExtentFsm`]

use isobmff::structure;
use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::boxes::MovieBox;
use crate::sample::SampleExtent;

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
    /// Those of [`isobmff::structure::MovieSampleExtentFsm::handle_input`].
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), JsError> {
        Ok(self.0.handle_input(offset, input)?)
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::poll_sample_extent`]
    pub fn poll_sample_extent(&mut self) -> Option<SampleExtent> {
        self.0.poll_sample_extent().map(SampleExtent)
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::movie`], copied
    pub fn movie(&self) -> Option<MovieBox> {
        self.0.movie().cloned().map(MovieBox)
    }

    /// [`isobmff::structure::MovieSampleExtentFsm::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::MovieSampleExtentFsm::finish`].
    pub fn finish(&mut self) -> Result<(), JsError> {
        Ok(self.0.finish()?)
    }
}

impl Default for MovieSampleExtentFsm {
    fn default() -> Self {
        Self::new()
    }
}
