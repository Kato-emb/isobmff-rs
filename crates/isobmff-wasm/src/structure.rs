//! The demuxers [`MovieExtentDemuxer`] and [`MovieDemuxer`] with the [`WantedInput`] they state, and the muxers [`NonFragmentedMuxer`] and [`FragmentedMuxer`]

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

    /// [`isobmff::WantedInput`] as the plain object `WantedInput`
    #[wasm_bindgen(typescript_type = "WantedInput")]
    pub type JsWantedInput;
}

/// [`isobmff::WantedInput`]
#[derive(Debug, Clone, Copy, Serialize)]
struct WantedInput {
    /// [`isobmff::WantedInput::offset`]
    offset: u64,
    /// [`isobmff::WantedInput::length`]
    length: Option<u64>,
}

impl From<isobmff::WantedInput> for WantedInput {
    fn from(wanted: isobmff::WantedInput) -> Self {
        Self {
            offset: wanted.offset(),
            length: wanted.length(),
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const WANTED_INPUT: &str = r#"
/** `isobmff::WantedInput` */
export interface WantedInput {
    offset: bigint;
    length: bigint | undefined;
}
"#;

/// [`isobmff::MovieDemuxer`] gathering with [`isobmff::SampleExtents`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MovieExtentDemuxer(isobmff::MovieDemuxer<isobmff::SampleExtents>);

#[wasm_bindgen]
impl MovieExtentDemuxer {
    /// [`isobmff::MovieDemuxer::with_gathering`], of [`isobmff::SampleExtents::new`] and [`isobmff::DemuxLimits::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(isobmff::MovieDemuxer::with_gathering(
            isobmff::SampleExtents::new(),
            isobmff::DemuxLimits::new(),
        ))
    }

    /// [`isobmff::MovieDemuxer::handle_input`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::MovieDemuxer::handle_input`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleInput)]
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), Failure> {
        Ok(self.0.handle_input(offset, input)?)
    }

    /// [`isobmff::MovieDemuxer::poll_output`]
    ///
    /// # Errors
    ///
    /// `"binding"`: the value did not convert to a plain object.
    #[wasm_bindgen(js_name = pollOutput)]
    pub fn poll_output(&mut self) -> Result<Option<JsSampleExtent>, Failure> {
        self.0
            .poll_output()
            .map(|extent| to_plain(&SampleExtent::from(extent)))
            .transpose()
    }

    /// [`isobmff::MovieDemuxer::wanted_input`]
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

    /// [`isobmff::MovieDemuxer::movie`], copied into a root of its own
    pub fn movie(&self) -> Option<MovieBox> {
        self.0.movie().cloned().map(MovieBox::new_root)
    }

    /// [`isobmff::MovieDemuxer::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::MovieDemuxer::finish`], of the
    /// kind of the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for MovieExtentDemuxer {
    fn default() -> Self {
        Self::new()
    }
}

/// [`isobmff::MovieDemuxer`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct MovieDemuxer(isobmff::MovieDemuxer);

#[wasm_bindgen]
impl MovieDemuxer {
    /// [`isobmff::MovieDemuxer::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(isobmff::MovieDemuxer::new())
    }

    /// [`isobmff::MovieDemuxer::handle_input`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::MovieDemuxer::handle_input`], of the
    /// kind of the layer that refused.
    #[wasm_bindgen(js_name = handleInput)]
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), Failure> {
        Ok(self.0.handle_input(offset, input)?)
    }

    /// [`isobmff::MovieDemuxer::poll_output`]
    #[wasm_bindgen(js_name = pollOutput)]
    pub fn poll_output(&mut self) -> Option<Sample> {
        self.0.poll_output().map(Sample)
    }

    /// [`isobmff::MovieDemuxer::wanted_input`]
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

    /// [`isobmff::MovieDemuxer::file_type`], copied into a root of its own
    #[wasm_bindgen(js_name = fileType)]
    pub fn file_type(&self) -> Option<FileTypeBox> {
        self.0.file_type().cloned().map(FileTypeBox)
    }

    /// [`isobmff::MovieDemuxer::movie`], copied into a root of its own
    pub fn movie(&self) -> Option<MovieBox> {
        self.0.movie().cloned().map(MovieBox::new_root)
    }

    /// [`isobmff::MovieDemuxer::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::MovieDemuxer::finish`], of the kind of
    /// the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for MovieDemuxer {
    fn default() -> Self {
        Self::new()
    }
}

/// [`isobmff::NonFragmentedMuxer`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct NonFragmentedMuxer(isobmff::NonFragmentedMuxer);

#[wasm_bindgen]
impl NonFragmentedMuxer {
    /// [`isobmff::NonFragmentedMuxer::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(isobmff::NonFragmentedMuxer::new())
    }

    /// [`isobmff::NonFragmentedMuxer::handle_file_type`], of a copy of `file_type`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::NonFragmentedMuxer::handle_file_type`],
    /// of the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleFileType)]
    pub fn handle_file_type(&mut self, file_type: &FileTypeBox) -> Result<(), Failure> {
        Ok(self.0.handle_file_type(file_type.0.clone())?)
    }

    /// [`isobmff::NonFragmentedMuxer::handle_movie`], of a copy of `movie`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::NonFragmentedMuxer::handle_movie`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleMovie)]
    pub fn handle_movie(&mut self, movie: &MovieBox) -> Result<(), Failure> {
        Ok(self.0.handle_movie(movie.value())?)
    }

    /// [`isobmff::NonFragmentedMuxer::begin_chunk`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::NonFragmentedMuxer::begin_chunk`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = beginChunk)]
    pub fn begin_chunk(&mut self) -> Result<(), Failure> {
        Ok(self.0.begin_chunk()?)
    }

    /// [`isobmff::NonFragmentedMuxer::handle_sample`], taking `sample`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::NonFragmentedMuxer::handle_sample`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleSample)]
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Failure> {
        Ok(self.0.handle_sample(sample.0)?)
    }

    /// [`isobmff::NonFragmentedMuxer::finish_chunk`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::NonFragmentedMuxer::finish_chunk`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = finishChunk)]
    pub fn finish_chunk(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish_chunk()?)
    }

    /// [`isobmff::NonFragmentedMuxer::poll_output`]
    #[wasm_bindgen(js_name = pollOutput)]
    pub fn poll_output(&mut self) -> Option<Uint8Array> {
        self.0.poll_output().map(|bytes| Uint8Array::from(&*bytes))
    }

    /// [`isobmff::NonFragmentedMuxer::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::NonFragmentedMuxer::finish`], of the
    /// kind of the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for NonFragmentedMuxer {
    fn default() -> Self {
        Self::new()
    }
}

/// [`isobmff::FragmentedMuxer`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct FragmentedMuxer(isobmff::FragmentedMuxer);

#[wasm_bindgen]
impl FragmentedMuxer {
    /// [`isobmff::FragmentedMuxer::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(isobmff::FragmentedMuxer::new())
    }

    /// [`isobmff::FragmentedMuxer::handle_file_type`], of a copy of `file_type`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::FragmentedMuxer::handle_file_type`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = handleFileType)]
    pub fn handle_file_type(&mut self, file_type: &FileTypeBox) -> Result<(), Failure> {
        Ok(self.0.handle_file_type(file_type.0.clone())?)
    }

    /// [`isobmff::FragmentedMuxer::handle_movie`], of a copy of `movie`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::FragmentedMuxer::handle_movie`], of the
    /// kind of the layer that refused.
    #[wasm_bindgen(js_name = handleMovie)]
    pub fn handle_movie(&mut self, movie: &MovieBox) -> Result<(), Failure> {
        Ok(self.0.handle_movie(movie.value())?)
    }

    /// [`isobmff::FragmentedMuxer::begin_fragment`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::FragmentedMuxer::begin_fragment`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = beginFragment)]
    pub fn begin_fragment(&mut self, sequence_number: u32) -> Result<(), Failure> {
        Ok(self.0.begin_fragment(sequence_number)?)
    }

    /// [`isobmff::FragmentedMuxer::handle_sample`], taking `sample`
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::FragmentedMuxer::handle_sample`], of the
    /// kind of the layer that refused.
    #[wasm_bindgen(js_name = handleSample)]
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Failure> {
        Ok(self.0.handle_sample(sample.0)?)
    }

    /// [`isobmff::FragmentedMuxer::finish_fragment`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::FragmentedMuxer::finish_fragment`], of
    /// the kind of the layer that refused.
    #[wasm_bindgen(js_name = finishFragment)]
    pub fn finish_fragment(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish_fragment()?)
    }

    /// [`isobmff::FragmentedMuxer::poll_output`]
    #[wasm_bindgen(js_name = pollOutput)]
    pub fn poll_output(&mut self) -> Option<Uint8Array> {
        self.0.poll_output().map(|bytes| Uint8Array::from(&*bytes))
    }

    /// [`isobmff::FragmentedMuxer::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::FragmentedMuxer::finish`], of the kind
    /// of the layer that refused.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for FragmentedMuxer {
    fn default() -> Self {
        Self::new()
    }
}
