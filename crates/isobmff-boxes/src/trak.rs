//! [`TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, ChildBoxes, Error,
    FieldReader, FieldWriter, OtherBoxes, boxes,
};

use crate::mdia::MediaBox;
use crate::tkhd::TrackHeaderBox;

/// Box that holds everything declaring one track
///
/// [`TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1. Both children the spec marks
/// mandatory are promoted to fields. An `edts`, which maps the track's media
/// onto the movie's timeline, has no fields yet and is kept in
/// [`other_boxes`](Self::other_boxes) — so the times this crate reports are the
/// media's own, with no edit list applied.
#[doc(alias = "trak")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct TrackBox {
    tkhd: TrackHeaderBox,
    mdia: MediaBox,
    other_boxes: OtherBoxes,
}

impl TrackBox {
    /// Creates the box from the two declarations the spec requires
    #[must_use]
    pub const fn new(tkhd: TrackHeaderBox, mdia: MediaBox) -> Self {
        Self {
            tkhd,
            mdia,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the declarations the track applies as a whole
    #[must_use]
    pub const fn tkhd(&self) -> &TrackHeaderBox {
        &self.tkhd
    }

    /// Returns everything declaring the media the track carries
    #[must_use]
    pub const fn mdia(&self) -> &MediaBox {
        &self.mdia
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for TrackBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trak");
}

impl BoxDecode for TrackBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `tkhd` or
    ///   `mdia`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one of either.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: one of them
    ///   does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut tkhd_boxes = ChildBoxes::new();
        let mut mdia_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == TrackHeaderBox::BOX_TYPE {
                tkhd_boxes.push(child);
            } else if box_type == MediaBox::BOX_TYPE {
                mdia_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            tkhd: tkhd_boxes.exactly_one()?,
            mdia: mdia_boxes.exactly_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for TrackBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.tkhd
            .encoded_len()
            .saturating_add(self.mdia.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.tkhd.encode(writer.take_remainder())?;
        rest = self.mdia.encode(rest)?;
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{
        BoxDecode, BoxEncode, BoxType, Error, FourCC, FullBoxFlags, LanguageCode, Mp4EpochSeconds,
        NullTerminatedString,
    };

    use super::TrackBox;
    use crate::hdlr::HandlerBox;
    use crate::mdhd::MediaHeaderBox;
    use crate::mdia::MediaBox;
    use crate::minf::MediaInformationBox;
    use crate::stbl::SampleTableBox;
    use crate::stsd::SampleDescriptionBox;
    use crate::tkhd::TrackHeaderBox;

    /// Track box of a video track, with every mandatory child in place
    pub(crate) fn track() -> TrackBox {
        TrackBox::new(
            TrackHeaderBox::new(
                FullBoxFlags::new(0x3).unwrap(),
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                1,
                90_000,
            ),
            MediaBox::new(
                MediaHeaderBox::new(
                    Mp4EpochSeconds::from_seconds(0),
                    Mp4EpochSeconds::from_seconds(0),
                    90_000,
                    90_000,
                    LanguageCode::UND,
                ),
                HandlerBox::new(
                    FourCC::new(*b"vide"),
                    NullTerminatedString::new(String::from("VideoHandler")).unwrap(),
                ),
                MediaInformationBox::new(SampleTableBox::new(
                    SampleDescriptionBox::new(Vec::new()),
                )),
            ),
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(track: &TrackBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(track.payload_len()).unwrap()];
        track.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&track());

        assert_eq!(TrackBox::decode_payload(&payload).unwrap(), track());
    }

    #[test]
    fn a_box_holding_only_a_track_header_is_rejected() {
        let whole = encoded_payload(&track());
        let track_header_len = usize::try_from(track().tkhd().payload_len()).unwrap() + 8;

        assert_eq!(
            TrackBox::decode_payload(whole.get(..track_header_len).unwrap()),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"mdia")))
        );
    }
}
