//! [`isobmff::structure`]: the demux FSMs [`MovieSampleExtentFsm`] and [`MovieDemuxFsm`] with the [`WantedInput`] they state, and the mux FSMs [`NonFragmentedMuxFsm`] and [`FragmentedMuxFsm`]

use isobmff::structure;
use js_sys::Uint8Array;
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::boxes::{FileTypeBox, MovieBox};
use crate::sample::{Sample, SampleExtent};
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
        self.0.movie().cloned().map(MovieBox::new_root)
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

/// [`isobmff::structure::MovieDemuxFsm`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MovieDemuxFsm(structure::MovieDemuxFsm);

#[wasm_bindgen]
impl MovieDemuxFsm {
    /// [`isobmff::structure::MovieDemuxFsm::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(structure::MovieDemuxFsm::new())
    }

    /// [`isobmff::structure::MovieDemuxFsm::handle_input`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::MovieDemuxFsm::handle_input`], of the
    /// kind of the layer that refused.
    #[wasm_bindgen(js_name = handleInput)]
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), Failure> {
        Ok(self.0.handle_input(offset, input)?)
    }

    /// [`isobmff::structure::MovieDemuxFsm::poll_sample`]
    #[wasm_bindgen(js_name = pollSample)]
    pub fn poll_sample(&mut self) -> Option<Sample> {
        self.0.poll_sample().map(Sample)
    }

    /// [`isobmff::structure::MovieDemuxFsm::wanted_input`]
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

    /// [`isobmff::structure::MovieDemuxFsm::file_type`], copied into a root of its own
    #[wasm_bindgen(js_name = fileType)]
    pub fn file_type(&self) -> Option<FileTypeBox> {
        self.0.file_type().cloned().map(FileTypeBox)
    }

    /// [`isobmff::structure::MovieDemuxFsm::movie`], copied into a root of its own
    pub fn movie(&self) -> Option<MovieBox> {
        self.0.movie().cloned().map(MovieBox::new_root)
    }

    /// [`isobmff::structure::MovieDemuxFsm::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::MovieDemuxFsm::finish`], of the kind of
    /// the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for MovieDemuxFsm {
    fn default() -> Self {
        Self::new()
    }
}

/// [`isobmff::structure::NonFragmentedMuxFsm`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct NonFragmentedMuxFsm(structure::NonFragmentedMuxFsm);

#[wasm_bindgen]
impl NonFragmentedMuxFsm {
    /// [`isobmff::structure::NonFragmentedMuxFsm::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(structure::NonFragmentedMuxFsm::new())
    }

    /// [`isobmff::structure::NonFragmentedMuxFsm::handle_file_type`], of a copy of `file_type`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::NonFragmentedMuxFsm::handle_file_type`],
    /// of the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleFileType)]
    pub fn handle_file_type(&mut self, file_type: &FileTypeBox) -> Result<(), Failure> {
        Ok(self.0.handle_file_type(file_type.0.clone())?)
    }

    /// [`isobmff::structure::NonFragmentedMuxFsm::handle_movie`], of a copy of `movie`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::NonFragmentedMuxFsm::handle_movie`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleMovie)]
    pub fn handle_movie(&mut self, movie: &MovieBox) -> Result<(), Failure> {
        Ok(self.0.handle_movie(movie.value())?)
    }

    /// [`isobmff::structure::NonFragmentedMuxFsm::begin_chunk`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::NonFragmentedMuxFsm::begin_chunk`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = beginChunk)]
    pub fn begin_chunk(&mut self) -> Result<(), Failure> {
        Ok(self.0.begin_chunk()?)
    }

    /// [`isobmff::structure::NonFragmentedMuxFsm::handle_sample`], taking `sample`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::NonFragmentedMuxFsm::handle_sample`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleSample)]
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Failure> {
        Ok(self.0.handle_sample(sample.0)?)
    }

    /// [`isobmff::structure::NonFragmentedMuxFsm::finish_chunk`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::NonFragmentedMuxFsm::finish_chunk`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = finishChunk)]
    pub fn finish_chunk(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish_chunk()?)
    }

    /// [`isobmff::structure::NonFragmentedMuxFsm::poll_output`]
    #[wasm_bindgen(js_name = pollOutput)]
    pub fn poll_output(&mut self) -> Option<Uint8Array> {
        self.0.poll_output().map(|bytes| Uint8Array::from(&*bytes))
    }

    /// [`isobmff::structure::NonFragmentedMuxFsm::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::NonFragmentedMuxFsm::finish`], of the
    /// kind of the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for NonFragmentedMuxFsm {
    fn default() -> Self {
        Self::new()
    }
}

/// [`isobmff::structure::FragmentedMuxFsm`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct FragmentedMuxFsm(structure::FragmentedMuxFsm);

#[wasm_bindgen]
impl FragmentedMuxFsm {
    /// [`isobmff::structure::FragmentedMuxFsm::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(structure::FragmentedMuxFsm::new())
    }

    /// [`isobmff::structure::FragmentedMuxFsm::handle_file_type`], of a copy of `file_type`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::FragmentedMuxFsm::handle_file_type`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleFileType)]
    pub fn handle_file_type(&mut self, file_type: &FileTypeBox) -> Result<(), Failure> {
        Ok(self.0.handle_file_type(file_type.0.clone())?)
    }

    /// [`isobmff::structure::FragmentedMuxFsm::handle_movie`], of a copy of `movie`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::FragmentedMuxFsm::handle_movie`], of the
    /// kind of the layer that refused.
    #[wasm_bindgen(js_name = handleMovie)]
    pub fn handle_movie(&mut self, movie: &MovieBox) -> Result<(), Failure> {
        Ok(self.0.handle_movie(movie.value())?)
    }

    /// [`isobmff::structure::FragmentedMuxFsm::begin_fragment`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::FragmentedMuxFsm::begin_fragment`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = beginFragment)]
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Failure> {
        Ok(self.0.begin_fragment(sequence_number)?)
    }

    /// [`isobmff::structure::FragmentedMuxFsm::handle_sample`], taking `sample`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::FragmentedMuxFsm::handle_sample`], of the
    /// kind of the layer that refused.
    #[wasm_bindgen(js_name = handleSample)]
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Failure> {
        Ok(self.0.handle_sample(sample.0)?)
    }

    /// [`isobmff::structure::FragmentedMuxFsm::finish_fragment`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::FragmentedMuxFsm::finish_fragment`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = finishFragment)]
    pub fn finish_fragment(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish_fragment()?)
    }

    /// [`isobmff::structure::FragmentedMuxFsm::poll_output`]
    #[wasm_bindgen(js_name = pollOutput)]
    pub fn poll_output(&mut self) -> Option<Uint8Array> {
        self.0.poll_output().map(|bytes| Uint8Array::from(&*bytes))
    }

    /// [`isobmff::structure::FragmentedMuxFsm::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::structure::FragmentedMuxFsm::finish`], of the kind
    /// of the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for FragmentedMuxFsm {
    fn default() -> Self {
        Self::new()
    }
}
