//! The root of [`isobmff`]: the framing of ISO/IEC 14496-12 §4.2 ([`BoxType`], [`BoxSize`], the [`BoxHeader`] they form, the [`Boxes`] laid end to end), [`FourCc`] and [`AnyBox`]

use alloc::vec;

use wasm_bindgen::JsError;
use wasm_bindgen::prelude::wasm_bindgen;

/// [`isobmff::FourCc`]
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct FourCc(pub(crate) isobmff::FourCc);

#[wasm_bindgen]
impl FourCc {
    /// Returns the four characters, as [`isobmff::FourCc`] displays them
    #[wasm_bindgen(js_name = toString)]
    pub fn display(&self) -> String {
        self.0.to_string()
    }
}

/// [`isobmff::BoxType`]
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct BoxType(pub(crate) isobmff::BoxType);

#[wasm_bindgen]
impl BoxType {
    /// Returns the fourcc, or `uuid` and the user type, as [`isobmff::BoxType`] displays it
    #[wasm_bindgen(js_name = toString)]
    pub fn display(&self) -> String {
        self.0.to_string()
    }
}

/// [`isobmff::BoxSize`]
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct BoxSize(isobmff::BoxSize);

#[wasm_bindgen]
impl BoxSize {
    /// [`isobmff::BoxSize::total_bytes`]
    pub fn total_bytes(&self) -> Option<u64> {
        self.0.total_bytes()
    }
}

/// [`isobmff::BoxHeader`]
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct BoxHeader(pub(crate) isobmff::BoxHeader);

#[wasm_bindgen]
impl BoxHeader {
    /// [`isobmff::BoxHeader::box_type`]
    pub fn box_type(&self) -> BoxType {
        BoxType(self.0.box_type())
    }

    /// [`isobmff::BoxHeader::size`]
    pub fn size(&self) -> BoxSize {
        BoxSize(self.0.size())
    }

    /// [`isobmff::BoxHeader::encoded_len`]
    pub fn encoded_len(&self) -> usize {
        self.0.encoded_len()
    }
}

/// [`isobmff::RawBox`], owning a copy of its payload
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct RawBox {
    header: isobmff::BoxHeader,
    payload: Vec<u8>,
}

#[wasm_bindgen]
impl RawBox {
    /// [`isobmff::RawBox::header`]
    pub fn header(&self) -> BoxHeader {
        BoxHeader(self.header)
    }

    /// [`isobmff::RawBox::payload`], copied
    pub fn payload(&self) -> Vec<u8> {
        self.payload.clone()
    }
}

/// [`isobmff::Boxes`] over a copy of its input
#[wasm_bindgen]
#[derive(Debug)]
pub struct Boxes(vec::IntoIter<Result<RawBox, isobmff::Error>>);

#[wasm_bindgen]
impl Boxes {
    /// [`isobmff::Boxes::new`]
    #[wasm_bindgen(constructor)]
    pub fn new(input: &[u8]) -> Self {
        // Why not hold `isobmff::Boxes` itself: it borrows `input`, which a
        // value handed to JavaScript cannot, so its steps are taken here at once.
        Self(
            isobmff::Boxes::new(input)
                .map(|step| {
                    step.map(|framed| RawBox {
                        header: framed.header(),
                        payload: framed.payload().to_vec(),
                    })
                })
                .collect::<Vec<_>>()
                .into_iter(),
        )
    }

    /// [`Iterator::next`] of [`isobmff::Boxes`], the box yielded or none once they are used up
    ///
    /// # Errors
    ///
    /// The error [`isobmff::Boxes`] yields, after which every call returns none.
    #[wasm_bindgen(js_name = next)]
    pub fn next_box(&mut self) -> Result<Option<RawBox>, JsError> {
        Ok(self.0.next().transpose()?)
    }
}

/// [`isobmff::AnyBox`]
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct AnyBox(pub(crate) isobmff::AnyBox);

#[wasm_bindgen]
impl AnyBox {
    /// [`isobmff::AnyBox::box_type`]
    pub fn box_type(&self) -> BoxType {
        BoxType(self.0.box_type())
    }
}
