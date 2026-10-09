//! PROTOTYPE — throwaway. A 1:1 JS binding of the library pieces a remux to a
//! non-fragmented file needs, to measure the binding's size and the cost of
//! driving the FSMs from JavaScript. Not production code.

use std::io::Cursor;

use isobmff::Mp4EpochSeconds;
use isobmff::boxes::{
    FileTypeBox, MediaBox, MediaHeaderBox, MovieBox, MovieHeaderBox, TrackBox, TrackHeaderBox,
};
use isobmff::sample::Sample;
use isobmff::structure::{MovieDemuxFsm, NonFragmentedMuxFsm};
use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;

fn js_error(error: impl core::fmt::Display) -> JsError {
    JsError::new(&error.to_string())
}

// ---- demux -----------------------------------------------------------------

#[wasm_bindgen(js_name = MovieDemuxFsm)]
pub struct JsMovieDemuxFsm(MovieDemuxFsm);

#[wasm_bindgen(js_class = MovieDemuxFsm)]
impl JsMovieDemuxFsm {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(MovieDemuxFsm::new())
    }
    /// The offset of the one read wanted next, or undefined when nothing is wanted
    pub fn wanted_offset(&self) -> Option<u64> {
        self.0.wanted_input().map(|wanted| wanted.offset())
    }
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), JsError> {
        self.0.handle_input(offset, input).map_err(js_error)
    }
    pub fn finish(&mut self) -> Result<(), JsError> {
        self.0.finish().map_err(js_error)
    }
    pub fn poll_sample(&mut self) -> Option<JsSample> {
        self.0.poll_sample().map(JsSample)
    }
    pub fn movie(&self) -> Option<JsMovieBox> {
        self.0.movie().cloned().map(JsMovieBox)
    }
    pub fn file_type(&self) -> Option<JsFileTypeBox> {
        self.0.file_type().cloned().map(JsFileTypeBox)
    }
}

#[wasm_bindgen(js_name = Sample)]
pub struct JsSample(Sample);

#[wasm_bindgen(js_class = Sample)]
impl JsSample {
    #[wasm_bindgen(getter)]
    pub fn track_id(&self) -> u32 {
        self.0.properties().track_id
    }
    #[wasm_bindgen(getter)]
    pub fn decode_time(&self) -> u64 {
        self.0.properties().decode_time
    }
    #[wasm_bindgen(getter)]
    pub fn sample_duration(&self) -> u32 {
        self.0.properties().sample_duration
    }
    #[wasm_bindgen(getter)]
    pub fn sample_composition_time_offset(&self) -> i64 {
        self.0.properties().sample_composition_time_offset
    }
    #[wasm_bindgen(getter)]
    pub fn sample_description_index(&self) -> u32 {
        self.0.properties().sample_description_index
    }
    #[wasm_bindgen(getter)]
    pub fn sync(&self) -> bool {
        !self.0.properties().sample_flags.sample_is_non_sync_sample()
    }
    /// A copy of the sample's bytes
    pub fn data(&self) -> Vec<u8> {
        self.0.data().to_vec()
    }
}

// ---- boxes: just what the non-fragmented remux edits -------------------------

#[wasm_bindgen(js_name = FileTypeBox)]
pub struct JsFileTypeBox(FileTypeBox);

#[wasm_bindgen(js_name = MovieBox)]
pub struct JsMovieBox(MovieBox);

#[wasm_bindgen(js_class = MovieBox)]
impl JsMovieBox {
    /// A movie of `mvhd` and `trak`, carrying no `mvex`
    pub fn create(mvhd: &JsMovieHeaderBox, trak: Vec<JsTrackBox>) -> Result<JsMovieBox, JsError> {
        MovieBox::new(mvhd.0.clone(), trak.into_iter().map(|track| track.0).collect(), None)
            .map(JsMovieBox)
            .ok_or_else(|| JsError::new("the movie declares no track, or two of one track_ID"))
    }
    pub fn mvhd(&self) -> JsMovieHeaderBox {
        JsMovieHeaderBox(self.0.mvhd().clone())
    }
    pub fn trak(&self) -> Vec<JsTrackBox> {
        self.0.trak().iter().cloned().map(JsTrackBox).collect()
    }
}

#[wasm_bindgen(js_name = MovieHeaderBox)]
pub struct JsMovieHeaderBox(MovieHeaderBox);

#[wasm_bindgen(js_class = MovieHeaderBox)]
impl JsMovieHeaderBox {
    #[wasm_bindgen(getter)]
    pub fn timescale(&self) -> u32 {
        self.0.timescale()
    }
    pub fn with_modification_time(&self, seconds: u64) -> JsMovieHeaderBox {
        JsMovieHeaderBox(self.0.clone().with_modification_time(Mp4EpochSeconds::from_seconds(seconds)))
    }
}

#[wasm_bindgen(js_name = TrackBox)]
pub struct JsTrackBox(TrackBox);

#[wasm_bindgen(js_class = TrackBox)]
impl JsTrackBox {
    pub fn tkhd(&self) -> JsTrackHeaderBox {
        JsTrackHeaderBox(self.0.tkhd().clone())
    }
    pub fn set_tkhd(&mut self, tkhd: &JsTrackHeaderBox) {
        *self.0.tkhd_mut() = tkhd.0.clone();
    }
    pub fn mdia(&self) -> JsMediaBox {
        JsMediaBox(self.0.mdia().clone())
    }
    pub fn set_mdia(&mut self, mdia: &JsMediaBox) {
        *self.0.mdia_mut() = mdia.0.clone();
    }
}

#[wasm_bindgen(js_name = TrackHeaderBox)]
pub struct JsTrackHeaderBox(TrackHeaderBox);

#[wasm_bindgen(js_class = TrackHeaderBox)]
impl JsTrackHeaderBox {
    #[wasm_bindgen(getter)]
    pub fn track_id(&self) -> u32 {
        self.0.track_id()
    }
    pub fn with_modification_time(&self, seconds: u64) -> JsTrackHeaderBox {
        JsTrackHeaderBox(self.0.clone().with_modification_time(Mp4EpochSeconds::from_seconds(seconds)))
    }
}

#[wasm_bindgen(js_name = MediaBox)]
pub struct JsMediaBox(MediaBox);

#[wasm_bindgen(js_class = MediaBox)]
impl JsMediaBox {
    pub fn mdhd(&self) -> JsMediaHeaderBox {
        JsMediaHeaderBox(self.0.mdhd().clone())
    }
    pub fn set_mdhd(&mut self, mdhd: &JsMediaHeaderBox) {
        *self.0.mdhd_mut() = mdhd.0.clone();
    }
}

#[wasm_bindgen(js_name = MediaHeaderBox)]
pub struct JsMediaHeaderBox(MediaHeaderBox);

#[wasm_bindgen(js_class = MediaHeaderBox)]
impl JsMediaHeaderBox {
    pub fn with_modification_time(&self, seconds: u64) -> JsMediaHeaderBox {
        JsMediaHeaderBox(self.0.clone().with_modification_time(Mp4EpochSeconds::from_seconds(seconds)))
    }
}

// ---- mux -------------------------------------------------------------------

#[wasm_bindgen(js_name = NonFragmentedMuxFsm)]
pub struct JsNonFragmentedMuxFsm(NonFragmentedMuxFsm);

#[wasm_bindgen(js_class = NonFragmentedMuxFsm)]
impl JsNonFragmentedMuxFsm {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(NonFragmentedMuxFsm::new())
    }
    pub fn handle_file_type(&mut self, file_type: &JsFileTypeBox) -> Result<(), JsError> {
        self.0.handle_file_type(file_type.0.clone()).map_err(js_error)
    }
    pub fn handle_movie(&mut self, movie: JsMovieBox) -> Result<(), JsError> {
        self.0.handle_movie(movie.0).map_err(js_error)
    }
    pub fn begin_chunk(&mut self) -> Result<(), JsError> {
        self.0.begin_chunk().map_err(js_error)
    }
    /// Takes the sample: the JS handle is spent
    pub fn handle_sample(&mut self, sample: JsSample) -> Result<(), JsError> {
        self.0.handle_sample(sample.0).map_err(js_error)
    }
    pub fn finish_chunk(&mut self) -> Result<(), JsError> {
        self.0.finish_chunk().map_err(js_error)
    }
    pub fn poll_output(&mut self) -> Option<Vec<u8>> {
        self.0.poll_output().map(|bytes| bytes.to_vec())
    }
    pub fn finish(&mut self) -> Result<(), JsError> {
        self.0.finish().map_err(js_error)
    }
}

// ---- the Rust remux, for comparison: same algorithm, output kept in memory ----

/// The current Rust remux to a non-fragmented file, over the whole file handed in as bytes, with a fixed `now`
#[wasm_bindgen]
pub fn prototype_rust_remux_to_non_fragmented(file: &[u8], now_seconds: u64) -> Result<Vec<u8>, JsError> {
    let mut written = Vec::new();
    crate::remux::remux_to_non_fragmented(
        Cursor::new(file),
        &mut written,
        Mp4EpochSeconds::from_seconds(now_seconds),
    )
    .map_err(js_error)?;
    Ok(written)
}

/// The current Rust remux over the Blob through FileReaderSync, output kept in memory
#[wasm_bindgen]
pub fn prototype_rust_remux_blob(blob: web_sys::Blob, now_seconds: u64) -> Result<Vec<u8>, JsError> {
    let mut written = Vec::new();
    crate::remux::remux_to_non_fragmented(
        crate::source::BlobSource::new(blob).map_err(js_error)?,
        &mut written,
        Mp4EpochSeconds::from_seconds(now_seconds),
    )
    .map_err(js_error)?;
    Ok(written)
}
