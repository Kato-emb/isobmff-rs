//! [`SampleAuxiliaryInformationOffsetsBox`]

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxHeader,
};

use crate::full_box::version_width;
use crate::saiz::AuxInfoType;
use crate::table::{read_entries, table_len, write_entry_count};

/// [`SampleAuxiliaryInformationOffsetsBox`] (`saio`), ISO/IEC 14496-12 §8.7.9
///
/// The position of the sample auxiliary information of one type and stream,
/// for each chunk or track run, or for all of them where the box holds one
/// offset and the information is contiguous. In a sample table an offset
/// counts as the chunk offsets of the samples do; in a track fragment, from
/// the base offset its track fragment header establishes.
///
/// The box is written in the `version` it holds, 32-bit offsets under version
/// 0 and 64-bit ones under version 1. An offset that version 0 does not carry
/// is refused when the box is written, with
/// [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange).
#[doc(alias = "saio")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleAuxiliaryInformationOffsetsBox {
    /// Version of the box: 0 for 32-bit offsets, 1 for 64-bit ones
    pub version: u8,
    /// Type and stream of the information, which flag `0x1` says is there
    pub aux_info_type: Option<AuxInfoType>,
    /// `offset` of the information of every chunk or track run, in order
    pub offsets: Vec<u64>,
}

impl BoxDefinition for SampleAuxiliaryInformationOffsetsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"saio");
}

impl BoxDecode for SampleAuxiliaryInformationOffsetsBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let version = header.version();
        let width = version_width(version)?;
        let offset_len = if width == FieldWidth::Compact { 4 } else { 8 };
        let aux_info_type = AuxInfoType::read_if_flagged(header, reader)?;
        let entry_count = reader.read_u32()?;
        let offsets = read_entries(reader, entry_count, offset_len, |reader| {
            reader.read_unsigned(width)
        })?;

        Ok(Self {
            version,
            aux_info_type,
            offsets,
        })
    }
}

impl BoxEncode for SampleAuxiliaryInformationOffsetsBox {
    fn payload_len(&self) -> u64 {
        let offset_len = if self.version == 0 { 4 } else { 8 };

        table_len(self.offsets.len(), offset_len)
            .saturating_add(AuxInfoType::header_len(self.aux_info_type))
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;

        AuxInfoType::write_with_header(self.aux_info_type, self.version, writer)?;
        write_entry_count(writer, self.offsets.len())?;
        self.offsets
            .iter()
            .try_for_each(|offset| writer.write_unsigned(width, *offset))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth, FourCc};

    use super::SampleAuxiliaryInformationOffsetsBox;
    use crate::{AuxInfoType, encoded};

    /// `saio` of version 0 and no type, of one offset `0x200`
    const SAIO_0: &[u8] = b"\0\0\0\x14saio\0\0\0\0\0\0\0\x01\0\0\x02\0";

    /// `saio` of version 1 and type `cenc`, parameter 0, of offsets `0x200` and `0x1_0000_0000`
    const SAIO_1: &[u8] = b"\0\0\0\x28saio\x01\0\0\x01cenc\0\0\0\0\0\0\0\x02\
        \0\0\0\0\0\0\x02\0\0\0\0\x01\0\0\0\0";

    fn compact() -> SampleAuxiliaryInformationOffsetsBox {
        SampleAuxiliaryInformationOffsetsBox {
            version: 0,
            aux_info_type: None,
            offsets: vec![0x200],
        }
    }

    fn extended() -> SampleAuxiliaryInformationOffsetsBox {
        SampleAuxiliaryInformationOffsetsBox {
            version: 1,
            aux_info_type: Some(AuxInfoType {
                aux_info_type: FourCc::new(*b"cenc"),
                aux_info_type_parameter: 0,
            }),
            offsets: vec![0x200, 0x1_0000_0000],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                SampleAuxiliaryInformationOffsetsBox::decode(SAIO_0),
                SampleAuxiliaryInformationOffsetsBox::decode(SAIO_1)
            ),
            (
                Ok((compact(), b"".as_slice())),
                Ok((extended(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (encoded(&compact()), encoded(&extended())),
            (SAIO_0.to_vec(), SAIO_1.to_vec())
        );
    }

    #[test]
    fn an_offset_version_0_does_not_carry_is_refused_when_written() {
        let mut wide = compact();
        wide.offsets = vec![0x1_0000_0000];

        assert_eq!(
            wide.encode(&mut [0; 20]),
            Err(Error::out_of_range(0x1_0000_0000, FieldWidth::Compact))
        );
    }

    #[test]
    fn a_count_of_more_offsets_than_the_payload_holds_is_refused() {
        assert_eq!(
            SampleAuxiliaryInformationOffsetsBox::decode_payload(b"\x01\0\0\0\0\0\0\x01\0\0\0\0"),
            Err(Error::truncated_payload(16, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SampleAuxiliaryInformationOffsetsBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
