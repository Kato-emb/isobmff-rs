//! A vendor box whose reading fails in a way of its own is read as a child like any box

#![cfg(feature = "alloc")]

#[cfg(test)]
mod tests {
    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxType, ChildBoxes, Error, FieldReader, InContainer, boxes,
    };

    /// Failure of the vendor crate: a box failure, or a checksum that disagrees
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum VendorError {
        Box(Error),
        ChecksumMismatch,
    }

    impl From<Error> for VendorError {
        fn from(box_error: Error) -> Self {
            Self::Box(box_error)
        }
    }

    impl InContainer for VendorError {
        fn in_container(self, container: BoxType) -> Self {
            match self {
                Self::Box(box_error) => Self::Box(box_error.in_container(container)),
                Self::ChecksumMismatch => Self::ChecksumMismatch,
            }
        }
    }

    /// Vendor box whose payload is a 16-bit value and its checksum, the value inverted
    #[derive(PartialEq, Eq, Debug)]
    struct CheckedValueBox(u16);

    impl BoxDefinition for CheckedValueBox {
        const BOX_TYPE: BoxType = BoxType::compact(*b"chkv");
    }

    impl BoxDecode for CheckedValueBox {
        type Error = VendorError;

        fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, VendorError> {
            let value = reader.read_u16()?;
            if reader.read_u16()? != !value {
                return Err(VendorError::ChecksumMismatch);
            }

            Ok(Self(value))
        }
    }

    /// Collects every box a payload holds
    fn collected(payload: &[u8]) -> ChildBoxes<'_> {
        boxes(payload).collect::<Result<_, _>>().unwrap()
    }

    #[test]
    fn a_child_with_a_failure_of_its_own_reads_as_any_child() {
        let mut children = collected(b"\0\0\0\x0cchkv\0\x07\xff\xf8");

        assert_eq!(
            children.take_exactly_one::<CheckedValueBox>(),
            Ok(CheckedValueBox(7))
        );
    }

    #[test]
    fn a_box_failure_of_the_child_names_it_on_the_path() {
        let mut truncated = collected(b"\0\0\0\x0achkv\0\x07");

        assert_eq!(
            truncated.take_exactly_one::<CheckedValueBox>(),
            Err(VendorError::Box(
                Error::truncated_payload(4, 2).in_container(CheckedValueBox::BOX_TYPE)
            ))
        );
    }

    #[test]
    fn a_failure_of_a_kind_of_its_own_passes_out_as_it_was() {
        let mut disagreeing = collected(b"\0\0\0\x0cchkv\0\x07\0\x07");

        assert_eq!(
            disagreeing.take_exactly_one::<CheckedValueBox>(),
            Err(VendorError::ChecksumMismatch)
        );
    }

    #[test]
    fn a_count_the_quantity_forbids_is_reported_as_the_failure_the_child_reports() {
        assert_eq!(
            ChildBoxes::new().take_exactly_one::<CheckedValueBox>(),
            Err(VendorError::Box(Error::missing_mandatory_box(
                CheckedValueBox::BOX_TYPE
            )))
        );
    }
}
