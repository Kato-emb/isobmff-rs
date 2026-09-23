//! [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes, boxes,
};

use crate::chunk_offset::{ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsets};
use crate::ctts::CompositionOffsetBox;
use crate::padb::PaddingBitsBox;
use crate::sample_size::{CompactSampleSizeBox, SampleSizeBox, SampleSizes};
use crate::sdtp::SampleDependencyTypeBox;
use crate::stdp::DegradationPriorityBox;
use crate::stsc::SampleToChunkBox;
use crate::stsd::SampleDescriptionBox;
use crate::stss::SyncSampleBox;
use crate::stts::TimeToSampleBox;

/// Box types a sample table states the sizes of its samples with, of which it holds one
const SAMPLE_SIZE_BOXES: &[BoxType] = &[SampleSizeBox::BOX_TYPE, CompactSampleSizeBox::BOX_TYPE];

/// Box types a sample table states the offsets of its chunks with, of which it holds one
const CHUNK_OFFSET_BOXES: &[BoxType] = &[ChunkOffsetBox::BOX_TYPE, ChunkLargeOffsetBox::BOX_TYPE];

/// Box that holds every table locating and describing the samples of a track
///
/// [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1. The tables a track
/// that references data must state are promoted to fields of their own — the
/// sample descriptions, the decode timeline, the grouping into chunks, the
/// sample sizes, and the chunk offsets — and so are the optional tables that
/// state the composition time offsets, the sync samples, the padding bits,
/// the degradation priorities, and the sample dependencies, which
/// [`new`](Self::new) leaves out and the `with_` methods set. Every other child
/// is kept in [`other_boxes`](Self::other_boxes) and written back unread.
///
/// Decoding asks for all five required tables. §8.5.1 lets the `stbl` of a track that
/// references no data hold no children at all, and such a box does not decode
/// into this type — the raw walk still reads it. The sample sizes are taken
/// from whichever of `stsz` and `stz2` the box holds, and the chunk offsets
/// from whichever of `stco` and `co64`; the one it holds is the one written
/// back.
///
/// On encode the children are written in the order the spec lists them — `stsd`,
/// `stts`, `ctts`, `stsc`, the sample sizes, the chunk offsets, `stss`, `padb`, `stdp`,
/// then `sdtp` — and then the children no field claims, so a round-trip
/// settles the order rather than preserving it.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{
///     ChunkOffsetBox, ChunkOffsets, SampleDescriptionBox, SampleSizeBox, SampleSizeEntries,
///     SampleSizes, SampleTableBox, SampleToChunkBox, TimeToSampleBox,
/// };
/// use isobmff_core::{BoxDecode, BoxEncode};
///
/// // A fragmented movie describes its samples in its fragments, so these are empty
/// let sample_table = SampleTableBox::new(
///     SampleDescriptionBox::new(Vec::new()),
///     TimeToSampleBox::new(Vec::new()),
///     SampleToChunkBox::new(Vec::new()),
///     SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(Vec::new()))),
///     ChunkOffsets::Stco(ChunkOffsetBox::new(Vec::new())),
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
    sample_sizes: SampleSizes,
    chunk_offsets: ChunkOffsets,
    ctts: Option<CompositionOffsetBox>,
    stss: Option<SyncSampleBox>,
    padb: Option<PaddingBitsBox>,
    stdp: Option<DegradationPriorityBox>,
    sdtp: Option<SampleDependencyTypeBox>,
    other_boxes: OtherBoxes,
}

impl SampleTableBox {
    /// Creates the box from the tables that locate and describe the samples
    ///
    /// The box holds none of the optional tables; the `with_` methods set them.
    #[must_use]
    pub const fn new(
        stsd: SampleDescriptionBox,
        stts: TimeToSampleBox,
        stsc: SampleToChunkBox,
        sample_sizes: SampleSizes,
        chunk_offsets: ChunkOffsets,
    ) -> Self {
        Self {
            stsd,
            stts,
            stsc,
            sample_sizes,
            chunk_offsets,
            ctts: None,
            stss: None,
            padb: None,
            stdp: None,
            sdtp: None,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Sets the offset from the decode time of every sample to its composition time
    #[must_use]
    pub fn with_ctts(self, ctts: CompositionOffsetBox) -> Self {
        Self {
            ctts: Some(ctts),
            ..self
        }
    }

    /// Sets which samples are sync samples
    #[must_use]
    pub fn with_stss(self, stss: SyncSampleBox) -> Self {
        Self {
            stss: Some(stss),
            ..self
        }
    }

    /// Sets how each sample depends on the others
    #[must_use]
    pub fn with_sdtp(self, sdtp: SampleDependencyTypeBox) -> Self {
        Self {
            sdtp: Some(sdtp),
            ..self
        }
    }

    /// Sets how many bits at the end of each sample are padding
    #[must_use]
    pub fn with_padb(self, padb: PaddingBitsBox) -> Self {
        Self {
            padb: Some(padb),
            ..self
        }
    }

    /// Sets the degradation priority of each sample
    #[must_use]
    pub fn with_stdp(self, stdp: DegradationPriorityBox) -> Self {
        Self {
            stdp: Some(stdp),
            ..self
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

    /// Returns how many bytes each sample occupies, in whichever table the box states it
    #[must_use]
    pub const fn sample_sizes(&self) -> &SampleSizes {
        &self.sample_sizes
    }

    /// Returns where every chunk of the track lies, at whichever width the box states it
    #[must_use]
    pub const fn chunk_offsets(&self) -> &ChunkOffsets {
        &self.chunk_offsets
    }

    /// Returns the offset from the decode time of every sample to its composition time
    ///
    /// `None` when the box carries no `ctts`, which has every sample composed
    /// when it is decoded.
    #[must_use]
    pub const fn ctts(&self) -> Option<&CompositionOffsetBox> {
        self.ctts.as_ref()
    }

    /// Returns which samples are sync samples
    ///
    /// `None` when the box carries no `stss`, which has every sample a sync
    /// sample.
    #[must_use]
    pub const fn stss(&self) -> Option<&SyncSampleBox> {
        self.stss.as_ref()
    }

    /// Returns how each sample depends on the others, if the box states it
    #[must_use]
    pub const fn sdtp(&self) -> Option<&SampleDependencyTypeBox> {
        self.sdtp.as_ref()
    }

    /// Returns how many bits at the end of each sample are padding, if the box states it
    #[must_use]
    pub const fn padb(&self) -> Option<&PaddingBitsBox> {
        self.padb.as_ref()
    }

    /// Returns the degradation priority of each sample, if the box states it
    #[must_use]
    pub const fn stdp(&self) -> Option<&DegradationPriorityBox> {
        self.stdp.as_ref()
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
    ///   `stsd`, `stts`, or `stsc`.
    /// * [`MissingAlternativeBox`](isobmff_core::ErrorKind::MissingAlternativeBox):
    ///   neither `stsz` nor `stz2`, or neither `stco` nor `co64`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one of
    ///   any of them, or of any of the optional tables.
    /// * [`DuplicateAlternativeBox`](isobmff_core::ErrorKind::DuplicateAlternativeBox):
    ///   both a `stsz` and a `stz2`, or both a `stco` and a `co64`, of which §8.7.3
    ///   and §8.7.5 have the box hold one.
    /// * Whatever a child reports, on the [`containers`](Error::containers) path: one
    ///   of the tables does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut sample_description_boxes = ChildBoxes::new();
        let mut time_to_sample_boxes = ChildBoxes::new();
        let mut sample_to_chunk_boxes = ChildBoxes::new();
        let mut sample_size_boxes = ChildBoxes::new();
        let mut chunk_offset_boxes = ChildBoxes::new();
        let mut composition_offset_boxes = ChildBoxes::new();
        let mut sync_sample_boxes = ChildBoxes::new();
        let mut padding_bits_boxes = ChildBoxes::new();
        let mut degradation_priority_boxes = ChildBoxes::new();
        let mut sample_dependency_type_boxes = ChildBoxes::new();
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
            } else if SAMPLE_SIZE_BOXES.contains(&box_type) {
                sample_size_boxes.push(child);
            } else if CHUNK_OFFSET_BOXES.contains(&box_type) {
                chunk_offset_boxes.push(child);
            } else if box_type == CompositionOffsetBox::BOX_TYPE {
                composition_offset_boxes.push(child);
            } else if box_type == SyncSampleBox::BOX_TYPE {
                sync_sample_boxes.push(child);
            } else if box_type == PaddingBitsBox::BOX_TYPE {
                padding_bits_boxes.push(child);
            } else if box_type == DegradationPriorityBox::BOX_TYPE {
                degradation_priority_boxes.push(child);
            } else if box_type == SampleDependencyTypeBox::BOX_TYPE {
                sample_dependency_type_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            stsd: sample_description_boxes.exactly_one()?,
            stts: time_to_sample_boxes.exactly_one()?,
            stsc: sample_to_chunk_boxes.exactly_one()?,
            sample_sizes: SampleSizes::decode(
                sample_size_boxes.exactly_one_variant(SAMPLE_SIZE_BOXES)?,
            )?,
            chunk_offsets: ChunkOffsets::decode(
                chunk_offset_boxes.exactly_one_variant(CHUNK_OFFSET_BOXES)?,
            )?,
            ctts: composition_offset_boxes.zero_or_one()?,
            stss: sync_sample_boxes.zero_or_one()?,
            padb: padding_bits_boxes.zero_or_one()?,
            stdp: degradation_priority_boxes.zero_or_one()?,
            sdtp: sample_dependency_type_boxes.zero_or_one()?,
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
            .saturating_add(self.ctts.as_ref().map_or(0, BoxEncode::encoded_len))
            .saturating_add(self.stsc.encoded_len())
            .saturating_add(self.sample_sizes.encoded_len())
            .saturating_add(self.chunk_offsets.encoded_len())
            .saturating_add(self.stss.as_ref().map_or(0, BoxEncode::encoded_len))
            .saturating_add(self.padb.as_ref().map_or(0, BoxEncode::encoded_len))
            .saturating_add(self.stdp.as_ref().map_or(0, BoxEncode::encoded_len))
            .saturating_add(self.sdtp.as_ref().map_or(0, BoxEncode::encoded_len))
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.stsd.encode(writer.take_remainder())?;
        rest = self.stts.encode(rest)?;
        if let Some(ctts) = &self.ctts {
            rest = ctts.encode(rest)?;
        }
        rest = self.stsc.encode(rest)?;
        rest = self.sample_sizes.encode(rest)?;
        rest = self.chunk_offsets.encode(rest)?;
        if let Some(stss) = &self.stss {
            rest = stss.encode(rest)?;
        }
        if let Some(padb) = &self.padb {
            rest = padb.encode(rest)?;
        }
        if let Some(stdp) = &self.stdp {
            rest = stdp.encode(rest)?;
        }
        if let Some(sdtp) = &self.sdtp {
            rest = sdtp.encode(rest)?;
        }
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

    use isobmff_core::{AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, boxes};

    use super::{CHUNK_OFFSET_BOXES, SAMPLE_SIZE_BOXES, SampleTableBox};
    use crate::chunk_offset::{ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsets};
    use crate::ctts::{CompositionOffsetBox, CompositionOffsetEntry};
    use crate::padb::{PaddingBitsBox, PaddingBitsEntry};
    use crate::sample_size::{CompactSampleSizeBox, SampleSizeBox, SampleSizeEntries, SampleSizes};
    use crate::sdtp::{SampleDependencyTypeBox, SampleDependencyTypeEntry};
    use crate::stdp::{DegradationPriorityBox, DegradationPriorityEntry};
    use crate::stsc::SampleToChunkBox;
    use crate::stsd::SampleDescriptionBox;
    use crate::stss::{SyncSampleBox, SyncSampleEntry};
    use crate::stts::TimeToSampleBox;
    use crate::trun::CompositionTimeOffset;

    /// Sample table of a track whose samples are all described by fragments
    pub(crate) fn sample_table() -> SampleTableBox {
        SampleTableBox::new(
            SampleDescriptionBox::new(Vec::new()),
            TimeToSampleBox::new(Vec::new()),
            SampleToChunkBox::new(Vec::new()),
            SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(Vec::new()))),
            ChunkOffsets::Stco(ChunkOffsetBox::new(Vec::new())),
        )
    }

    /// Sample table stating every optional table for one sample
    fn sample_table_with_every_optional_table() -> SampleTableBox {
        sample_table()
            .with_sdtp(SampleDependencyTypeBox::new(vec![
                SampleDependencyTypeEntry::new(2, 2, 1, 2).unwrap(),
            ]))
            .with_stdp(DegradationPriorityBox::new(vec![
                DegradationPriorityEntry::new(3),
            ]))
            .with_padb(PaddingBitsBox::new(vec![PaddingBitsEntry::new(5).unwrap()]))
            .with_stss(SyncSampleBox::new(vec![SyncSampleEntry::new(1)]))
            .with_ctts(
                CompositionOffsetBox::new(vec![CompositionOffsetEntry::new(
                    1,
                    CompositionTimeOffset::new(-8).unwrap(),
                )])
                .unwrap(),
            )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sample_table: &SampleTableBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sample_table.payload_len()).unwrap()];
        sample_table.encode_payload(&mut buffer).unwrap();

        buffer
    }

    /// Writes one child whole, header and payload, as it lies in a sample table
    fn encoded_child(child: &(impl BoxDefinition + BoxEncode)) -> Vec<u8> {
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
    fn a_box_holding_every_optional_table_reads_back_as_the_value_that_wrote_it() {
        let table = sample_table_with_every_optional_table();

        let payload = encoded_payload(&table);

        assert_eq!(SampleTableBox::decode_payload(&payload).unwrap(), table);
    }

    #[test]
    fn the_children_are_written_in_the_order_the_spec_lists_them() {
        let payload = encoded_payload(&sample_table_with_every_optional_table());

        let box_types: Vec<BoxType> = boxes(&payload)
            .map(|child| child.unwrap().header().box_type())
            .collect();

        assert_eq!(
            box_types,
            [
                *b"stsd", *b"stts", *b"ctts", *b"stsc", *b"stsz", *b"stco", *b"stss", *b"padb",
                *b"stdp", *b"sdtp",
            ]
            .map(BoxType::compact)
        );
    }

    #[test]
    fn a_second_optional_table_of_one_type_is_rejected() {
        let table = sample_table_with_every_optional_table();
        let seconds = [
            (
                encoded_child(table.ctts().unwrap()),
                CompositionOffsetBox::BOX_TYPE,
            ),
            (
                encoded_child(table.stss().unwrap()),
                SyncSampleBox::BOX_TYPE,
            ),
            (
                encoded_child(table.padb().unwrap()),
                PaddingBitsBox::BOX_TYPE,
            ),
            (
                encoded_child(table.stdp().unwrap()),
                DegradationPriorityBox::BOX_TYPE,
            ),
            (
                encoded_child(table.sdtp().unwrap()),
                SampleDependencyTypeBox::BOX_TYPE,
            ),
        ];

        for (second, box_type) in seconds {
            let payload = [encoded_payload(&table), second].concat();

            assert_eq!(
                SampleTableBox::decode_payload(&payload),
                Err(Error::duplicate_box(box_type))
            );
        }
    }

    #[test]
    fn a_child_no_field_claims_is_kept_and_written_back() {
        let payload = [
            encoded_payload(&sample_table()),
            vec![
                0, 0, 0, 0x10, b's', b'u', b'b', b's', 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ]
        .concat();

        let sample_table = SampleTableBox::decode_payload(&payload).unwrap();

        assert_eq!(
            sample_table.other_boxes().first().map(AnyBox::box_type),
            Some(BoxType::compact(*b"subs"))
        );
        assert_eq!(encoded_payload(&sample_table), payload);
    }

    #[test]
    fn a_box_missing_one_of_the_tables_it_must_hold_is_rejected() {
        let table = sample_table();
        let children = [
            (
                BoxType::compact(*b"stsd"),
                encoded_child(table.stsd()),
                Error::missing_mandatory_box(BoxType::compact(*b"stsd")),
            ),
            (
                BoxType::compact(*b"stts"),
                encoded_child(table.stts()),
                Error::missing_mandatory_box(BoxType::compact(*b"stts")),
            ),
            (
                BoxType::compact(*b"stsc"),
                encoded_child(table.stsc()),
                Error::missing_mandatory_box(BoxType::compact(*b"stsc")),
            ),
            (
                BoxType::compact(*b"stsz"),
                encoded_child(&SampleSizeBox::from_sizes([])),
                Error::missing_alternative_box(SAMPLE_SIZE_BOXES),
            ),
            (
                BoxType::compact(*b"stco"),
                encoded_child(&ChunkOffsetBox::new(Vec::new())),
                Error::missing_alternative_box(CHUNK_OFFSET_BOXES),
            ),
        ];

        for (missing, _, reported) in &children {
            let payload: Vec<u8> = children
                .iter()
                .filter(|(box_type, ..)| box_type != missing)
                .flat_map(|(_, bytes, _)| bytes.clone())
                .collect();

            assert_eq!(SampleTableBox::decode_payload(&payload), Err(*reported));
        }
    }

    #[test]
    fn a_box_stating_its_chunk_offsets_in_64_bits_reads_back_as_the_value_that_wrote_it() {
        let mut table = sample_table();
        table.chunk_offsets = ChunkOffsets::Co64(ChunkLargeOffsetBox::new(Vec::new()));

        let payload = encoded_payload(&table);

        assert_eq!(SampleTableBox::decode_payload(&payload).unwrap(), table);
    }

    #[test]
    fn a_box_stating_its_chunk_offsets_both_ways_is_rejected() {
        let payload = [
            encoded_payload(&sample_table()),
            encoded_child(&ChunkLargeOffsetBox::new(Vec::new())),
        ]
        .concat();

        assert_eq!(
            SampleTableBox::decode_payload(&payload),
            Err(Error::duplicate_alternative_box(CHUNK_OFFSET_BOXES))
        );
    }

    #[test]
    fn a_box_stating_its_sample_sizes_in_a_stz2_reads_back_as_the_value_that_wrote_it() {
        let mut table = sample_table();
        table.sample_sizes = SampleSizes::Stz2(CompactSampleSizeBox::from_sizes([4, 8, 15]));

        let payload = encoded_payload(&table);

        assert_eq!(SampleTableBox::decode_payload(&payload).unwrap(), table);
    }

    #[test]
    fn a_box_stating_its_sample_sizes_twice_is_rejected() {
        let both_ways = [
            encoded_payload(&sample_table()),
            encoded_child(&CompactSampleSizeBox::from_sizes([])),
        ]
        .concat();
        let one_way_twice = [
            encoded_payload(&sample_table()),
            encoded_child(&SampleSizeBox::from_sizes([])),
        ]
        .concat();

        assert_eq!(
            SampleTableBox::decode_payload(&both_ways),
            Err(Error::duplicate_alternative_box(SAMPLE_SIZE_BOXES))
        );
        assert_eq!(
            SampleTableBox::decode_payload(&one_way_twice),
            Err(Error::duplicate_box(SampleSizeBox::BOX_TYPE))
        );
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
