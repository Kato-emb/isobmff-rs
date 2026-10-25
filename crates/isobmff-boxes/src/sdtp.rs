//! [`SampleDependencyTypeBox`] and the [`SampleDependencyTypeEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{TableEntry, entries_len, read_to_end, write_entries};

/// Dependencies of one sample, one entry of a [`SampleDependencyTypeBox`]
///
/// Each field is a 2-bit value, 0 where it is unknown. A value wider than 2
/// bits is refused when the entry is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field.
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleDependencyTypeEntry {
    /// 1: a leading sample with a dependency before the referenced I-picture,
    /// so not decodable; 2: not a leading sample; 3: a leading sample with no
    /// such dependency, so decodable
    pub is_leading: u8,
    /// 1: the sample depends on others (not an I picture); 2: it does not (an I
    /// picture); 3 is reserved
    pub sample_depends_on: u8,
    /// 1: other samples may depend on this one (not disposable); 2: no other
    /// sample does (disposable); 3 is reserved
    pub sample_is_depended_on: u8,
    /// 1: the sample holds redundant coding; 2: it holds none; 3 is reserved
    pub sample_has_redundancy: u8,
}

impl TableEntry for SampleDependencyTypeEntry {
    const ENCODED_LEN: u64 = 1;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [byte] = *reader.read_bytes::<1>()?;

        Ok(Self {
            is_leading: byte >> 6,
            sample_depends_on: byte >> 4 & 0b11,
            sample_is_depended_on: byte >> 2 & 0b11,
            sample_has_redundancy: byte & 0b11,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let fields = [
            (self.is_leading, "is_leading"),
            (self.sample_depends_on, "sample_depends_on"),
            (self.sample_is_depended_on, "sample_is_depended_on"),
            (self.sample_has_redundancy, "sample_has_redundancy"),
        ];
        if let Some((_, field)) = fields.iter().find(|(value, _)| *value > 0b11) {
            return Err(Error::invalid_value(field));
        }

        writer.write_bytes(&[self.is_leading << 6
            | self.sample_depends_on << 4
            | self.sample_is_depended_on << 2
            | self.sample_has_redundancy])
    }
}

/// [`SampleDependencyTypeBox`] (`sdtp`), ISO/IEC 14496-12 §8.6.4
///
/// For each sample, whether it depends on others, whether others depend on
/// it, whether it holds redundant coding, and whether it is a leading sample.
///
/// The box states no count: its table runs to the end of the payload, one
/// entry per byte, as FFmpeg reads it. The specification takes the number of
/// entries from the `sample_count` of the sample size box.
#[doc(alias = "sdtp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleDependencyTypeBox {
    /// Dependencies of every sample, in sample order
    pub entries: Vec<SampleDependencyTypeEntry>,
}

impl BoxDefinition for SampleDependencyTypeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sdtp");
}

impl BoxDecode for SampleDependencyTypeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            entries: read_to_end(reader)?,
        })
    }
}

impl BoxEncode for SampleDependencyTypeBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.entries).saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        write_entries(writer, &self.entries)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{SampleDependencyTypeBox, SampleDependencyTypeEntry};
    use crate::encoded;

    /// `sdtp` of an I picture others depend on, and a disposable leading sample that is decodable
    const SDTP: &[u8] = b"\0\0\0\x0esdtp\0\0\0\0\xa6\xd8";

    fn sample_dependency_type() -> SampleDependencyTypeBox {
        SampleDependencyTypeBox {
            entries: vec![
                SampleDependencyTypeEntry {
                    is_leading: 2,
                    sample_depends_on: 2,
                    sample_is_depended_on: 1,
                    sample_has_redundancy: 2,
                },
                SampleDependencyTypeEntry {
                    is_leading: 3,
                    sample_depends_on: 1,
                    sample_is_depended_on: 2,
                    sample_has_redundancy: 0,
                },
            ],
        }
    }

    #[test]
    fn reads_an_entry_from_every_byte_to_the_end_of_the_payload() {
        assert_eq!(
            SampleDependencyTypeBox::decode(SDTP),
            Ok((sample_dependency_type(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&sample_dependency_type()), SDTP);
    }

    #[test]
    fn a_value_wider_than_2_bits_is_refused_when_written() {
        let mut wide = sample_dependency_type();
        wide.entries.last_mut().unwrap().sample_is_depended_on = 4;

        assert_eq!(
            wide.encode(&mut [0; 14]),
            Err(Error::invalid_value("sample_is_depended_on"))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_version_and_flags_is_refused() {
        assert_eq!(
            SampleDependencyTypeBox::decode_payload(b"\0\0"),
            Err(Error::truncated_payload(4, 2))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SampleDependencyTypeBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
