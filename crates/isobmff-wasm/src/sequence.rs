//! [`isobmff::sequence`]: [`BoxReader`] and the [`BoxEvent`]s it reports

use isobmff::sequence;
use js_sys::Array;
use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::Range;
use crate::framing::BoxHeader;

/// [`isobmff::sequence::BoxEvent`], one method for each variant
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct BoxEvent(sequence::BoxEvent);

#[wasm_bindgen]
impl BoxEvent {
    /// The header of [`Header`](sequence::BoxEvent::Header), or none for another variant
    pub fn header(&self) -> Option<BoxHeader> {
        match &self.0 {
            sequence::BoxEvent::Header(header) => Some(BoxHeader(*header)),
            sequence::BoxEvent::Payload(_) | sequence::BoxEvent::End => None,
        }
    }

    /// A copy of the bytes of [`Payload`](sequence::BoxEvent::Payload), or none for another variant
    pub fn payload(&self) -> Option<Vec<u8>> {
        match &self.0 {
            sequence::BoxEvent::Payload(payload) => Some(payload.clone()),
            sequence::BoxEvent::Header(_) | sequence::BoxEvent::End => None,
        }
    }

    /// Whether the event is [`End`](sequence::BoxEvent::End)
    pub fn end(&self) -> bool {
        matches!(self.0, sequence::BoxEvent::End)
    }
}

/// [`isobmff::sequence::BoxReader`]
#[wasm_bindgen]
#[derive(Debug)]
pub struct BoxReader(sequence::BoxReader);

#[wasm_bindgen]
impl BoxReader {
    /// [`isobmff::sequence::BoxReader::new`]
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self(sequence::BoxReader::new())
    }

    /// [`isobmff::sequence::BoxReader::handle_input`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::sequence::BoxReader::handle_input`].
    pub fn handle_input(&mut self, input: &[u8]) -> Result<(), JsError> {
        Ok(self.0.handle_input(input)?)
    }

    /// [`isobmff::sequence::BoxReader::poll_event`], the extent and the event as an array of two
    pub fn poll_event(&mut self) -> Option<Array> {
        self.0
            .poll_event()
            .map(|(extent, event)| Array::of2(&Range::from(extent), &BoxEvent(event).into()))
    }

    /// [`isobmff::sequence::BoxReader::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::sequence::BoxReader::finish`].
    pub fn finish(&mut self) -> Result<(), JsError> {
        Ok(self.0.finish()?)
    }
}

impl Default for BoxReader {
    fn default() -> Self {
        Self::new()
    }
}
