//! [`RestrictedSchemeInfoBox`], the restriction a sample entry is under

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::csch::CompatibleSchemeTypeBox;
use crate::frma::OriginalFormatBox;
use crate::sample_entry::encode_optional;
use crate::schi::SchemeInformationBox;
use crate::schm::SchemeTypeBox;

/// [`RestrictedSchemeInfoBox`] (`rinf`), ISO/IEC 14496-12 §13.5.3
///
/// Everything needed to understand the restriction applied to a sample
/// entry, which requires actions of the player or the renderer: the
/// original sample entry type, and the scheme of the restriction with its
/// information. The box is formatted as a `ProtectionSchemeInfoBox`.
///
/// `frma` is mandatory in a restricted sample entry; a sample entry whose
/// type was not transformed may hold the box as advisory information without
/// one. The box reads and writes a `frma` wherever there is one.
///
/// The box writes `frma`, `schm`, the `csch` boxes and `schi` in this order,
/// whatever order they were read in: the order of its syntax, with the
/// `csch` boxes, which §13.5.5 lets the box hold, after the `schm` they add
/// schemes to. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them. A
/// `frma`, `csch` or `schi` that does not read is kept there too; a box
/// without a `schm` that reads is refused.
#[doc(alias = "rinf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct RestrictedSchemeInfoBox {
    /// Original sample entry type, `original_format` in the syntax
    pub frma: Option<OriginalFormatBox>,
    /// Scheme of the restriction, `scheme_type_box` in the syntax
    pub schm: SchemeTypeBox,
    /// Further schemes the track conforms to, in the order they came
    pub csch: Vec<CompatibleSchemeTypeBox>,
    /// Information the schemes need, `info` in the syntax
    pub schi: Option<SchemeInformationBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for RestrictedSchemeInfoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"rinf");
}

impl BoxDecode for RestrictedSchemeInfoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            frma: children.take_zero_or_one_if_decoded()?,
            schm: children.take_exactly_one()?,
            csch: children.take_zero_or_more_if_decoded(),
            schi: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for RestrictedSchemeInfoBox {
    fn payload_len(&self) -> u64 {
        [
            self.frma.as_ref().map_or(0, BoxEncode::encoded_len),
            self.schm.encoded_len(),
            self.schi.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.csch.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = encode_optional(writer.take_remainder(), self.frma.as_ref())?;
        rest = self.schm.encode(rest)?;
        for csch in &self.csch {
            rest = csch.encode(rest)?;
        }
        rest = encode_optional(rest, self.schi.as_ref())?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::RestrictedSchemeInfoBox;
    use crate::csch::CompatibleSchemeTypeBox;
    use crate::encoded;
    use crate::frma::OriginalFormatBox;
    use crate::schi::SchemeInformationBox;
    use crate::schm::SchemeTypeBox;

    /// `frma` of the format `avc1`
    const FRMA: &[u8] = b"\0\0\0\x0cfrmaavc1";

    /// `schm` of the scheme `stvi` version 1
    const SCHM: &[u8] = b"\0\0\0\x14schm\0\0\0\0stvi\0\0\0\x01";

    /// `csch` of the scheme `podv` version 1
    const CSCH: &[u8] = b"\0\0\0\x14csch\0\0\0\0podv\0\0\0\x01";

    /// `schi` of one box of the schemes
    const SCHI: &[u8] = b"\0\0\0\x11schi\0\0\0\x09xtra\x01";

    /// Box of a type `rinf` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn restricted_scheme_info() -> RestrictedSchemeInfoBox {
        RestrictedSchemeInfoBox {
            frma: Some(OriginalFormatBox {
                data_format: FourCc::new(*b"avc1"),
            }),
            schm: SchemeTypeBox {
                scheme_type: FourCc::new(*b"stvi"),
                scheme_version: 1,
                scheme_uri: None,
            },
            csch: vec![CompatibleSchemeTypeBox {
                scheme_type: FourCc::new(*b"podv"),
                scheme_version: 1,
                scheme_uri: None,
            }],
            schi: Some(SchemeInformationBox {
                other_boxes: OtherBoxes::from(ChildBoxes::decode(b"\0\0\0\x09xtra\x01").unwrap()),
            }),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_whatever_order_they_came_in() {
        assert_eq!(
            RestrictedSchemeInfoBox::decode(
                &[b"\0\0\0\x55rinf".as_slice(), FREE, SCHI, CSCH, SCHM, FRMA].concat()
            ),
            Ok((restricted_scheme_info(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_original_format_the_schemes_their_information_then_the_other_children() {
        assert_eq!(
            encoded(&restricted_scheme_info()),
            [b"\0\0\0\x55rinf".as_slice(), FRMA, SCHM, CSCH, SCHI, FREE].concat()
        );
    }

    #[test]
    fn an_advisory_box_holds_no_original_format() {
        let expected = RestrictedSchemeInfoBox {
            frma: None,
            other_boxes: OtherBoxes::new(),
            ..restricted_scheme_info()
        };

        assert_eq!(
            RestrictedSchemeInfoBox::decode_payload(&[SCHM, CSCH, SCHI].concat()),
            Ok(expected)
        );
    }

    #[test]
    fn a_box_without_a_scheme_is_refused() {
        assert_eq!(
            RestrictedSchemeInfoBox::decode_payload(&[FRMA, SCHI].concat()),
            Err(Error::missing_mandatory_box(SchemeTypeBox::BOX_TYPE))
        );
    }
}
