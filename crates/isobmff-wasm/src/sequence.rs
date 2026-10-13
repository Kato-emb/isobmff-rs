//! [`isobmff::sequence`]: [`BoxReader`] and the [`BoxEvent`]s it reports

use isobmff::sequence;
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::framing::BoxHeader;
use crate::{Failure, Range, to_plain};

#[wasm_bindgen]
extern "C" {
    /// [`isobmff::sequence::BoxEvent`] and its extent as a plain array of a `Range` and a `BoxEvent`
    #[wasm_bindgen(typescript_type = "[Range, BoxEvent]")]
    pub type JsPolledEvent;
}

/// [`isobmff::sequence::BoxEvent`]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum BoxEvent {
    /// [`isobmff::sequence::BoxEvent::Header`]
    Header {
        /// The header read
        header: BoxHeader,
    },
    /// [`isobmff::sequence::BoxEvent::Payload`]
    Payload {
        /// The bytes of the payload read
        #[serde(with = "serde_bytes")]
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

#[wasm_bindgen(typescript_custom_section)]
const BOX_EVENT: &str = r#"
/** `isobmff::sequence::BoxEvent` */
export type BoxEvent = { type: "header"; header: BoxHeader } | { type: "payload"; payload: Uint8Array } | { type: "end" };
"#;

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
    #[wasm_bindgen(js_name = pollEvent)]
    pub fn poll_event(&mut self) -> Result<Option<JsPolledEvent>, Failure> {
        self.0
            .poll_event()
            .map(|(extent, event)| to_plain(&(Range::from(extent), BoxEvent::from(event))))
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
