//! [`ScrambleSchemeInfoBox`], the scrambling applied to the samples of a sample entry or to an item

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::sample_entry::encode_optional;
use crate::schi::SchemeInformationBox;
use crate::schm::SchemeTypeBox;

/// [`ScrambleSchemeInfoBox`] (`scrb`), ISO/IEC 14496-12 §13.4.8
///
/// Everything needed to understand a scrambling applied to the samples of a
/// sample entry or to an item: a scrambling modifies binary patterns of the
/// media without modifying its syntax, so the media still conforms to its
/// format. The box is a `ProtectionSchemeInfoBox` without the
/// `OriginalFormatBox`.
///
/// The box writes `schm` and `schi` in this order, whatever order they were
/// read in: the order of its syntax. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them. A `schi`
/// that does not read is kept there too, and `schi` reads back [`None`]; a
/// box without a `schm` that reads is refused.
#[doc(alias = "scrb")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ScrambleSchemeInfoBox {
    /// Scheme of the scrambling, `scheme_type_box` in the syntax
    pub schm: SchemeTypeBox,
    /// Information the scheme needs, `info` in the syntax
    pub schi: Option<SchemeInformationBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for ScrambleSchemeInfoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"scrb");
}

impl BoxDecode for ScrambleSchemeInfoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            schm: children.take_exactly_one()?,
            schi: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for ScrambleSchemeInfoBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .chain(self.schi.as_ref().map(BoxEncode::encoded_len))
            .fold(self.schm.encoded_len(), u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.schm.encode(writer.take_remainder())?;
        rest = encode_optional(rest, self.schi.as_ref())?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::ScrambleSchemeInfoBox;
    use crate::encoded;
    use crate::schi::SchemeInformationBox;
    use crate::schm::SchemeTypeBox;

    /// `schm` of the scheme `cbcs` version 1.0
    const SCHM: &[u8] = b"\0\0\0\x14schm\0\0\0\0cbcs\0\x01\0\0";

    /// `schi` of one box of the scheme
    const SCHI: &[u8] = b"\0\0\0\x11schi\0\0\0\x09tenc\x01";

    /// `scrb` of the scheme and its information
    const SCRB: &[u8] =
        b"\0\0\0\x2dscrb\0\0\0\x14schm\0\0\0\0cbcs\0\x01\0\0\0\0\0\x11schi\0\0\0\x09tenc\x01";

    fn scramble_scheme_info() -> ScrambleSchemeInfoBox {
        ScrambleSchemeInfoBox {
            schm: SchemeTypeBox {
                scheme_type: FourCc::new(*b"cbcs"),
                scheme_version: 0x0001_0000,
                scheme_uri: None,
            },
            schi: Some(SchemeInformationBox {
                other_boxes: OtherBoxes::from(ChildBoxes::decode(b"\0\0\0\x09tenc\x01").unwrap()),
            }),
            other_boxes: OtherBoxes::new(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ScrambleSchemeInfoBox::decode(SCRB),
            Ok((scramble_scheme_info(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&scramble_scheme_info()), SCRB);
    }

    #[test]
    fn reads_its_children_whatever_order_they_came_in() {
        assert_eq!(
            ScrambleSchemeInfoBox::decode_payload(&[SCHI, SCHM].concat()),
            Ok(scramble_scheme_info())
        );
    }

    #[test]
    fn a_box_without_a_scheme_is_refused() {
        assert_eq!(
            ScrambleSchemeInfoBox::decode_payload(SCHI),
            Err(Error::missing_mandatory_box(SchemeTypeBox::BOX_TYPE))
        );
    }
}
