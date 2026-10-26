//! [`TrackSelectionBox`], the switch group and the attributes of a track

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{entries_len, read_to_end, write_entries};

/// [`TrackSelectionBox`] (`tsel`), ISO/IEC 14496-12 §8.10.3
///
/// The switch group of a track, and the attributes that describe it or tell it
/// apart from the other tracks of its alternate or switch group. The box lies
/// in the `UserDataBox` of the track it describes.
///
/// The box reads version 0 alone. The `flags` are read past and written as
/// zero.
#[doc(alias = "tsel")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackSelectionBox {
    /// Switch group of the track, or 0 where the file tells nothing about switching to and from it
    pub switch_group: i32,
    /// Attributes of the track, such as `tesc` or `cdec`, to the end of the box
    pub attribute_list: Vec<FourCc>,
}

impl BoxDefinition for TrackSelectionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tsel");
}

impl BoxDecode for TrackSelectionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            switch_group: reader.read_i32()?,
            attribute_list: read_to_end(reader)?,
        })
    }
}

impl BoxEncode for TrackSelectionBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.attribute_list).saturating_add(4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_i32(self.switch_group)?;
        write_entries(writer, &self.attribute_list)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::TrackSelectionBox;
    use crate::encoded;

    /// `tsel` of switch group 2 and the attributes `cdec` and `bitr`
    const TSEL: &[u8] = b"\0\0\0\x18tsel\0\0\0\0\0\0\0\x02cdecbitr";

    fn track_selection() -> TrackSelectionBox {
        TrackSelectionBox {
            switch_group: 2,
            attribute_list: vec![FourCc::new(*b"cdec"), FourCc::new(*b"bitr")],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            TrackSelectionBox::decode(TSEL),
            Ok((track_selection(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&track_selection()), TSEL);
    }

    #[test]
    fn a_payload_ending_inside_an_attribute_is_refused() {
        assert_eq!(
            TrackSelectionBox::decode_payload(b"\0\0\0\0\0\0\0\x02cde"),
            Err(Error::truncated_payload(12, 11))
        );
    }

    #[test]
    fn a_version_other_than_0_is_refused() {
        assert_eq!(
            TrackSelectionBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
