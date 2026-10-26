//! [`TrackReferenceBox`], the references of a track to other tracks and track groups

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, FieldReader, FieldWriter,
    FourCc,
};

use crate::box_type_argument;
use crate::table::{entries_len, read_to_end, write_entries};

/// [`TrackReferenceTypeBox`], ISO/IEC 14496-12 §8.3.3
///
/// References of one type from the track to other tracks, or to track
/// groups. The type is the box type of the box, which [`TrackReferenceBox`]
/// reads into [`reference_type`](Self::reference_type) and writes from it, so
/// a box of any type, registered or not, reads.
///
/// The box has no box type of its own, so it implements neither
/// [`BoxDecode`] nor [`BoxDefinition`]: its [`BoxEncode`] writes its payload,
/// and a [`TrackReferenceBox`] writes the header.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackReferenceTypeBox {
    /// Type of the references, such as `hint`, `cdsc` or `font`
    ///
    /// It is written as the box type, so a `reference_type` of `uuid` is
    /// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue)
    /// for `reference_type` when the box is written. So is a
    /// `reference_type` that another [`TrackReference::Typed`] of the same
    /// [`TrackReferenceBox`] holds.
    pub reference_type: FourCc,
    /// `track_ID` of each track referenced, or `track_group_id` of each track group referenced
    ///
    /// A `track_group_id` refers to the track group whose
    /// [`TrackGroupTypeBox`](crate::TrackGroupTypeBox) has bit 0 of its
    /// `flags` set. Other structures index this array from 1.
    ///
    /// A value of 0, or one given twice, is read as found, but refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `track_IDs` when the box is written.
    pub track_ids: Vec<u32>,
}

impl Default for TrackReferenceTypeBox {
    fn default() -> Self {
        Self {
            reference_type: FourCc::new([0; 4]),
            track_ids: Vec::new(),
        }
    }
}

impl TrackReferenceTypeBox {
    /// Reads the payload of a box of the box type `reference_type`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a `track_IDs` entry.
    fn decode_payload(reference_type: FourCc, payload: &[u8]) -> Result<Self, Error> {
        Ok(Self {
            reference_type,
            track_ids: read_to_end(&mut FieldReader::new(payload))?,
        })
    }
}

impl BoxEncode for TrackReferenceTypeBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.track_ids)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut sorted = self.track_ids.clone();
        sorted.sort_unstable();
        if sorted.first() == Some(&0) || has_adjacent_equal(&sorted) {
            return Err(Error::invalid_value("track_IDs"));
        }
        write_entries(writer, &self.track_ids)
    }
}

/// Returns whether two neighbours of `values` are equal, which in sorted values means a value occurs more than once
fn has_adjacent_equal<Value: PartialEq>(values: &[Value]) -> bool {
    values
        .windows(2)
        .any(|pair| matches!(pair, [first, second] if first == second))
}

/// One child of a [`TrackReferenceBox`]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum TrackReference {
    /// References of the type the box type gives
    Typed(TrackReferenceTypeBox),
    /// Child under a `uuid` box type, or one that does not decode, kept as its bytes
    Other(AnyBox),
}

/// [`TrackReferenceBox`] (`tref`), ISO/IEC 14496-12 §8.3.3
///
/// References from the track to other tracks, or to track groups, one
/// [`TrackReferenceTypeBox`] for each type of reference.
///
/// Every child under a compact box type is read as a
/// [`TrackReferenceTypeBox`] of that type. A child under a `uuid` box type,
/// and one whose payload ends inside a `track_IDs` entry, is kept as its
/// bytes. The children are kept, and written back, in the order they came.
///
/// What §8.3.3 forbids is read as found, but refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) when the box is
/// written: two [`TrackReference::Typed`] of one `reference_type`, and a
/// `track_IDs` entry of 0 or given twice in one reference.
///
/// # Untrusted input
///
/// The box reads each child as it frames it, and allocates room for a child
/// only once it has read it, so what it allocates is bounded by the length of
/// the payload, which the caller sets.
#[doc(alias = "tref")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct TrackReferenceBox {
    /// Children, in the order they came
    pub track_references: Vec<TrackReference>,
}

impl BoxDefinition for TrackReferenceBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tref");
}

impl BoxDecode for TrackReferenceBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut track_references = Vec::new();
        for child in Boxes::new(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            let payload = child.payload();
            let read = match box_type {
                BoxType::Compact(reference_type) => {
                    TrackReferenceTypeBox::decode_payload(reference_type.four_cc(), payload)
                        .map(TrackReference::Typed)
                        .ok()
                }
                BoxType::Extended(_) => None,
            };
            track_references.push(read.unwrap_or_else(|| {
                TrackReference::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self { track_references })
    }
}

impl BoxEncode for TrackReferenceBox {
    fn payload_len(&self) -> u64 {
        self.track_references
            .iter()
            .map(|child| match child {
                TrackReference::Typed(typed) => box_type_argument::encoded_len(typed),
                TrackReference::Other(other) => other.encoded_len(),
            })
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut reference_types: Vec<FourCc> = self
            .track_references
            .iter()
            .filter_map(|child| match child {
                TrackReference::Typed(typed) => Some(typed.reference_type),
                TrackReference::Other(_) => None,
            })
            .collect();
        reference_types.sort_unstable();
        if has_adjacent_equal(&reference_types) {
            return Err(Error::invalid_value("reference_type"));
        }
        let mut rest = writer.take_remainder();
        for child in &self.track_references {
            rest = match child {
                TrackReference::Typed(typed) => {
                    box_type_argument::encode(typed.reference_type, "reference_type", typed, rest)?
                }
                TrackReference::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error, FourCc};

    use super::{TrackReference, TrackReferenceBox, TrackReferenceTypeBox};
    use crate::encoded;

    /// `hint` reference to the track 1
    const HINT: &[u8] = b"\0\0\0\x0chint\0\0\0\x01";

    /// References of a type ISO/IEC 14496-12 does not register, to the tracks 3 and 2
    const UNREGISTERED: &[u8] = b"\0\0\0\x10xref\0\0\0\x03\0\0\0\x02";

    /// Child under a `uuid` box type
    const EXTENDED: &[u8] =
        b"\0\0\0\x1cuuid\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\0\0\0\x07";

    fn typed(reference_type: &[u8; 4], track_ids: &[u32]) -> TrackReference {
        TrackReference::Typed(TrackReferenceTypeBox {
            reference_type: FourCc::new(*reference_type),
            track_ids: track_ids.to_vec(),
        })
    }

    fn references() -> TrackReferenceBox {
        TrackReferenceBox {
            track_references: vec![
                typed(b"hint", &[1]),
                TrackReference::Other(AnyBox::from_raw_bytes(
                    BoxType::extended(
                        *b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10",
                    ),
                    b"\0\0\0\x07".to_vec(),
                )),
                typed(b"xref", &[3, 2]),
            ],
        }
    }

    #[test]
    fn reads_every_child_of_a_compact_type_as_references_of_that_type_in_the_order_they_came() {
        let bytes = [b"\0\0\0\x40tref".as_slice(), HINT, EXTENDED, UNREGISTERED].concat();

        assert_eq!(
            TrackReferenceBox::decode(&bytes),
            Ok((references(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_each_child_under_its_type_in_the_order_it_holds_them() {
        assert_eq!(
            encoded(&references()),
            [b"\0\0\0\x40tref".as_slice(), HINT, EXTENDED, UNREGISTERED].concat()
        );
    }

    #[test]
    fn a_child_ending_inside_a_track_id_is_kept_as_its_bytes() {
        let expected = TrackReferenceBox {
            track_references: vec![TrackReference::Other(AnyBox::from_raw_bytes(
                BoxType::compact(*b"cdsc"),
                b"\0\0\0\x01\0\0".to_vec(),
            ))],
        };

        assert_eq!(
            TrackReferenceBox::decode_payload(b"\0\0\0\x0ecdsc\0\0\0\x01\0\0"),
            Ok(expected)
        );
    }

    #[test]
    fn a_child_cut_short_by_the_payload_is_refused() {
        assert_eq!(
            TrackReferenceBox::decode_payload(b"\0\0\0\x0chint\0\0"),
            Err(Error::truncated_box(12, 10))
        );
    }

    #[test]
    fn references_of_the_type_uuid_are_refused_when_written() {
        let references = TrackReferenceBox {
            track_references: vec![typed(b"uuid", &[1])],
        };

        assert_eq!(
            references.encode(&mut [0; 20]),
            Err(Error::invalid_value("reference_type"))
        );
    }

    #[test]
    fn two_references_of_one_type_are_refused_when_written() {
        let references = TrackReferenceBox {
            track_references: vec![typed(b"hint", &[1]), typed(b"hint", &[2])],
        };

        assert_eq!(
            references.encode(&mut [0; 32]),
            Err(Error::invalid_value("reference_type"))
        );
    }

    #[test]
    fn a_track_id_of_0_is_refused_when_written() {
        let references = TrackReferenceBox {
            track_references: vec![typed(b"hint", &[0])],
        };

        assert_eq!(
            references.encode(&mut [0; 20]),
            Err(Error::invalid_value("track_IDs"))
        );
    }

    #[test]
    fn a_track_id_given_twice_in_one_reference_is_refused_when_written() {
        let references = TrackReferenceBox {
            track_references: vec![typed(b"hint", &[1, 2, 1])],
        };

        assert_eq!(
            references.encode(&mut [0; 28]),
            Err(Error::invalid_value("track_IDs"))
        );
    }

    #[test]
    fn references_that_break_the_limits_of_writing_are_read_as_found() {
        let expected = TrackReferenceBox {
            track_references: vec![typed(b"hint", &[0, 1, 1]), typed(b"hint", &[2])],
        };

        assert_eq!(
            TrackReferenceBox::decode_payload(
                b"\0\0\0\x14hint\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\0\x0chint\0\0\0\x02"
            ),
            Ok(expected)
        );
    }
}
