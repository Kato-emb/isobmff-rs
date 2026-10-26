//! [`CompleteTrackInfoBox`], the original format of an incomplete track

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::frma::OriginalFormatBox;

/// [`CompleteTrackInfoBox`] (`cinf`), ISO/IEC 14496-12 §13.6.3
///
/// Holds, in its `OriginalFormatBox`, the sample entry format of the complete
/// track that was transformed into the incomplete track whose sample entry
/// holds the box, and optional boxes that tell how to process its samples.
///
/// The box writes `frma` first, whatever order it was read in: the order of
/// its syntax. The optional boxes are kept in
/// [`other_boxes`](Self::other_boxes), and written back after it in the
/// order they were read in. A box without a `frma` that reads is refused.
#[doc(alias = "cinf")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct CompleteTrackInfoBox {
    /// Sample entry format of the complete track, `original_format` in the syntax
    pub frma: OriginalFormatBox,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for CompleteTrackInfoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"cinf");
}

impl BoxDecode for CompleteTrackInfoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            frma: children.take_exactly_one()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for CompleteTrackInfoBox {
    fn payload_len(&self) -> u64 {
        self.other_boxes
            .as_slice()
            .iter()
            .map(AnyBox::encoded_len)
            .fold(self.frma.encoded_len(), u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.frma.encode(writer.take_remainder())?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::CompleteTrackInfoBox;
    use crate::encoded;
    use crate::frma::OriginalFormatBox;

    /// `frma` of the original format `avc1`
    const FRMA: &[u8] = b"\0\0\0\x0cfrmaavc1";

    /// Box that tells how to process the samples, of a type `cinf` holds no field for
    const XTRA: &[u8] = b"\0\0\0\x09xtra\x01";

    /// `cinf` of an `frma` of the original format `avc1`
    const CINF: &[u8] = b"\0\0\0\x14cinf\0\0\0\x0cfrmaavc1";

    fn complete_track_info() -> CompleteTrackInfoBox {
        CompleteTrackInfoBox {
            frma: OriginalFormatBox {
                data_format: FourCc::new(*b"avc1"),
            },
            other_boxes: OtherBoxes::new(),
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

    #[test]
    fn writes_the_original_format_before_the_boxes_it_read_first() {
        let with_box = CompleteTrackInfoBox {
            other_boxes: OtherBoxes::from(ChildBoxes::decode(XTRA).unwrap()),
            ..complete_track_info()
        };

        assert_eq!(
            CompleteTrackInfoBox::decode(&[b"\0\0\0\x1dcinf".as_slice(), XTRA, FRMA].concat()),
            Ok((with_box.clone(), b"".as_slice()))
        );
        assert_eq!(
            encoded(&with_box),
            [b"\0\0\0\x1dcinf".as_slice(), FRMA, XTRA].concat()
        );
    }

    #[test]
    fn a_box_without_an_original_format_is_refused() {
        assert_eq!(
            CompleteTrackInfoBox::decode_payload(XTRA),
            Err(Error::missing_mandatory_box(OriginalFormatBox::BOX_TYPE))
        );
    }
}
