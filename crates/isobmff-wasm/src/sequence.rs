//! [`isobmff::sequence`]: [`BoxReader`] and the [`BoxEvent`]s it reports

use isobmff::sequence;
use js_sys::Array;
use serde::Serialize;
use tsify::Tsify;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::framing::BoxHeader;
use crate::{Failure, Range};

/// [`isobmff::sequence::BoxEvent`]
#[derive(Debug, Clone, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum BoxEvent {
    /// [`isobmff::sequence::BoxEvent::Header`]
    #[non_exhaustive]
    Header {
        /// The header read
        header: BoxHeader,
    },
    /// [`isobmff::sequence::BoxEvent::Payload`]
    #[non_exhaustive]
    Payload {
        /// The bytes of the payload read
        #[serde(with = "serde_bytes")]
        #[tsify(type = "Uint8Array")]
        payload: Vec<u8>,
    },
    /// [`isobmff::sequence::BoxEvent::End`]
    End,
}

impl From<sequence::BoxEvent> for BoxEvent {
    fn from(event: sequence::BoxEvent) -> Self {
        match event {
            sequence::BoxEvent::Header(header) => Self::Header {
                header: header.into(),
            },
            sequence::BoxEvent::Payload(payload) => Self::Payload { payload },
            sequence::BoxEvent::End => Self::End,
        }
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
    /// Those of [`isobmff::sequence::BoxReader::handle_input`], of kind
    /// `"sequence"`, or `"box"` for a header that does not decode.
    #[wasm_bindgen(js_name = handleInput)]
    pub fn handle_input(&mut self, input: &[u8]) -> Result<(), Failure> {
        Ok(self.0.handle_input(input)?)
    }

    /// [`isobmff::sequence::BoxReader::poll_event`], the extent and the event as an array of two
    ///
    /// # Errors
    ///
    /// `"binding"`: the value did not convert to a plain object.
    #[wasm_bindgen(js_name = pollEvent, unchecked_return_type = "[Range, BoxEvent] | undefined")]
    pub fn poll_event(&mut self) -> Result<Option<Array>, Failure> {
        self.0
            .poll_event()
            .map(|(extent, event)| {
                Ok(Array::of2(
                    &JsValue::from(Range::from(extent).into_ts()?),
                    &JsValue::from(BoxEvent::from(event).into_ts()?),
                ))
            })
            .transpose()
    }

    /// [`isobmff::sequence::BoxReader::finish`]
    ///
    /// # Errors
    ///
    /// Those of [`isobmff::sequence::BoxReader::finish`], of kind
    /// `"sequence"`.
    pub fn finish(&mut self) -> Result<(), Failure> {
        Ok(self.0.finish()?)
    }
}

impl Default for BoxReader {
    fn default() -> Self {
        Self::new()
    }
}
