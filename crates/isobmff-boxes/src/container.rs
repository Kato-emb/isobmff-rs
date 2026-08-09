//! Promoting the boxes a container holds into the fields that claim them

use alloc::vec::Vec;

use isobmff_core::{AnyBox, BoxDecode, BoxDefinition, DecodeError, EncodeError, RawBox};

/// Decodes a child into a field that claims at most one box of its type
pub(crate) fn promote_once<Child>(
    slot: &mut Option<Child>,
    child: RawBox<'_>,
) -> Result<(), DecodeError>
where
    Child: BoxDecode + BoxDefinition,
{
    if slot.is_some() {
        return Err(DecodeError::DuplicateBox(Child::BOX_TYPE));
    }

    *slot = Some(decode_child::<Child>(child)?);

    Ok(())
}

/// Decodes a child and appends it to the field that claims every box of its type
pub(crate) fn promote_each<Child>(
    children: &mut Vec<Child>,
    child: RawBox<'_>,
) -> Result<(), DecodeError>
where
    Child: BoxDecode + BoxDefinition,
{
    children.push(decode_child::<Child>(child)?);

    Ok(())
}

/// Decodes one child, naming it in whatever failure it reports
fn decode_child<Child>(child: RawBox<'_>) -> Result<Child, DecodeError>
where
    Child: BoxDecode + BoxDefinition,
{
    Child::decode_payload(child.payload())
        .map_err(|error| DecodeError::child(Child::BOX_TYPE, error))
}

/// Keeps a child no field claims, as the bytes it lies as
pub(crate) fn keep_unpromoted(other_boxes: &mut Vec<AnyBox>, child: RawBox<'_>) {
    other_boxes.push(AnyBox::from_raw_bytes(
        child.header().box_type(),
        child.payload().to_vec(),
    ));
}

/// Returns the child of a field the spec marks mandatory, or reports it missing
pub(crate) fn require<Child: BoxDefinition>(slot: Option<Child>) -> Result<Child, DecodeError> {
    slot.ok_or(DecodeError::MissingMandatoryBox(Child::BOX_TYPE))
}

/// Returns the children of a field the spec requires at least one box in
pub(crate) fn require_any<Child: BoxDefinition>(
    children: Vec<Child>,
) -> Result<Vec<Child>, DecodeError> {
    if children.is_empty() {
        return Err(DecodeError::MissingMandatoryBox(Child::BOX_TYPE));
    }

    Ok(children)
}

/// Returns the length a run of erased boxes occupies
pub(crate) fn total_encoded_len(children: &[AnyBox]) -> u64 {
    children
        .iter()
        .fold(0, |total, child| total.saturating_add(child.encoded_len()))
}

/// Writes a run of erased boxes, in the order they are held
pub(crate) fn write_all<'buffer>(
    children: &[AnyBox],
    rest: &'buffer mut [u8],
) -> Result<&'buffer mut [u8], EncodeError> {
    let mut remaining = rest;
    for child in children {
        remaining = child.encode(remaining)?;
    }

    Ok(remaining)
}
