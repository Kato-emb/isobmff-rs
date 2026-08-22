//! [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxWrite as _, ChildBoxes, Error,
    FieldReader, FieldWriter, OtherBoxes, boxes,
};

use crate::stco::ChunkOffsetBox;
use crate::stsc::SampleToChunkBox;
use crate::stsd::SampleDescriptionBox;
use crate::stsz::SampleSizeBox;
use crate::stts::TimeToSampleBox;

/// Box that holds every table locating and describing the samples of a track
///
/// [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1. The tables a track
/// that references data must state are promoted to fields of their own — the
/// sample descriptions, the decode timeline, the grouping into chunks, the
/// sample sizes, and the chunk offsets — and every other child is kept in
/// [`other_boxes`](Self::other_boxes) and written back unread.
///
/// Decoding asks for all five. §8.5.1 lets the `stbl` of a track that
/// references no data hold no children at all, and such a box does not decode
/// into this type — the raw walk still reads it. The variants that state two of
/// the tables otherwise, `stz2` for the sizes and `co64` for the offsets, have
/// no type here yet, so a `stbl` stating them that way fails as one holding no
/// `stsz` or no `stco`.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{
///     ChunkOffsetBox, SampleDescriptionBox, SampleSizeBox, SampleSizes, SampleTableBox,
///     SampleToChunkBox, TimeToSampleBox,
/// };
/// use isobmff_core::{BoxRead, BoxWrite};
///
/// // A fragmented movie describes its samples in its fragments, so these are empty
/// let sample_table = SampleTableBox::new(
///     SampleDescriptionBox::new(Vec::new()),
///     TimeToSampleBox::new(Vec::new()),
///     SampleToChunkBox::new(Vec::new()),
///     SampleSizeBox::new(SampleSizes::PerSample(Vec::new())),
///     ChunkOffsetBox::new(Vec::new()),
/// );
///
/// // The header of the box and five tables that count nothing
/// assert_eq!(sample_table.encoded_len(), 92);
///
/// // Writing it and reading it back gives the value that wrote it
/// let mut buffer = vec![0; usize::try_from(sample_table.encoded_len()).unwrap()];
/// sample_table.encode(&mut buffer).unwrap();
///
/// assert_eq!(SampleTableBox::decode(&buffer).unwrap().0, sample_table);
/// ```
#[doc(alias = "stbl")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct SampleTableBox {
    stsd: SampleDescriptionBox,
    stts: TimeToSampleBox,
    stsc: SampleToChunkBox,
    stsz: SampleSizeBox,
    stco: ChunkOffsetBox,
    other_boxes: OtherBoxes,
}

impl SampleTableBox {
    /// Creates the box from the tables that locate and describe the samples
    #[must_use]
    pub const fn new(
        stsd: SampleDescriptionBox,
        stts: TimeToSampleBox,
        stsc: SampleToChunkBox,
        stsz: SampleSizeBox,
        stco: ChunkOffsetBox,
    ) -> Self {
        Self {
            stsd,
            stts,
            stsc,
            stsz,
            stco,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Returns the description of the coding every sample was made with
    #[must_use]
    pub const fn stsd(&self) -> &SampleDescriptionBox {
        &self.stsd
    }

    /// Returns the decode time of every sample, stated as deltas
    #[must_use]
    pub const fn stts(&self) -> &TimeToSampleBox {
        &self.stts
    }

    /// Returns the chunk each sample lies in
    #[must_use]
    pub const fn stsc(&self) -> &SampleToChunkBox {
        &self.stsc
    }

    /// Returns how many bytes each sample occupies
    #[must_use]
    pub const fn stsz(&self) -> &SampleSizeBox {
        &self.stsz
    }

    /// Returns where every chunk of the track lies
    #[must_use]
    pub const fn stco(&self) -> &ChunkOffsetBox {
        &self.stco
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for SampleTableBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stbl");
}

impl BoxDecode for SampleTableBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no
    ///   `stsd`, `stts`, `stsc`, `stsz`, or `stco`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one of
    ///   any of them.
    /// * Whatever a child reports, on the [`containers`](Error::containers) path: one
    ///   of the tables does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut sample_description_boxes = ChildBoxes::new();
        let mut time_to_sample_boxes = ChildBoxes::new();
        let mut sample_to_chunk_boxes = ChildBoxes::new();
        let mut sample_size_boxes = ChildBoxes::new();
        let mut chunk_offset_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == SampleDescriptionBox::BOX_TYPE {
                sample_description_boxes.push(child);
            } else if box_type == TimeToSampleBox::BOX_TYPE {
                time_to_sample_boxes.push(child);
            } else if box_type == SampleToChunkBox::BOX_TYPE {
                sample_to_chunk_boxes.push(child);
            } else if box_type == SampleSizeBox::BOX_TYPE {
                sample_size_boxes.push(child);
            } else if box_type == ChunkOffsetBox::BOX_TYPE {
                chunk_offset_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            stsd: sample_description_boxes.exactly_one()?,
            stts: time_to_sample_boxes.exactly_one()?,
            stsc: sample_to_chunk_boxes.exactly_one()?,
            stsz: sample_size_boxes.exactly_one()?,
            stco: chunk_offset_boxes.exactly_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for SampleTableBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.stsd
            .encoded_len()
            .saturating_add(self.stts.encoded_len())
            .saturating_add(self.stsc.encoded_len())
            .saturating_add(self.stsz.encoded_len())
            .saturating_add(self.stco.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.stsd.encode(writer.take_remainder())?;
        rest = self.stts.encode(rest)?;
        rest = self.stsc.encode(rest)?;
        rest = self.stsz.encode(rest)?;
        rest = self.stco.encode(rest)?;
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

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, BoxWrite, Error};

    use super::SampleTableBox;
    use crate::stco::ChunkOffsetBox;
    use crate::stsc::SampleToChunkBox;
    use crate::stsd::SampleDescriptionBox;
    use crate::stsz::{SampleSizeBox, SampleSizes};
    use crate::stts::TimeToSampleBox;

    /// Sample table of a track whose samples are all described by fragments
    pub(crate) fn sample_table() -> SampleTableBox {
        SampleTableBox::new(
            SampleDescriptionBox::new(Vec::new()),
            TimeToSampleBox::new(Vec::new()),
            SampleToChunkBox::new(Vec::new()),
            SampleSizeBox::new(SampleSizes::PerSample(Vec::new())),
            ChunkOffsetBox::new(Vec::new()),
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sample_table: &SampleTableBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sample_table.payload_len()).unwrap()];
        sample_table.encode_payload(&mut buffer).unwrap();

        buffer
    }

    /// Writes one child whole, header and payload, as it lies in a sample table
    fn encoded_child(child: &impl BoxWrite) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(child.encoded_len()).unwrap()];
        child.encode(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&sample_table());

        assert_eq!(
            SampleTableBox::decode_payload(&payload).unwrap(),
            sample_table()
        );
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [
            encoded_payload(&sample_table()),
            vec![
                0, 0, 0, 0x10, b's', b't', b's', b's', 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ]
        .concat();

        let sample_table = SampleTableBox::decode_payload(&payload).unwrap();

        assert_eq!(
            sample_table.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"stss"))
        );
        assert_eq!(encoded_payload(&sample_table), payload);
    }

    #[test]
    fn a_box_missing_one_of_the_tables_it_must_hold_is_rejected() {
        let table = sample_table();
        let children = [
            (BoxType::compact(*b"stsd"), encoded_child(table.stsd())),
            (BoxType::compact(*b"stts"), encoded_child(table.stts())),
            (BoxType::compact(*b"stsc"), encoded_child(table.stsc())),
            (BoxType::compact(*b"stsz"), encoded_child(table.stsz())),
            (BoxType::compact(*b"stco"), encoded_child(table.stco())),
        ];

        for (missing, _) in &children {
            let payload: Vec<u8> = children
                .iter()
                .filter(|(box_type, _)| box_type != missing)
                .flat_map(|(_, bytes)| bytes.clone())
                .collect();

            assert_eq!(
                SampleTableBox::decode_payload(&payload),
                Err(Error::missing_mandatory_box(*missing))
            );
        }
    }

    #[test]
    fn a_second_table_of_a_type_the_box_holds_once_is_rejected() {
        let payload = [
            encoded_payload(&sample_table()),
            encoded_child(sample_table().stts()),
        ]
        .concat();

        assert_eq!(
            SampleTableBox::decode_payload(&payload),
            Err(Error::duplicate_box(BoxType::compact(*b"stts")))
        );
    }
}
