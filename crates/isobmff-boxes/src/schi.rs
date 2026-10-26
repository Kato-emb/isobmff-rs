//! [`SchemeInformationBox`], the information a protection or a restriction scheme needs

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

/// [`SchemeInformationBox`] (`schi`), ISO/IEC 14496-12 §13.4.7
///
/// The boxes the scheme a `SchemeTypeBox` declares needs, whose types and
/// formats that scheme defines. They are kept in
/// [`other_boxes`](Self::other_boxes), in the order they came, and written
/// back in it.
#[doc(alias = "schi")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SchemeInformationBox {
    /// Boxes of the scheme, `scheme_specific_data` in the syntax
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for SchemeInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"schi");
}

impl BoxDecode for SchemeInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(reader.take_remainder())?),
        })
    }
}

impl BoxEncode for SchemeInformationBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, ChildBoxes, OtherBoxes};

    use super::SchemeInformationBox;
    use crate::encoded;

    /// Two boxes of a scheme, of types this crate does not read
    const SCHEME_BOXES: &[u8] = b"\0\0\0\x09tenc\x01\0\0\0\x08xtra";

    /// `schi` of the two boxes
    const SCHI: &[u8] = b"\0\0\0\x19schi\0\0\0\x09tenc\x01\0\0\0\x08xtra";

    fn scheme_information() -> SchemeInformationBox {
        SchemeInformationBox {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(SCHEME_BOXES).unwrap()),
        }
    }

    #[test]
    fn reads_the_boxes_of_the_scheme_in_the_order_they_came() {
        assert_eq!(
            SchemeInformationBox::decode(SCHI),
            Ok((scheme_information(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_boxes_of_the_scheme_in_the_order_they_came() {
        assert_eq!(encoded(&scheme_information()), SCHI);
    }
}
