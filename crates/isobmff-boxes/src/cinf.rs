//! [`CompleteTrackInfoBox`], the original format of an incomplete track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

/// [`CompleteTrackInfoBox`] (`cinf`), ISO/IEC 14496-12 §13.6.3
///
/// Holds, in its `OriginalFormatBox`, the sample entry format of the complete
/// track that was transformed into the incomplete track whose sample entry
/// holds the box, and optional boxes that tell how to process its samples.
///
/// This crate does not read the `OriginalFormatBox` yet: every child is kept
/// in [`other_boxes`](Self::other_boxes), and written back in the order it was
/// read in.
#[doc(alias = "cinf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct CompleteTrackInfoBox {
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for CompleteTrackInfoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"cinf");
}

impl BoxDecode for CompleteTrackInfoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(reader.take_remainder())?),
        })
    }
}

impl BoxEncode for CompleteTrackInfoBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, ChildBoxes, OtherBoxes};

    use super::CompleteTrackInfoBox;
    use crate::encoded;

    /// `frma` of the original format `avc1`
    const FRMA: &[u8] = b"\0\0\0\x0cfrmaavc1";

    /// `cinf` of an `frma` of the original format `avc1`
    const CINF: &[u8] = b"\0\0\0\x14cinf\0\0\0\x0cfrmaavc1";

    fn complete_track_info() -> CompleteTrackInfoBox {
        CompleteTrackInfoBox {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FRMA).unwrap()),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            CompleteTrackInfoBox::decode(CINF),
            Ok((complete_track_info(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&complete_track_info()), CINF);
    }
}
