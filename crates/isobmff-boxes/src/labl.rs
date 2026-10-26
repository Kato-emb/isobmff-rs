//! [`LabelBox`], a textual description of the entity that holds it

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, Utf8String,
};

use crate::full_box::read_version_zero;

/// Flag mask of `is_group_label`
const IS_GROUP_LABEL: u8 = 0x01;

/// [`LabelBox`] (`labl`), ISO/IEC 14496-12 §8.10.5
///
/// Textual description, in one language, of the entity that holds the box,
/// such as a track, which a player may offer as a choice. Labels of the same
/// `label_id` form a label group.
///
/// The box reads version 0 alone. Of the `flags`, the one the specification
/// defines is held, and the others are read past and written as zero.
#[doc(alias = "labl")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct LabelBox {
    /// Whether `label` is the summary, or title, of every label of the same `label_id`
    pub is_group_label: bool,
    /// Label group of the label, unique among the labels of the file, or 0 for a label of no group
    pub label_id: u16,
    /// IETF BCP 47 language tag of the language of `label`, such as `en-US`
    pub language: Utf8String,
    /// Textual description
    pub label: Utf8String,
}

impl BoxDefinition for LabelBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"labl");
}

impl BoxDecode for LabelBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [.., flags] = read_version_zero(reader)?.bits().to_be_bytes();
        let label_id = reader.read_u16()?;
        let mut strings = reader.take_remainder().splitn(2, |byte| *byte == 0);

        Ok(Self {
            is_group_label: flags & IS_GROUP_LABEL != 0,
            label_id,
            language: Utf8String::from_slice(strings.next().unwrap_or_default()),
            label: Utf8String::from_slice(strings.next().unwrap_or_default()),
        })
    }
}

impl BoxEncode for LabelBox {
    fn payload_len(&self) -> u64 {
        self.language
            .encoded_len()
            .saturating_add(self.label.encoded_len())
            .saturating_add(4 + 2)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let flags = if self.is_group_label {
            IS_GROUP_LABEL
        } else {
            0
        };
        writer.write_bytes(&[0, 0, 0, flags])?;
        writer.write_u16(self.label_id)?;
        let rest = self.language.encode(writer.take_remainder())?;
        self.label.encode(rest)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{BoxDecode, Error, Utf8String};

    use super::LabelBox;
    use crate::encoded;

    /// `labl` of the group label 3, in `en-US`, `Commentary`
    const LABL: &[u8] = b"\0\0\0\x1flabl\0\0\0\x01\0\x03en-US\0Commentary\0";

    fn string(text: &str) -> Utf8String {
        Utf8String::new(String::from(text)).unwrap()
    }

    fn label() -> LabelBox {
        LabelBox {
            is_group_label: true,
            label_id: 3,
            language: string("en-US"),
            label: string("Commentary"),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(LabelBox::decode(LABL), Ok((label(), b"".as_slice())));
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&label()), LABL);
    }

    #[test]
    fn flags_the_specification_does_not_define_are_written_as_zero() {
        let bytes = b"\0\0\0\x1flabl\0\x80\0\x01\0\x03en-US\0Commentary\0";
        let (read, _) = LabelBox::decode(bytes).unwrap();

        assert_eq!((read.clone(), encoded(&read)), (label(), LABL.to_vec()));
    }

    #[test]
    fn a_payload_ending_inside_the_label_id_is_refused() {
        assert_eq!(
            LabelBox::decode_payload(b"\0\0\0\0\0"),
            Err(Error::truncated_payload(6, 5))
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            LabelBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
