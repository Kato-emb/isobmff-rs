//! The root of [`isobmff`]: the framing of ISO/IEC 14496-12 §4.2 ([`BoxType`], [`BoxSize`], the [`BoxHeader`] they form, the [`Boxes`] laid end to end) and [`FourCc`]

use alloc::vec;

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::{Failure, to_plain};

#[wasm_bindgen]
extern "C" {
    /// [`isobmff::BoxType`] as the plain object `BoxType`
    #[wasm_bindgen(typescript_type = "BoxType")]
    pub type JsBoxType;

    /// One step of [`Boxes`] as the plain object `IteratorResult` the iterator protocol of JavaScript states
    #[wasm_bindgen(typescript_type = "IteratorResult<RawBox, undefined>")]
    pub type JsIteratorStep;
}

/// [`isobmff::FourCc`], the four characters as [`isobmff::FourCc`] displays them
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FourCc(String);

impl From<isobmff::FourCc> for FourCc {
    fn from(four_cc: isobmff::FourCc) -> Self {
        Self(four_cc.to_string())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const FOUR_CC: &str = r#"
/** `isobmff::FourCc`, the four characters as it displays them */
export type FourCc = string;
"#;

/// [`isobmff::BoxType`]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum BoxType {
    /// [`isobmff::BoxType::Compact`]
    #[serde(rename_all = "camelCase")]
    Compact {
        /// [`isobmff::CompactType::four_cc`]
        four_cc: FourCc,
    },
    /// [`isobmff::BoxType::Extended`]
    #[serde(rename_all = "camelCase")]
    Extended {
        /// The `usertype`, as [`isobmff::Uuid`] displays it
        user_type: String,
    },
}

impl From<isobmff::BoxType> for BoxType {
    fn from(box_type: isobmff::BoxType) -> Self {
        match box_type {
            isobmff::BoxType::Compact(compact) => Self::Compact {
                four_cc: compact.four_cc().into(),
            },
            isobmff::BoxType::Extended(user_type) => Self::Extended {
                user_type: user_type.to_string(),
            },
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const BOX_TYPE: &str = r#"
/** `isobmff::BoxType` */
export type BoxType = { type: "compact"; fourCc: FourCc } | { type: "extended"; userType: string };
"#;

/// [`isobmff::BoxSize`]
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum BoxSize {
    /// [`isobmff::BoxSize::Compact`]
    Compact {
        /// [`isobmff::CompactSize::get`]
        size: u32,
    },
    /// [`isobmff::BoxSize::Extended`]
    Extended {
        /// [`isobmff::ExtendedSize::get`]
        size: u64,
    },
    /// [`isobmff::BoxSize::ToEndOfFile`]
    ToEndOfFile,
}

impl From<isobmff::BoxSize> for BoxSize {
    fn from(size: isobmff::BoxSize) -> Self {
        match size {
            isobmff::BoxSize::Compact(compact) => Self::Compact {
                size: compact.get(),
            },
            isobmff::BoxSize::Extended(extended) => Self::Extended {
                size: extended.get(),
            },
            isobmff::BoxSize::ToEndOfFile => Self::ToEndOfFile,
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const BOX_SIZE: &str = r#"
/** `isobmff::BoxSize` */
export type BoxSize = { type: "compact"; size: number } | { type: "extended"; size: bigint } | { type: "toEndOfFile" };
"#;

/// [`isobmff::BoxHeader`]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoxHeader {
    /// [`isobmff::BoxHeader::box_type`]
    box_type: BoxType,
    /// [`isobmff::BoxHeader::size`]
    size: BoxSize,
}

impl From<isobmff::BoxHeader> for BoxHeader {
    fn from(header: isobmff::BoxHeader) -> Self {
        Self {
            box_type: header.box_type().into(),
            size: header.size().into(),
        }
    }
}

#[wasm_bindgen(typescript_custom_section)]
const BOX_HEADER: &str = r#"
/** `isobmff::BoxHeader` */
export interface BoxHeader {
    boxType: BoxType;
    size: BoxSize;
}
"#;

/// [`isobmff::RawBox`], owning a copy of its payload
#[derive(Debug, Clone, Serialize)]
struct RawBox {
    /// [`isobmff::RawBox::header`]
    header: BoxHeader,
    /// [`isobmff::RawBox::payload`]
    #[serde(with = "serde_bytes")]
    payload: Vec<u8>,
}

#[wasm_bindgen(typescript_custom_section)]
const RAW_BOX: &str = r#"
/** `isobmff::RawBox`, owning a copy of its payload */
export interface RawBox {
    header: BoxHeader;
    payload: Uint8Array;
}
"#;

/// One step of [`Boxes`] as the iterator protocol of JavaScript states it
#[derive(Debug, Serialize)]
struct IteratorStep {
    /// Whether the boxes are used up
    done: bool,
    /// The box the step yields, none once the boxes are used up
    value: Option<RawBox>,
}

/// [`isobmff::Boxes`] over a copy of its input, iterable
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
                        header: framed.header().into(),
                        payload: framed.payload().to_vec(),
                    })
                })
                .collect::<Vec<_>>()
                .into_iter(),
        )
    }

    /// [`Iterator::next`] of [`isobmff::Boxes`], as the iterator protocol states a step
    ///
    /// # Errors
    ///
    /// `"box"`: the error [`isobmff::Boxes`] yields, after which every step is
    /// done; `"binding"`: the box did not convert to a plain object.
    #[wasm_bindgen(js_name = next)]
    pub fn next_box(&mut self) -> Result<JsIteratorStep, Failure> {
        let step = self.0.next().transpose();
        if !matches!(step, Ok(Some(_))) {
            // Why not keep the buffer once the boxes are used up or failed: the
            // object `for…of` walks is the one `[Symbol.iterator]` returned,
            // which no caller holds to free, so its memory goes now rather than
            // when the collector comes.
            self.0 = Vec::new().into_iter();
        }
        let value = step?;
        to_plain(&IteratorStep {
            done: value.is_none(),
            value,
        })
    }

    /// Returns the boxes themselves, so that `for…of` walks them
    ///
    /// The boxes returned release what they hold once used up or failed; the
    /// object around them is left to the collector.
    #[wasm_bindgen(js_name = "[Symbol.iterator]")]
    pub fn iterator(self) -> Self {
        self
    }
}
