//! [`ExtendedTypeBox`] and the [`TypeCombinationBox`]es it holds

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, FourCc, OtherBoxes,
};

use crate::field::{entries_len, read_to_end, write_entries};

/// [`TypeCombinationBox`] (`tyco`), ISO/IEC 14496-12 §5.3
///
/// A combination of brands the file, segment, track or item complies with all
/// of at once.
#[doc(alias = "tyco")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TypeCombinationBox {
    /// Brands of the combination
    pub compatible_brands: Vec<FourCc>,
}

impl BoxDefinition for TypeCombinationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tyco");
}

impl BoxDecode for TypeCombinationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            compatible_brands: read_to_end(reader)?,
        })
    }
}

impl BoxEncode for TypeCombinationBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.compatible_brands)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_entries(writer, &self.compatible_brands)
    }
}

/// [`ExtendedTypeBox`] (`etyp`), ISO/IEC 14496-12 §5.3
///
/// Combinations of brands, of which a reader supports all the brands of at
/// least one to process what the box applies to — unless it supports a brand
/// of the type box before it.
///
/// Children of other types are kept in [`other_boxes`](Self::other_boxes),
/// and written back after the combinations.
///
/// The box holds one or more combinations. A box of none reads, but writing
/// it is refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue)
/// for `compatible_combinations`, so a box built from [`Default`] is given a
/// combination before it is written.
#[doc(alias = "etyp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ExtendedTypeBox {
    /// Combinations of brands, in the order they came
    pub compatible_combinations: Vec<TypeCombinationBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for ExtendedTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"etyp");
}

impl BoxDecode for ExtendedTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            compatible_combinations: children.take_zero_or_more()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for ExtendedTypeBox {
    fn payload_len(&self) -> u64 {
        self.compatible_combinations
            .iter()
            .map(BoxEncode::encoded_len)
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.compatible_combinations.is_empty() {
            return Err(Error::invalid_value("compatible_combinations"));
        }

        let mut rest = writer.take_remainder();
        for combination in &self.compatible_combinations {
            rest = combination.encode(rest)?;
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

    use isobmff_core::{BoxDecode, BoxEncode, BoxType, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::{ExtendedTypeBox, TypeCombinationBox};
    use crate::encoded;

    /// `tyco` of the brands `iso8` and `cmfc`
    const TYCO: &[u8] = b"\0\0\0\x10tycoiso8cmfc";

    /// `etyp` holding a `free` and then two `tyco`s, of `iso8` and `cmfc` and of `mif1`
    const ETYP: &[u8] = b"\0\0\0\x2cetyp\0\0\0\x08free\0\0\0\x10tycoiso8cmfc\0\0\0\x0ctycomif1";

    /// The same `etyp` written back, the combinations before the `free`
    const ETYP_WRITTEN: &[u8] =
        b"\0\0\0\x2cetyp\0\0\0\x10tycoiso8cmfc\0\0\0\x0ctycomif1\0\0\0\x08free";

    fn combination(brands: &[&[u8; 4]]) -> TypeCombinationBox {
        TypeCombinationBox {
            compatible_brands: brands.iter().map(|brand| FourCc::new(**brand)).collect(),
        }
    }

    fn extended_type() -> ExtendedTypeBox {
        ExtendedTypeBox {
            compatible_combinations: vec![
                combination(&[b"iso8", b"cmfc"]),
                combination(&[b"mif1"]),
            ],
            other_boxes: OtherBoxes::from(ChildBoxes::decode(b"\0\0\0\x08free").unwrap()),
        }
    }

    #[test]
    fn a_type_combination_reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            TypeCombinationBox::decode(TYCO),
            Ok((combination(&[b"iso8", b"cmfc"]), b"".as_slice()))
        );
    }

    #[test]
    fn a_type_combination_writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&combination(&[b"iso8", b"cmfc"])), TYCO);
    }

    #[test]
    fn a_type_combination_ending_inside_a_brand_is_refused() {
        assert_eq!(
            TypeCombinationBox::decode_payload(b"iso8cm"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn reads_its_combinations_in_order_and_keeps_the_child_it_has_no_field_for() {
        assert_eq!(
            ExtendedTypeBox::decode(ETYP),
            Ok((extended_type(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_combinations_and_then_the_child_it_has_no_field_for() {
        assert_eq!(encoded(&extended_type()), ETYP_WRITTEN);
    }

    #[test]
    fn a_box_of_no_combination_reads_but_is_refused_when_written() {
        let (read, _) = ExtendedTypeBox::decode(b"\0\0\0\x08etyp").unwrap();

        assert_eq!(
            (
                read.encode(&mut [0; 8]),
                ExtendedTypeBox::default().encode(&mut [0; 8])
            ),
            (
                Err(Error::invalid_value("compatible_combinations")),
                Err(Error::invalid_value("compatible_combinations"))
            )
        );
    }

    #[test]
    fn a_combination_that_does_not_read_names_itself_on_the_path_of_the_failure() {
        assert_eq!(
            ExtendedTypeBox::decode_payload(b"\0\0\0\x0atycois"),
            Err(Error::truncated_payload(4, 2).in_container(BoxType::compact(*b"tyco")))
        );
    }
}
