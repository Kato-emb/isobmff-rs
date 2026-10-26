//! [`MetadataKeyTableBox`], the keys of a multiplexed timed metadata track, and the [`MetadataKeyBox`]es it holds

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, ChildBoxes, Error, FieldReader,
    FieldWriter, FourCc, OtherBoxes,
};

use crate::box_type_argument;
use crate::keyd::MetadataKeyDeclarationBox;
use crate::loca::MetadataLocaleBox;
use crate::setu::MetadataSetupBox;
use crate::tref::has_adjacent_equal;

/// `local_key_id` that §12.8.4.3.1 keeps for value boxes inline in samples, which no key declares
const INLINE_KEY_ID: FourCc = FourCc::new([0xff; 4]);

/// `local_key_id` that marks a key unused, which any number of keys may take
const UNUSED_KEY_ID: FourCc = FourCc::new([0; 4]);

/// [`MetadataKeyBox`], ISO/IEC 14496-12 §12.8.4.3
///
/// One key the samples of a multiplexed timed metadata track may hold values
/// of: what the key is, the locale of its values, and its setup data. The
/// box type of the box is the `local_key_id` the value boxes in the samples
/// take, which [`MetadataKeyTableBox`] reads into
/// [`local_key_id`](Self::local_key_id) and writes from it, so a box of any
/// type reads.
///
/// The box writes `keyd`, `loca` and `setu` in this order, whatever order they
/// were read in: the order of its Syntax. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them, and so is
/// a `loca` or a `setu` that does not read, which then reads back [`None`].
///
/// The box has no box type of its own, so it implements neither
/// [`BoxDecode`] nor [`BoxDefinition`]: its [`BoxEncode`] writes its payload,
/// and a [`MetadataKeyTableBox`] writes the header.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MetadataKeyBox {
    /// Identifier of the key, the box type of its value boxes in the samples
    ///
    /// It is written as the box type. A `local_key_id` of four zero bytes
    /// marks the key unused. One of `uuid`, of four `0xff` bytes, which
    /// §12.8.4.3.1 keeps for values inline in samples, or one another
    /// [`MetadataKey::Typed`] of the same [`MetadataKeyTableBox`] holds, other
    /// than four zero bytes, is refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `local_key_id` when the box is written.
    pub local_key_id: FourCc,
    /// Namespace and value of the key
    pub keyd: MetadataKeyDeclarationBox,
    /// Locale of the values, which are for every locale where this is [`None`]
    pub loca: Option<MetadataLocaleBox>,
    /// Setup data the values need
    pub setu: Option<MetadataSetupBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl Default for MetadataKeyBox {
    fn default() -> Self {
        Self {
            local_key_id: UNUSED_KEY_ID,
            keyd: MetadataKeyDeclarationBox::default(),
            loca: None,
            setu: None,
            other_boxes: OtherBoxes::default(),
        }
    }
}

impl MetadataKeyBox {
    /// Reads the payload of a box of the box type `local_key_id`
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox):
    ///   the box holds no `keyd`.
    /// * The failures of a child that does not frame, and of a `keyd` that
    ///   does not read.
    fn decode_payload(local_key_id: FourCc, payload: &[u8]) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(payload)?;

        Ok(Self {
            local_key_id,
            keyd: children.take_exactly_one()?,
            loca: children.take_zero_or_one_if_decoded()?,
            setu: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for MetadataKeyBox {
    fn payload_len(&self) -> u64 {
        [
            self.keyd.encoded_len(),
            self.loca.as_ref().map_or(0, BoxEncode::encoded_len),
            self.setu.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.keyd.encode(writer.take_remainder())?;
        if let Some(loca) = &self.loca {
            rest = loca.encode(rest)?;
        }
        if let Some(setu) = &self.setu {
            rest = setu.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

/// One child of a [`MetadataKeyTableBox`]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum MetadataKey {
    /// Key of the `local_key_id` the box type gives
    Typed(MetadataKeyBox),
    /// Child under a `uuid` box type, or one that does not decode, kept as its bytes
    Other(AnyBox),
}

/// [`MetadataKeyTableBox`] (`keys`), ISO/IEC 14496-12 §12.8.4.2
///
/// The keys the samples of a multiplexed timed metadata track may hold values
/// of, one [`MetadataKeyBox`] each. A key it holds need not have values in
/// any sample.
///
/// Every child under a compact box type is read as a [`MetadataKeyBox`] of
/// that `local_key_id`, `free` included, which here has no meaning of its
/// own. A child under a `uuid` box type, and one that does not decode, such as
/// one of no `keyd`, is kept as its bytes. The children are kept, and written
/// back, in the order they came.
///
/// The box holds one or more keys. A box of none reads, but writing it is
/// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `MetadataKeyBox`, so a box built from [`Default`] is given a key before it
/// is written.
///
/// # Untrusted input
///
/// The box reads each child as it frames it, and allocates room for a child
/// only once it has read it, so what it allocates is bounded by the length of
/// the payload, which the caller sets.
#[doc(alias = "keys")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct MetadataKeyTableBox {
    /// Keys, in the order they came
    pub keys: Vec<MetadataKey>,
}

impl BoxDefinition for MetadataKeyTableBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"keys");
}

impl BoxDecode for MetadataKeyTableBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut keys = Vec::new();
        for child in Boxes::new(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            let payload = child.payload();
            let read = match box_type {
                BoxType::Compact(local_key_id) => {
                    MetadataKeyBox::decode_payload(local_key_id.four_cc(), payload)
                        .map(MetadataKey::Typed)
                        .ok()
                }
                BoxType::Extended(_) => None,
            };
            keys.push(read.unwrap_or_else(|| {
                MetadataKey::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self { keys })
    }
}

impl BoxEncode for MetadataKeyTableBox {
    fn payload_len(&self) -> u64 {
        self.keys
            .iter()
            .map(|child| match child {
                MetadataKey::Typed(key) => box_type_argument::encoded_len(key),
                MetadataKey::Other(other) => other.encoded_len(),
            })
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.keys.is_empty() {
            return Err(Error::invalid_value("MetadataKeyBox"));
        }
        let mut local_key_ids: Vec<FourCc> = self
            .keys
            .iter()
            .filter_map(|child| match child {
                MetadataKey::Typed(key) if key.local_key_id != UNUSED_KEY_ID => {
                    Some(key.local_key_id)
                }
                MetadataKey::Typed(_) | MetadataKey::Other(_) => None,
            })
            .collect();
        local_key_ids.sort_unstable();
        if has_adjacent_equal(&local_key_ids) || local_key_ids.contains(&INLINE_KEY_ID) {
            return Err(Error::invalid_value("local_key_id"));
        }
        let mut rest = writer.take_remainder();
        for child in &self.keys {
            rest = match child {
                MetadataKey::Typed(key) => {
                    box_type_argument::encode(key.local_key_id, "local_key_id", key, rest)?
                }
                MetadataKey::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_core::{
        AnyBox, BoxDecode, BoxEncode, BoxType, ChildBoxes, Error, FourCc, OtherBoxes, Utf8String,
    };

    use super::{MetadataKey, MetadataKeyBox, MetadataKeyTableBox};
    use crate::encoded;
    use crate::keyd::MetadataKeyDeclarationBox;
    use crate::loca::MetadataLocaleBox;
    use crate::setu::MetadataSetupBox;

    /// `keyd` of the namespace `uiso` and the user-data key `cprt`
    const KEYD: &[u8] = b"\0\0\0\x10keyduisocprt";

    /// `loca` of `fr-FR`
    const LOCA: &[u8] = b"\0\0\0\x0elocafr-FR\0";

    /// `setu` of one byte
    const SETU: &[u8] = b"\0\0\0\x09setu\x07";

    /// `xtra` holding nothing, a box `MetadataKeyBox` holds no field for
    const XTRA: &[u8] = b"\0\0\0\x08xtra";

    fn declaration() -> MetadataKeyDeclarationBox {
        MetadataKeyDeclarationBox::decode(KEYD).unwrap().0
    }

    fn table() -> MetadataKeyTableBox {
        MetadataKeyTableBox {
            keys: vec![
                MetadataKey::Typed(MetadataKeyBox {
                    local_key_id: FourCc::new(*b"cprt"),
                    keyd: declaration(),
                    loca: Some(MetadataLocaleBox {
                        locale_string: Utf8String::new(String::from("fr-FR")).unwrap(),
                    }),
                    setu: Some(MetadataSetupBox { data: vec![7] }),
                    other_boxes: OtherBoxes::from(ChildBoxes::decode(XTRA).unwrap()),
                }),
                MetadataKey::Typed(MetadataKeyBox {
                    local_key_id: FourCc::new([0, 0, 0, 72]),
                    keyd: declaration(),
                    ..MetadataKeyBox::default()
                }),
                MetadataKey::Other(AnyBox::from_raw_bytes(
                    BoxType::compact(*b"lost"),
                    LOCA.to_vec(),
                )),
            ],
        }
    }

    #[test]
    fn reads_every_child_of_a_compact_type_as_the_key_of_that_local_key_id() {
        let cprt = [b"\0\0\0\x37cprt".as_slice(), XTRA, SETU, LOCA, KEYD].concat();
        let integer = [b"\0\0\0\x18\0\0\0\x48".as_slice(), KEYD].concat();
        let no_declaration = [b"\0\0\0\x16lost".as_slice(), LOCA].concat();
        let bytes = [
            b"\0\0\0\x6dkeys".as_slice(),
            &cprt,
            &integer,
            &no_declaration,
        ]
        .concat();

        assert_eq!(
            MetadataKeyTableBox::decode(&bytes),
            Ok((table(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_each_key_under_its_local_key_id_with_keyd_then_loca_then_setu() {
        let cprt = [b"\0\0\0\x37cprt".as_slice(), KEYD, LOCA, SETU, XTRA].concat();
        let integer = [b"\0\0\0\x18\0\0\0\x48".as_slice(), KEYD].concat();
        let no_declaration = [b"\0\0\0\x16lost".as_slice(), LOCA].concat();

        assert_eq!(
            encoded(&table()),
            [
                b"\0\0\0\x6dkeys".as_slice(),
                &cprt,
                &integer,
                &no_declaration
            ]
            .concat()
        );
    }

    #[test]
    fn what_the_keys_may_not_hold_is_refused_when_written() {
        let key = |local_key_id: [u8; 4]| {
            MetadataKey::Typed(MetadataKeyBox {
                local_key_id: FourCc::new(local_key_id),
                keyd: declaration(),
                ..MetadataKeyBox::default()
            })
        };
        let empty = MetadataKeyTableBox::default();
        let twice = MetadataKeyTableBox {
            keys: vec![key(*b"cprt"), key(*b"cprt")],
        };
        let inline = MetadataKeyTableBox {
            keys: vec![key([0xff; 4])],
        };
        let unused_twice = MetadataKeyTableBox {
            keys: vec![key([0; 4]), key([0; 4])],
        };

        assert_eq!(
            (
                empty.encode(&mut [0; 8]),
                twice.encode(&mut [0; 64]),
                inline.encode(&mut [0; 64]),
                unused_twice.encode(&mut [0; 56]).map(|_| ())
            ),
            (
                Err(Error::invalid_value("MetadataKeyBox")),
                Err(Error::invalid_value("local_key_id")),
                Err(Error::invalid_value("local_key_id")),
                Ok(())
            )
        );
    }
}
