//! [`DataInformationBox`], the container of the data reference of a track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::dref::DataReferenceBox;

/// [`DataInformationBox`] (`dinf`), ISO/IEC 14496-12 §8.7.1
///
/// Declares where the media information of a track lies, through the data
/// reference it holds.
///
/// Children of other types are kept in [`other_boxes`](Self::other_boxes),
/// and written back after `dref`.
#[doc(alias = "dinf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct DataInformationBox {
    /// Data reference of the track
    pub dref: DataReferenceBox,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for DataInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dinf");
}

impl BoxDecode for DataInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            dref: children.take_exactly_one()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for DataInformationBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(self.dref.encoded_len(), u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.dref.encode(writer.take_remainder())?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::DataInformationBox;
    use crate::data_entry::{DataEntry, DataEntryUrlBox};
    use crate::dref::DataReferenceBox;
    use crate::encoded;

    /// `dref` of one `url ` of media data in the same file
    const DREF: &[u8] = b"\0\0\0\x1cdref\0\0\0\0\0\0\0\x01\0\0\0\x0curl \0\0\0\x01";

    /// Box of a type `dinf` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn data_information() -> DataInformationBox {
        DataInformationBox {
            dref: DataReferenceBox {
                data_entries: vec![DataEntry::Url(DataEntryUrlBox::default())],
            },
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_data_reference_and_keeps_the_child_it_has_no_field_for() {
        let bytes = [b"\0\0\0\x2cdinf".as_slice(), FREE, DREF].concat();

        assert_eq!(
            DataInformationBox::decode(&bytes),
            Ok((data_information(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_data_reference_and_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&data_information()),
            [b"\0\0\0\x2cdinf".as_slice(), DREF, FREE].concat()
        );
    }

    #[test]
    fn a_box_without_a_data_reference_is_refused() {
        assert_eq!(
            DataInformationBox::decode_payload(FREE),
            Err(Error::missing_mandatory_box(DataReferenceBox::BOX_TYPE))
        );
    }
}
