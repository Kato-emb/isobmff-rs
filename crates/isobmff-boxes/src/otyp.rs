//! [`OriginalFileTypeBox`], the brands of a file before it was transformed

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::etyp::ExtendedTypeBox;
use crate::ftyp::FileTypeBox;

/// [`OriginalFileTypeBox`] (`otyp`), ISO/IEC 14496-12 §8.16.5
///
/// Holds the [`FileTypeBox`] and the [`ExtendedTypeBox`] of a file before a
/// transformation of its box structure, such as compressing its top-level
/// boxes, made them no longer apply.
///
/// Children of other types — a nested `otyp` among them — are kept in
/// [`other_boxes`](Self::other_boxes), and written back after `ftyp` and
/// `etyp`. An `etyp` that does not read is kept there too, and `etyp` reads
/// back [`None`].
#[doc(alias = "otyp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct OriginalFileTypeBox {
    /// File type of the original file
    pub ftyp: FileTypeBox,
    /// Extended type of the original file
    pub etyp: Option<ExtendedTypeBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for OriginalFileTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"otyp");
}

impl BoxDecode for OriginalFileTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        // Why not a field for the nested `otyp`: reading it would recurse once
        // per level of nesting, which the input alone bounds, and a deep enough
        // input would overflow the stack.
        Ok(Self {
            ftyp: children.take_exactly_one()?,
            etyp: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for OriginalFileTypeBox {
    fn payload_len(&self) -> u64 {
        [
            self.ftyp.encoded_len(),
            self.etyp.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.ftyp.encode(writer.take_remainder())?;
        if let Some(etyp) = &self.etyp {
            rest = etyp.encode(rest)?;
        }
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

    use super::OriginalFileTypeBox;
    use crate::encoded;
    use crate::etyp::{ExtendedTypeBox, TypeCombinationBox};
    use crate::ftyp::FileTypeBox;

    /// `ftyp` of major brand `isom`, minor version 0 and the compatible brand `isom`
    const FTYP: &[u8] = b"\0\0\0\x14ftypisom\0\0\0\0isom";

    /// `etyp` of one `tyco`, of the brand `iso8`
    const ETYP: &[u8] = b"\0\0\0\x14etyp\0\0\0\x0ctycoiso8";

    /// `otyp` nested in the `otyp` that holds it, with an `ftyp` of its own
    const NESTED: &[u8] = b"\0\0\0\x1cotyp\0\0\0\x14ftypmp41\0\0\0\0mp41";

    fn file_type() -> FileTypeBox {
        FileTypeBox {
            major_brand: FourCc::new(*b"isom"),
            minor_version: 0,
            compatible_brands: vec![FourCc::new(*b"isom")],
        }
    }

    fn extended_type() -> ExtendedTypeBox {
        ExtendedTypeBox {
            compatible_combinations: vec![TypeCombinationBox {
                compatible_brands: vec![FourCc::new(*b"iso8")],
            }],
            other_boxes: OtherBoxes::new(),
        }
    }

    #[test]
    fn reads_its_file_type_and_extended_type_and_keeps_a_nested_original_file_type() {
        let payload = [NESTED, ETYP, FTYP].concat();

        assert_eq!(
            OriginalFileTypeBox::decode_payload(&payload),
            Ok(OriginalFileTypeBox {
                ftyp: file_type(),
                etyp: Some(extended_type()),
                other_boxes: OtherBoxes::from(ChildBoxes::decode(NESTED).unwrap()),
            })
        );
    }

    #[test]
    fn writes_its_file_type_then_its_extended_type_then_the_children_it_has_no_field_for() {
        let original_file_type = OriginalFileTypeBox {
            ftyp: file_type(),
            etyp: Some(extended_type()),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(NESTED).unwrap()),
        };

        assert_eq!(
            encoded(&original_file_type),
            [b"\0\0\0\x4cotyp".as_slice(), FTYP, ETYP, NESTED].concat()
        );
    }

    #[test]
    fn a_box_without_a_file_type_is_refused() {
        assert_eq!(
            OriginalFileTypeBox::decode_payload(ETYP),
            Err(Error::missing_mandatory_box(FileTypeBox::BOX_TYPE))
        );
    }

    #[test]
    fn an_extended_type_that_does_not_read_is_kept_among_the_other_children() {
        let unreadable = b"\0\0\0\x10etyp\0\0\0\x06tyco";
        let payload = [FTYP, unreadable].concat();

        assert_eq!(
            OriginalFileTypeBox::decode_payload(&payload),
            Ok(OriginalFileTypeBox {
                ftyp: file_type(),
                etyp: None,
                other_boxes: OtherBoxes::from(ChildBoxes::decode(unreadable).unwrap()),
            })
        );
    }
}
