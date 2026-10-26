//! [`SubTrackSampleGroupBox`], the sample groups that make up a sub track

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{TableEntry, entries_len, read_entries, write_entries};

/// [`SubTrackSampleGroupBox`] (`stsg`), ISO/IEC 14496-12 §8.13.6
///
/// Defines a sub track as the samples of one or more sample groups of one
/// sample grouping, each named by the index of its sample group description
/// entry.
///
/// `item_count` is the length of `group_description_indices`: a box of more
/// indices than 16 bits count is refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `item_count`.
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
///
/// [`Default`] is the box of `grouping_type` four zero bytes, of no sample
/// group.
#[doc(alias = "stsg")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SubTrackSampleGroupBox {
    /// Sample grouping of the sample groups, as in its `sbgp` and `sgpd`
    pub grouping_type: FourCc,
    /// Index of the sample group description entry of each sample group of the sub track
    pub group_description_indices: Vec<u32>,
}

impl Default for SubTrackSampleGroupBox {
    fn default() -> Self {
        Self {
            grouping_type: FourCc::new([0; 4]),
            group_description_indices: Vec::new(),
        }
    }
}

impl BoxDefinition for SubTrackSampleGroupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsg");
}

impl BoxDecode for SubTrackSampleGroupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let grouping_type = FourCc::new(*reader.read_bytes::<4>()?);
        let item_count = reader.read_u16()?;

        Ok(Self {
            grouping_type,
            group_description_indices: read_entries(
                reader,
                u32::from(item_count),
                u32::ENCODED_LEN,
                u32::read,
            )?,
        })
    }
}

impl BoxEncode for SubTrackSampleGroupBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.group_description_indices).saturating_add(4 + 4 + 2)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let item_count = u16::try_from(self.group_description_indices.len())
            .map_err(|_| Error::invalid_value("item_count"))?;

        write_version_zero(writer)?;
        writer.write_bytes(self.grouping_type.as_bytes())?;
        writer.write_u16(item_count)?;
        write_entries(writer, &self.group_description_indices)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FourCc};

    use super::SubTrackSampleGroupBox;
    use crate::encoded;

    /// `stsg` of the `tele` sample groups of the entries 1 and 2
    const STSG: &[u8] = b"\0\0\0\x1astsg\0\0\0\0tele\0\x02\0\0\0\x01\0\0\0\x02";

    fn sub_track_sample_group() -> SubTrackSampleGroupBox {
        SubTrackSampleGroupBox {
            grouping_type: FourCc::new(*b"tele"),
            group_description_indices: vec![1, 2],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SubTrackSampleGroupBox::decode(STSG),
            Ok((sub_track_sample_group(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&sub_track_sample_group()), STSG);
    }

    #[test]
    fn a_count_of_more_indices_than_the_payload_holds_is_refused() {
        assert_eq!(
            SubTrackSampleGroupBox::decode_payload(b"\0\0\0\0tele\0\x02\0\0\0\x01"),
            Err(Error::truncated_payload(18, 14))
        );
    }

    #[test]
    fn more_indices_than_16_bits_count_are_refused_when_written() {
        let mut too_many = sub_track_sample_group();
        too_many.group_description_indices = vec![1; 0x1_0000];

        assert_eq!(
            too_many.encode(&mut vec![0; 0x4_0020]),
            Err(Error::invalid_value("item_count"))
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            SubTrackSampleGroupBox::decode_payload(b"\x01\0\0\0tele\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
