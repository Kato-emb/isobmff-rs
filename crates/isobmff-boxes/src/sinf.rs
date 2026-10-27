//! [`ProtectionSchemeInfoBox`], the protection a sample entry or an item is under

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::frma::OriginalFormatBox;
use crate::sample_entry::encode_optional;
use crate::schi::SchemeInformationBox;
use crate::schm::SchemeTypeBox;

/// [`ProtectionSchemeInfoBox`] (`sinf`), ISO/IEC 14496-12 §13.4.2
///
/// Everything needed to understand the protection applied to the samples of
/// a protected sample entry or to an item: the original format of a sample
/// entry, and the scheme of the protection with its information. More than
/// one in a sample entry are alternative descriptions of the same protection.
///
/// `frma` is mandatory in a protected sample entry and shall not be present
/// for item protection; the box reads and writes whichever is there. `schm`
/// and `schi` occur both or neither: either reads alone, but writing one
/// without the other, in its field or in [`other_boxes`](Self::other_boxes),
/// is refused with
/// [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox).
///
/// The box writes `frma`, `schm` and `schi` in this order, whatever order
/// they were read in: the order of Table 2. Children of other types are kept
/// in [`other_boxes`](Self::other_boxes), and written back after them. A
/// `frma`, `schm` or `schi` that does not read is kept there too, and the
/// field reads back [`None`].
#[doc(alias = "sinf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ProtectionSchemeInfoBox {
    /// Format of the sample entry before it was protected, `original_format` in the syntax
    pub frma: Option<OriginalFormatBox>,
    /// Scheme of the protection, `scheme_type_box` in the syntax
    pub schm: Option<SchemeTypeBox>,
    /// Information the scheme needs, `info` in the syntax
    pub schi: Option<SchemeInformationBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for ProtectionSchemeInfoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sinf");
}

impl BoxDecode for ProtectionSchemeInfoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            frma: children.take_zero_or_one_if_decoded()?,
            schm: children.take_zero_or_one_if_decoded()?,
            schi: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for ProtectionSchemeInfoBox {
    fn payload_len(&self) -> u64 {
        [
            self.frma.as_ref().map_or(0, BoxEncode::encoded_len),
            self.schm.as_ref().map_or(0, BoxEncode::encoded_len),
            self.schi.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let holds = |typed: bool, box_type: BoxType| {
            typed
                || self
                    .other_boxes
                    .as_slice()
                    .iter()
                    .any(|kept| kept.box_type() == box_type)
        };
        let has_schm = holds(self.schm.is_some(), SchemeTypeBox::BOX_TYPE);
        if has_schm != holds(self.schi.is_some(), SchemeInformationBox::BOX_TYPE) {
            return Err(Error::missing_mandatory_box(if has_schm {
                SchemeInformationBox::BOX_TYPE
            } else {
                SchemeTypeBox::BOX_TYPE
            }));
        }

        let mut rest = encode_optional(writer.take_remainder(), self.frma.as_ref())?;
        rest = encode_optional(rest, self.schm.as_ref())?;
        rest = encode_optional(rest, self.schi.as_ref())?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FourCc, OtherBoxes,
    };

    use super::ProtectionSchemeInfoBox;
    use crate::encoded;
    use crate::frma::OriginalFormatBox;
    use crate::schi::SchemeInformationBox;
    use crate::schm::SchemeTypeBox;

    /// `frma` of the format `avc1`
    const FRMA: &[u8] = b"\0\0\0\x0cfrmaavc1";

    /// `schm` of the scheme `cenc` version 1.0
    const SCHM: &[u8] = b"\0\0\0\x14schm\0\0\0\0cenc\0\x01\0\0";

    /// `schi` of one box of the scheme
    const SCHI: &[u8] = b"\0\0\0\x11schi\0\0\0\x09tenc\x01";

    /// Box of a type `sinf` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn protection_scheme_info() -> ProtectionSchemeInfoBox {
        ProtectionSchemeInfoBox {
            frma: Some(OriginalFormatBox {
                data_format: FourCc::new(*b"avc1"),
            }),
            schm: Some(SchemeTypeBox {
                scheme_type: FourCc::new(*b"cenc"),
                scheme_version: 0x0001_0000,
                scheme_uri: None,
            }),
            schi: Some(SchemeInformationBox {
                other_boxes: OtherBoxes::from(ChildBoxes::decode(b"\0\0\0\x09tenc\x01").unwrap()),
            }),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_children_whatever_order_they_came_in() {
        assert_eq!(
            ProtectionSchemeInfoBox::decode(
                &[b"\0\0\0\x41sinf".as_slice(), FREE, SCHI, SCHM, FRMA].concat()
            ),
            Ok((protection_scheme_info(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_original_format_the_scheme_its_information_then_the_other_children() {
        assert_eq!(
            encoded(&protection_scheme_info()),
            [b"\0\0\0\x41sinf".as_slice(), FRMA, SCHM, SCHI, FREE].concat()
        );
    }

    #[test]
    fn a_box_for_item_protection_holds_no_original_format() {
        let expected = ProtectionSchemeInfoBox {
            frma: None,
            other_boxes: OtherBoxes::new(),
            ..protection_scheme_info()
        };

        assert_eq!(
            ProtectionSchemeInfoBox::decode_payload(&[SCHM, SCHI].concat()),
            Ok(expected)
        );
    }

    #[test]
    fn a_scheme_of_a_version_it_does_not_read_is_kept_as_another_child() {
        let schm_version_1 = b"\0\0\0\x14schm\x01\0\0\0cenc\0\x01\0\0";
        let expected = ProtectionSchemeInfoBox {
            frma: None,
            schm: None,
            schi: None,
            other_boxes: OtherBoxes::from(ChildBoxes::decode(schm_version_1).unwrap()),
        };

        assert_eq!(
            ProtectionSchemeInfoBox::decode_payload(schm_version_1),
            Ok(expected)
        );
    }

    #[test]
    fn a_scheme_given_twice_is_refused() {
        assert_eq!(
            ProtectionSchemeInfoBox::decode_payload(&[SCHM, SCHM].concat()),
            Err(Error::duplicate_box(BoxType::compact(*b"schm")))
        );
    }

    #[test]
    fn a_scheme_without_its_information_is_refused_when_written() {
        let without_schi = ProtectionSchemeInfoBox {
            schi: None,
            ..protection_scheme_info()
        };

        assert_eq!(
            without_schi.encode(&mut [0; 48]).map(|_| ()),
            Err(Error::missing_mandatory_box(SchemeInformationBox::BOX_TYPE))
        );
    }

    #[test]
    fn scheme_information_without_its_scheme_is_refused_when_written() {
        let without_schm = ProtectionSchemeInfoBox {
            schm: None,
            ..protection_scheme_info()
        };

        assert_eq!(
            without_schm.encode(&mut [0; 45]).map(|_| ()),
            Err(Error::missing_mandatory_box(SchemeTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_scheme_kept_as_another_child_pairs_with_its_information() {
        let schm_version_1 = b"\0\0\0\x14schm\x01\0\0\0cenc\0\x01\0\0";
        let kept_schm = ProtectionSchemeInfoBox {
            schm: None,
            other_boxes: OtherBoxes::from(ChildBoxes::decode(schm_version_1).unwrap()),
            ..protection_scheme_info()
        };

        assert_eq!(
            encoded(&kept_schm),
            [b"\0\0\0\x39sinf".as_slice(), FRMA, SCHI, schm_version_1].concat()
        );
    }
}
