//! The root of [`isobmff`]: the framing of ISO/IEC 14496-12 §4.2 ([`BoxType`], [`BoxSize`], the [`BoxHeader`] they form, the [`Boxes`] laid end to end) and [`FourCc`]

use alloc::vec;

use serde::Serialize;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::Failure;

/// [`isobmff::FourCc`], the four characters as [`isobmff::FourCc`] displays them
#[derive(Debug, Clone, Serialize, Tsify)]
pub struct FourCc(String);

impl From<isobmff::FourCc> for FourCc {
    fn from(four_cc: isobmff::FourCc) -> Self {
        Self(four_cc.to_string())
    }
}

/// [`isobmff::BoxType`]
#[derive(Debug, Clone, Serialize, Tsify)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum BoxType {
    /// [`isobmff::BoxType::Compact`]
    #[serde(rename_all = "camelCase")]
    #[non_exhaustive]
    Compact {
        /// [`isobmff::CompactType::four_cc`]
        four_cc: FourCc,
    },
    /// [`isobmff::BoxType::Extended`]
    #[serde(rename_all = "camelCase")]
    #[non_exhaustive]
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

/// [`isobmff::BoxSize`]
#[derive(Debug, Clone, Copy, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum BoxSize {
    /// [`isobmff::BoxSize::Compact`]
    #[non_exhaustive]
    Compact {
        /// [`isobmff::CompactSize::get`]
        size: u32,
    },
    /// [`isobmff::BoxSize::Extended`]
    #[non_exhaustive]
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

/// [`isobmff::BoxHeader`]
#[derive(Debug, Clone, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct BoxHeader {
    /// [`isobmff::BoxHeader::box_type`]
    pub box_type: BoxType,
    /// [`isobmff::BoxHeader::size`]
    pub size: BoxSize,
}

impl From<isobmff::BoxHeader> for BoxHeader {
    fn from(header: isobmff::BoxHeader) -> Self {
        Self {
            box_type: header.box_type().into(),
            size: header.size().into(),
        }
    }
}

/// [`isobmff::RawBox`], owning a copy of its payload
#[derive(Debug, Clone, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[non_exhaustive]
pub struct RawBox {
    /// [`isobmff::RawBox::header`]
    pub header: BoxHeader,
    /// [`isobmff::RawBox::payload`]
    #[serde(with = "serde_bytes")]
    #[tsify(type = "Uint8Array")]
    pub payload: Vec<u8>,
}

/// One step of [`Boxes`] as the iterator protocol of JavaScript states it
#[derive(Debug, Serialize, Tsify)]
#[tsify(large_number_types_as_bigints)]
#[non_exhaustive]
pub struct IteratorStep {
    /// Whether the boxes are used up
    pub done: bool,
    /// The box the step yields, none once the boxes are used up
    pub value: Option<RawBox>,
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
    #[wasm_bindgen(js_name = next, unchecked_return_type = "IteratorResult<RawBox, undefined>")]
    pub fn next_box(&mut self) -> Result<Ts<IteratorStep>, Failure> {
        let step = self.0.next().transpose();
        if !matches!(step, Ok(Some(_))) {
            // Why not keep the buffer once the boxes are used up or failed: the
            // object `for…of` walks is the one `[Symbol.iterator]` returned,
            // which no caller holds to free, so its memory goes now rather than
            // when the collector comes.
            self.0 = Vec::new().into_iter();
        }
        let value = step?;
        Ok(IteratorStep {
            done: value.is_none(),
            value,
        }
        .into_ts()?)
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
