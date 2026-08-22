//! [`DataInformationBox`] (`dinf`), ISO/IEC 14496-12 §8.7.1

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, boxes,
};

use crate::dref::DataReferenceBox;

/// Box that declares where the media data of a track lies
///
/// [`DataInformationBox`] (`dinf`), ISO/IEC 14496-12 §8.7.1. The `dref` it must
/// hold is promoted to a field of its own, and every other child is kept in
/// [`other_boxes`](Self::other_boxes) and written back unread.
#[doc(alias = "dinf")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct DataInformationBox {
    dref: DataReferenceBox,
    other_boxes: OtherBoxes,
}

impl DataInformationBox {
    /// Creates the box from the table of places the media data lies
    #[must_use]
    pub const fn new(dref: DataReferenceBox) -> Self {
        Self {
            dref,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the table of places the media data of the track lies
    #[must_use]
    pub const fn dref(&self) -> &DataReferenceBox {
        &self.dref
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for DataInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dinf");
}

impl BoxDecode for DataInformationBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `dref`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one `dref`.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: the
    ///   `dref` does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut data_reference_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            if child.header().box_type() == DataReferenceBox::BOX_TYPE {
                data_reference_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            dref: data_reference_boxes.exactly_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for DataInformationBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.dref.encoded_len().saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.dref.encode(writer.take_remainder())?;
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error};

    use super::DataInformationBox;
    use crate::dref::tests::data_reference;

    /// Data information of a track whose data lies in the file it is read from
    pub(crate) fn data_information() -> DataInformationBox {
        DataInformationBox::new(data_reference())
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(data_information: &DataInformationBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(data_information.payload_len()).unwrap()];
        data_information.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&data_information());

        assert_eq!(
            DataInformationBox::decode_payload(&payload).unwrap(),
            data_information()
        );
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [
            encoded_payload(&data_information()),
            vec![0, 0, 0, 0x08, b'f', b'r', b'e', b'e'],
        ]
        .concat();

        let data_information = DataInformationBox::decode_payload(&payload).unwrap();

        assert_eq!(
            data_information.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"free"))
        );
        assert_eq!(encoded_payload(&data_information), payload);
    }

    #[test]
    fn a_box_holding_no_data_reference_is_rejected() {
        assert_eq!(
            DataInformationBox::decode_payload(b""),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"dref")))
        );
    }
}
