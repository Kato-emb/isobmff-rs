//! [`MediaInformationBox`] (`minf`), ISO/IEC 14496-12 §8.4.4

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, RawBox, boxes,
};

use crate::dinf::DataInformationBox;
use crate::hmhd::HintMediaHeaderBox;
use crate::nmhd::NullMediaHeaderBox;
use crate::smhd::SoundMediaHeaderBox;
use crate::stbl::SampleTableBox;
use crate::sthd::SubtitleMediaHeaderBox;
use crate::vmhd::VideoMediaHeaderBox;

/// Box types the media information box takes its media header from
const MEDIA_HEADER_BOXES: &[BoxType] = &[
    VideoMediaHeaderBox::BOX_TYPE,
    SoundMediaHeaderBox::BOX_TYPE,
    HintMediaHeaderBox::BOX_TYPE,
    NullMediaHeaderBox::BOX_TYPE,
    SubtitleMediaHeaderBox::BOX_TYPE,
];

/// Header the kind of media a track carries puts in its `minf`
///
/// ISO/IEC 14496-12 §8.4.5. There is a different media information header for
/// each track type, and the one matching the handler is the one the `minf`
/// holds — so the slot takes any of these, and exactly one of them.
///
/// The variants are the headers ISO/IEC 14496-12 itself defines. §8.4.5 lets a
/// derived specification define one of its own, which no variant here names — a
/// `minf` headed by one of those states no header and keeps it among the
/// children no field claims.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MediaInformationHeader {
    /// Header of a video track, `vmhd`
    Video(VideoMediaHeaderBox),
    /// Header of an audio track, `smhd`
    Sound(SoundMediaHeaderBox),
    /// Header of a hint track, `hmhd`
    Hint(HintMediaHeaderBox),
    /// Header of a track no specific header names, `nmhd`
    Null(NullMediaHeaderBox),
    /// Header of a subtitle track, `sthd`
    Subtitle(SubtitleMediaHeaderBox),
}

impl MediaInformationHeader {
    /// Reads the header `child` holds, for a child of one of the five types
    fn decode(child: RawBox<'_>) -> Result<Self, Error> {
        let box_type = child.header().box_type();
        let payload = child.payload();

        if box_type == VideoMediaHeaderBox::BOX_TYPE {
            VideoMediaHeaderBox::decode_payload(payload).map(Self::Video)
        } else if box_type == SoundMediaHeaderBox::BOX_TYPE {
            SoundMediaHeaderBox::decode_payload(payload).map(Self::Sound)
        } else if box_type == HintMediaHeaderBox::BOX_TYPE {
            HintMediaHeaderBox::decode_payload(payload).map(Self::Hint)
        } else if box_type == NullMediaHeaderBox::BOX_TYPE {
            NullMediaHeaderBox::decode_payload(payload).map(Self::Null)
        } else {
            SubtitleMediaHeaderBox::decode_payload(payload).map(Self::Subtitle)
        }
        .map_err(|error| error.in_container(box_type))
    }

    /// Returns the length this header occupies, header and payload
    fn encoded_len(&self) -> u64 {
        match self {
            Self::Video(vmhd) => vmhd.encoded_len(),
            Self::Sound(smhd) => smhd.encoded_len(),
            Self::Hint(hmhd) => hmhd.encoded_len(),
            Self::Null(nmhd) => nmhd.encoded_len(),
            Self::Subtitle(sthd) => sthd.encoded_len(),
        }
    }

    /// Writes the header into the front of `buffer` and returns what is left
    fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error> {
        match self {
            Self::Video(vmhd) => vmhd.encode(buffer),
            Self::Sound(smhd) => smhd.encode(buffer),
            Self::Hint(hmhd) => hmhd.encode(buffer),
            Self::Null(nmhd) => nmhd.encode(buffer),
            Self::Subtitle(sthd) => sthd.encode(buffer),
        }
    }
}

/// Box that holds every declaration specific to the media of one track
///
/// [`MediaInformationBox`] (`minf`), ISO/IEC 14496-12 §8.4.4. All three children
/// the spec marks mandatory are promoted to fields: the media header the track's
/// kind selects, the `dinf` that says where the media data lives, and the `stbl`
/// that locates the samples within it.
///
/// The media header slot is filled when the header is one of the five ISO/IEC
/// 14496-12 defines. §8.4.5 lets a derived specification define a header of its
/// own, which no type here names — a `minf` headed by one of those keeps it
/// among [`other_boxes`](Self::other_boxes) and the slot reads back [`None`].
/// A box built with [`new`](Self::new) always states one of the five.
///
/// On encode the children are written in the order the spec lists them — the
/// media header, `dinf`, `stbl` — and then the children no field claims, so a
/// round-trip settles the order rather than preserving it. A header kept among
/// those children is written after the `stbl` with them.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{
///     ChunkOffsetBox, DataEntry, DataEntryUrlBox, DataInformationBox, DataReferenceBox,
///     MediaInformationBox, MediaInformationHeader, SampleDescriptionBox, SampleSizeBox,
///     SampleSizes, SampleTableBox, SampleToChunkBox, TimeToSampleBox, VideoMediaHeaderBox,
/// };
/// use isobmff_core::{BoxDecode, BoxEncode};
///
/// // A video track composing its image over what is under it
/// let media_header = MediaInformationHeader::Video(VideoMediaHeaderBox::new(0, [0; 3]));
///
/// // Its media data lies in the very file this box is written to
/// let data_information =
///     DataInformationBox::new(DataReferenceBox::new(vec![DataEntry::Url(
///         DataEntryUrlBox::new(None),
///     )]));
///
/// // Its samples are described in fragments, so these tables are empty
/// let sample_table = SampleTableBox::new(
///     SampleDescriptionBox::new(Vec::new()),
///     TimeToSampleBox::new(Vec::new()),
///     SampleToChunkBox::new(Vec::new()),
///     SampleSizeBox::new(SampleSizes::PerSample(Vec::new())),
///     ChunkOffsetBox::new(Vec::new()),
/// );
///
/// let media_information =
///     MediaInformationBox::new(media_header, data_information, sample_table);
///
/// // The header of the box and the three children the spec requires
/// assert_eq!(media_information.encoded_len(), 156);
///
/// // Writing it and reading it back gives the value that wrote it
/// let mut buffer = vec![0; usize::try_from(media_information.encoded_len()).unwrap()];
/// media_information.encode(&mut buffer).unwrap();
///
/// assert_eq!(MediaInformationBox::decode(&buffer).unwrap().0, media_information);
/// ```
#[doc(alias = "minf")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MediaInformationBox {
    media_information_header: Option<MediaInformationHeader>,
    dinf: DataInformationBox,
    stbl: SampleTableBox,
    other_boxes: OtherBoxes,
}

impl MediaInformationBox {
    /// Creates the box from the three declarations the spec requires
    #[must_use]
    pub const fn new(
        media_information_header: MediaInformationHeader,
        dinf: DataInformationBox,
        stbl: SampleTableBox,
    ) -> Self {
        Self {
            media_information_header: Some(media_information_header),
            dinf,
            stbl,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the header the kind of media this track carries states, or
    /// [`None`] when the box was read with a header no variant of
    /// [`MediaInformationHeader`] names
    #[must_use]
    pub const fn media_information_header(&self) -> Option<&MediaInformationHeader> {
        self.media_information_header.as_ref()
    }

    /// Returns where the media data of the track lies
    #[must_use]
    pub const fn dinf(&self) -> &DataInformationBox {
        &self.dinf
    }

    /// Returns the tables that locate and describe the track's samples
    #[must_use]
    pub const fn stbl(&self) -> &SampleTableBox {
        &self.stbl
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for MediaInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"minf");
}

impl BoxDecode for MediaInformationBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no
    ///   `dinf` or no `stbl`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one
    ///   `dinf`, more than one `stbl`, or more than one media header of one type.
    /// * [`DuplicateAlternativeBox`](isobmff_core::ErrorKind::DuplicateAlternativeBox):
    ///   media headers of two kinds, of which §8.4.5 has the box hold one.
    /// * Whatever a child reports, on the [`containers`](Error::containers) path: one
    ///   of them does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut media_header_boxes = Vec::new();
        let mut data_information_boxes = ChildBoxes::new();
        let mut sample_table_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if MEDIA_HEADER_BOXES.contains(&box_type) {
                media_header_boxes.push(child);
            } else if box_type == DataInformationBox::BOX_TYPE {
                data_information_boxes.push(child);
            } else if box_type == SampleTableBox::BOX_TYPE {
                sample_table_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        let media_information_header = match media_header_boxes.as_slice() {
            [] => None,
            [stated] => Some(MediaInformationHeader::decode(*stated)?),
            [first, rest @ ..] => {
                let box_type = first.header().box_type();
                let of_one_kind = rest
                    .iter()
                    .all(|other| other.header().box_type() == box_type);

                return Err(if of_one_kind {
                    Error::duplicate_box(box_type)
                } else {
                    Error::duplicate_alternative_box(MEDIA_HEADER_BOXES)
                });
            }
        };

        Ok(Self {
            media_information_header,
            dinf: data_information_boxes.exactly_one()?,
            stbl: sample_table_boxes.exactly_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for MediaInformationBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.media_information_header
            .as_ref()
            .map_or(0, MediaInformationHeader::encoded_len)
            .saturating_add(self.dinf.encoded_len())
            .saturating_add(self.stbl.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        if let Some(media_information_header) = &self.media_information_header {
            rest = media_information_header.encode(rest)?;
        }
        rest = self.dinf.encode(rest)?;
        rest = self.stbl.encode(rest)?;
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error};

    use super::{MEDIA_HEADER_BOXES, MediaInformationBox, MediaInformationHeader};
    use crate::dinf::tests::data_information;
    use crate::smhd::tests::sound_media_header;
    use crate::stbl::tests::sample_table;
    use crate::vmhd::tests::video_media_header;

    /// Media information of a video track whose data lies in the file it is read from
    pub(crate) fn media_information() -> MediaInformationBox {
        MediaInformationBox::new(
            MediaInformationHeader::Video(video_media_header()),
            data_information(),
            sample_table(),
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(media_information: &MediaInformationBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(media_information.payload_len()).unwrap()];
        media_information.encode_payload(&mut buffer).unwrap();

        buffer
    }

    /// Writes one child whole, header and payload, as it lies in this box
    fn encoded_child(child: &(impl BoxDefinition + BoxEncode)) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(child.encoded_len()).unwrap()];
        child.encode(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&media_information());

        assert_eq!(
            MediaInformationBox::decode_payload(&payload).unwrap(),
            media_information()
        );
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [
            encoded_payload(&media_information()),
            vec![0, 0, 0, 0x08, b'f', b'r', b'e', b'e'],
        ]
        .concat();

        let media_information = MediaInformationBox::decode_payload(&payload).unwrap();

        assert_eq!(
            media_information
                .other_boxes()
                .first()
                .map(AnyBox::box_type),
            Some(BoxType::compact(*b"free"))
        );
        assert_eq!(encoded_payload(&media_information), payload);
    }

    #[test]
    fn a_box_headed_by_a_media_header_no_variant_here_names_states_none_and_keeps_it() {
        let derived_media_header = vec![0, 0, 0, 0x0c, b'g', b'm', b'h', b'd', 0, 0, 0, 0];
        let payload = [
            derived_media_header.clone(),
            encoded_child(&data_information()),
            encoded_child(&sample_table()),
        ]
        .concat();

        let media_information = MediaInformationBox::decode_payload(&payload).unwrap();

        assert_eq!(media_information.media_information_header(), None);
        assert_eq!(
            media_information
                .other_boxes()
                .first()
                .map(AnyBox::box_type),
            Some(BoxType::compact(*b"gmhd"))
        );
        assert_eq!(
            encoded_payload(&media_information),
            [
                encoded_child(&data_information()),
                encoded_child(&sample_table()),
                derived_media_header,
            ]
            .concat()
        );
    }

    #[test]
    fn a_box_holding_media_headers_of_two_kinds_is_rejected() {
        let payload = [
            encoded_payload(&media_information()),
            encoded_child(&sound_media_header()),
        ]
        .concat();

        assert_eq!(
            MediaInformationBox::decode_payload(&payload),
            Err(Error::duplicate_alternative_box(MEDIA_HEADER_BOXES))
        );
    }

    #[test]
    fn a_second_media_header_of_one_kind_is_rejected() {
        let payload = [
            encoded_payload(&media_information()),
            encoded_child(&video_media_header()),
        ]
        .concat();

        assert_eq!(
            MediaInformationBox::decode_payload(&payload),
            Err(Error::duplicate_box(BoxType::compact(*b"vmhd")))
        );
    }

    #[test]
    fn a_box_missing_one_of_the_children_it_must_hold_is_rejected() {
        let children = [
            (
                BoxType::compact(*b"dinf"),
                encoded_child(&data_information()),
            ),
            (BoxType::compact(*b"stbl"), encoded_child(&sample_table())),
        ];

        for (missing, _) in &children {
            let payload: Vec<u8> = [(
                BoxType::compact(*b"vmhd"),
                encoded_child(&video_media_header()),
            )]
            .iter()
            .chain(&children)
            .filter(|(box_type, _)| box_type != missing)
            .flat_map(|(_, bytes)| bytes.clone())
            .collect();

            assert_eq!(
                MediaInformationBox::decode_payload(&payload),
                Err(Error::missing_mandatory_box(*missing))
            );
        }
    }
}
