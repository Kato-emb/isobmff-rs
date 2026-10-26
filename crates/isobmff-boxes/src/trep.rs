//! [`TrackExtensionPropertiesBox`], the characteristics of a track in the movie fragments

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::assp::AlternativeStartupSequencePropertiesBox;
use crate::full_box::{read_version_zero, write_version_zero};

/// [`TrackExtensionPropertiesBox`] (`trep`), ISO/IEC 14496-12 §8.8.15
///
/// Documents or summarizes characteristics of one track in the movie
/// fragments after the movie, in the boxes it holds.
///
/// The box writes `assp` after `track_ID`, and then the children of other
/// types, which are kept in [`other_boxes`](Self::other_boxes). An `assp`
/// that does not read is kept there too, and reads back [`None`].
#[doc(alias = "trep")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct TrackExtensionPropertiesBox {
    /// `track_ID` of the track the properties are for
    pub track_id: u32,
    /// Properties of the alternative startup sequence sample groups of the track
    pub assp: Option<AlternativeStartupSequencePropertiesBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for TrackExtensionPropertiesBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trep");
}

impl BoxDecode for TrackExtensionPropertiesBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let track_id = reader.read_u32()?;
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            track_id,
            assp: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for TrackExtensionPropertiesBox {
    fn payload_len(&self) -> u64 {
        [4 + 4, self.assp.as_ref().map_or(0, BoxEncode::encoded_len)]
            .into_iter()
            .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u32(self.track_id)?;
        let mut rest = writer.take_remainder();
        if let Some(assp) = &self.assp {
            rest = assp.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, ChildBoxes, Error, OtherBoxes};

    use super::TrackExtensionPropertiesBox;
    use crate::assp::{AlternativeStartupSequenceOffsets, AlternativeStartupSequencePropertiesBox};
    use crate::encoded;

    /// `assp` of version 0 and a bound of -2
    const ASSP: &[u8] = b"\0\0\0\x10assp\0\0\0\0\xff\xff\xff\xfe";

    /// `free` holding nothing, a box `trep` holds no field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn properties() -> TrackExtensionPropertiesBox {
        TrackExtensionPropertiesBox {
            track_id: 2,
            assp: Some(AlternativeStartupSequencePropertiesBox {
                offsets: AlternativeStartupSequenceOffsets::Unparameterized {
                    min_initial_alt_startup_offset: -2,
                },
            }),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn reads_its_track_and_the_children_that_follow() {
        let bytes = [b"\0\0\0\x28trep\0\0\0\0\0\0\0\x02".as_slice(), FREE, ASSP].concat();

        assert_eq!(
            TrackExtensionPropertiesBox::decode(&bytes),
            Ok((properties(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_track_then_assp_then_the_child_it_has_no_field_for() {
        assert_eq!(
            encoded(&properties()),
            [b"\0\0\0\x28trep\0\0\0\0\0\0\0\x02".as_slice(), ASSP, FREE].concat()
        );
    }

    #[test]
    fn an_assp_that_does_not_read_is_kept_as_its_bytes() {
        let unread = b"\0\0\0\x0cassp\x02\0\0\0";
        let bytes = [b"\0\0\0\x1ctrep\0\0\0\0\0\0\0\x02".as_slice(), unread].concat();
        let (read, _) = TrackExtensionPropertiesBox::decode(&bytes).unwrap();

        assert_eq!((encoded(&read), read.assp), (bytes, None));
    }

    #[test]
    fn a_payload_ending_inside_the_track_is_refused() {
        assert_eq!(
            TrackExtensionPropertiesBox::decode_payload(b"\0\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            TrackExtensionPropertiesBox::decode_payload(b"\x01\0\0\0\0\0\0\x02"),
            Err(Error::unsupported_version(1))
        );
    }
}
