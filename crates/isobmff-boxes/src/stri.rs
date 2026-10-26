//! [`SubTrackInformationBox`], the groups and the attributes of a sub track

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{entries_len, read_to_end, write_entries};

/// [`SubTrackInformationBox`] (`stri`), ISO/IEC 14496-12 §8.13.4
///
/// The switch group and the alternate group of a sub track, its identifier
/// within the track, and the attributes that describe it or tell it apart
/// from the other tracks and sub tracks of its groups.
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
#[doc(alias = "stri")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SubTrackInformationBox {
    /// Switch group of the sub track, or 0 where the file tells nothing about switching to and from it
    pub switch_group: i16,
    /// Alternate group of the sub track, or 0 where the file tells nothing of its relations to other tracks and sub tracks
    pub alternate_group: i16,
    /// Identifier of the sub track, unique within the track, or 0 where none is assigned
    pub sub_track_id: u32,
    /// Attributes of the sub track, such as `tesc` or `bitr`, to the end of the box
    pub attribute_list: Vec<FourCc>,
}

impl BoxDefinition for SubTrackInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stri");
}

impl BoxDecode for SubTrackInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            switch_group: reader.read_i16()?,
            alternate_group: reader.read_i16()?,
            sub_track_id: reader.read_u32()?,
            attribute_list: read_to_end(reader)?,
        })
    }
}

impl BoxEncode for SubTrackInformationBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.attribute_list).saturating_add(4 + 2 + 2 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_i16(self.switch_group)?;
        writer.write_i16(self.alternate_group)?;
        writer.write_u32(self.sub_track_id)?;
        write_entries(writer, &self.attribute_list)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::SubTrackInformationBox;
    use crate::encoded;

    /// `stri` of switch group 1, alternate group 1, sub track 2 and the attribute `tesc`
    const STRI: &[u8] = b"\0\0\0\x18stri\0\0\0\0\0\x01\0\x01\0\0\0\x02tesc";

    fn sub_track_information() -> SubTrackInformationBox {
        SubTrackInformationBox {
            switch_group: 1,
            alternate_group: 1,
            sub_track_id: 2,
            attribute_list: vec![FourCc::new(*b"tesc")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SubTrackInformationBox::decode(STRI),
            Ok((sub_track_information(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&sub_track_information()), STRI);
    }

    #[test]
    fn a_payload_ending_inside_the_sub_track_id_is_refused() {
        assert_eq!(
            SubTrackInformationBox::decode_payload(b"\0\0\0\0\0\x01\0\x01\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            SubTrackInformationBox::decode_payload(b"\x01\0\0\0\0\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
