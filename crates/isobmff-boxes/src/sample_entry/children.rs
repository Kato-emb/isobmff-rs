//! The boxes any sample entry may hold, and the order an entry lays its children out in

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, OtherBoxes, RawBox,
};

use crate::btrt::BitRateBox;
use crate::rinf::RestrictedSchemeInfoBox;
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;

/// Children of a class that defines no box of its own, which holds only the boxes any sample entry may hold
pub(super) struct AnyEntryChildren {
    pub(super) derived_boxes: OtherBoxes,
    pub(super) sinf: Vec<ProtectionSchemeInfoBox>,
    pub(super) scrb: Vec<ScrambleSchemeInfoBox>,
    pub(super) rinf: Option<RestrictedSchemeInfoBox>,
    pub(super) btrt: Option<BitRateBox>,
    pub(super) extra_boxes: OtherBoxes,
}

impl AnyEntryChildren {
    /// Reads the children `payload` holds, split as [`split_children`] splits them
    ///
    /// # Errors
    ///
    /// * The failures of a child that does not frame, and of a `btrt` given
    ///   more than once.
    pub(super) fn read(payload: &[u8]) -> Result<Self, Error> {
        let (derived_boxes, mut children) = split_children(payload, &[])?;

        Ok(Self {
            derived_boxes,
            sinf: children.take_zero_or_more_if_decoded(),
            scrb: children.take_zero_or_more_if_decoded(),
            rinf: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

/// Boxes of ISO/IEC 14496-12 that any sample entry may hold, which an entry writes after the boxes its class defines, in this order
pub(super) struct AnyEntryBoxes<'entry> {
    pub(super) sinf: &'entry [ProtectionSchemeInfoBox],
    pub(super) scrb: &'entry [ScrambleSchemeInfoBox],
    pub(super) rinf: Option<&'entry RestrictedSchemeInfoBox>,
    pub(super) btrt: Option<&'entry BitRateBox>,
}

impl AnyEntryBoxes<'_> {
    /// Box types of the boxes
    pub(super) const BOX_TYPES: [BoxType; 4] = [
        ProtectionSchemeInfoBox::BOX_TYPE,
        ScrambleSchemeInfoBox::BOX_TYPE,
        RestrictedSchemeInfoBox::BOX_TYPE,
        BitRateBox::BOX_TYPE,
    ];

    /// Returns the length of the boxes
    pub(super) fn encoded_len(&self) -> u64 {
        self.sinf
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.scrb.iter().map(BoxEncode::encoded_len))
            .chain(self.rinf.map(BoxEncode::encoded_len))
            .chain(self.btrt.map(BoxEncode::encoded_len))
            .fold(0, u64::saturating_add)
    }

    /// Writes the boxes onto the front of `rest`, and returns what is left
    ///
    /// # Errors
    ///
    /// * The failures of writing a box.
    pub(super) fn encode<'buffer>(
        &self,
        mut rest: &'buffer mut [u8],
    ) -> Result<&'buffer mut [u8], Error> {
        for sinf in self.sinf {
            rest = sinf.encode(rest)?;
        }
        for scrb in self.scrb {
            rest = scrb.encode(rest)?;
        }
        rest = encode_optional(rest, self.rinf)?;
        encode_optional(rest, self.btrt)
    }
}

/// Returns the length of the children of an entry: `derived_boxes`, the boxes of ISO/IEC 14496-12 the class defines, of the lengths `defined`, and `extra_boxes`
pub(super) fn children_len(
    derived_boxes: &OtherBoxes,
    defined: impl IntoIterator<Item = u64>,
    extra_boxes: &OtherBoxes,
) -> u64 {
    derived_boxes
        .as_slice()
        .iter()
        .chain(extra_boxes.as_slice())
        .map(AnyBox::encoded_len)
        .chain(defined)
        .fold(0, u64::saturating_add)
}

/// Writes the children of an entry onto the front of `rest` in the order of the three parts: `derived_boxes`, the boxes of ISO/IEC 14496-12 that `encode_defined` writes, then `extra_boxes`
///
/// # Errors
///
/// * The failures of writing a child.
pub(super) fn encode_children<'buffer>(
    mut rest: &'buffer mut [u8],
    derived_boxes: &OtherBoxes,
    encode_defined: impl FnOnce(&'buffer mut [u8]) -> Result<&'buffer mut [u8], Error>,
    extra_boxes: &OtherBoxes,
) -> Result<(), Error> {
    for kept in derived_boxes.as_slice() {
        rest = kept.encode(rest)?;
    }
    rest = encode_defined(rest)?;
    for kept in extra_boxes.as_slice() {
        rest = kept.encode(rest)?;
    }

    Ok(())
}

/// Writes `child`, if there is one, onto the front of `rest`, and returns what is left
///
/// # Errors
///
/// * The failures of writing `child`.
pub(crate) fn encode_optional<'buffer, Child: BoxEncode + BoxDefinition>(
    rest: &'buffer mut [u8],
    child: Option<&Child>,
) -> Result<&'buffer mut [u8], Error> {
    match child {
        Some(child) => child.encode(rest),
        None => Ok(rest),
    }
}

/// Splits the children of an entry before the first child of a type in `defined`, the boxes its class defines, or of a type in [`AnyEntryBoxes::BOX_TYPES`]
///
/// Returns the children before it, which are the boxes of a derived
/// specification where the entry lays its children out in the order its
/// class does, and the children from it on, from which the class takes its
/// boxes and keeps the rest as the extra boxes.
///
/// # Errors
///
/// * The failures of [`ChildBoxes::decode`]: a child does not frame.
pub(super) fn split_children<'payload>(
    payload: &'payload [u8],
    defined: &[BoxType],
) -> Result<(OtherBoxes, ChildBoxes<'payload>), Error> {
    let mut rest = payload;
    while !rest.is_empty() {
        let (child, after) = RawBox::split_first(rest)?;
        let box_type = child.header().box_type();
        if defined.contains(&box_type) || AnyEntryBoxes::BOX_TYPES.contains(&box_type) {
            break;
        }
        rest = after;
    }
    let (derived, defined_onward) = payload.split_at(payload.len().saturating_sub(rest.len()));

    Ok((
        OtherBoxes::from(ChildBoxes::decode(derived)?),
        ChildBoxes::decode(defined_onward)?,
    ))
}
