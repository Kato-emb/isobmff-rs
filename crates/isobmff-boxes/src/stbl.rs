//! [`SampleTableBox`], the time and data indexing of the samples of a track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, BoxVariants, ChildBoxes, Error,
    FieldReader, FieldWriter, OtherBoxes,
};

use crate::chunk_offset::ChunkOffsetVariant;
use crate::cslg::CompositionToDecodeBox;
use crate::ctts::CompositionOffsetBox;
use crate::padb::PaddingBitsBox;
use crate::saio::SampleAuxiliaryInformationOffsetsBox;
use crate::saiz::SampleAuxiliaryInformationSizesBox;
use crate::sample_size::SampleSizeVariant;
use crate::sdtp::SampleDependencyTypeBox;
use crate::ssrt::SubSampleReferenceTableBox;
use crate::stdp::DegradationPriorityBox;
use crate::stsc::SampleToChunkBox;
use crate::stsh::ShadowSyncSampleBox;
use crate::stss::SyncSampleBox;
use crate::stts::TimeToSampleBox;
use crate::subs::SubSampleInformationBox;

/// [`SampleTableBox`] (`stbl`), ISO/IEC 14496-12 §8.5.1
///
/// The tables that locate the samples of a track in time and in the file,
/// and that tell their type, size, chunk and offset.
///
/// A track that references no data need hold no child here. A track that
/// does holds an `stts`, an `stsc`, one sample size variant and one chunk
/// offset variant, so these are [`None`] together or [`Some`] together: a box
/// that holds some but not all of them reads, but writing it is refused, with
/// [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox) or
/// [`MissingVariantBox`](isobmff_core::ErrorKind::MissingVariantBox) for the
/// first that is missing.
///
/// An optional child of a version the box does not read is kept in
/// [`other_boxes`](Self::other_boxes), as are children of other types, such
/// as the `stsd`. Those are written back after the children the box holds a
/// field for.
#[doc(alias = "stbl")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SampleTableBox {
    /// Durations of the samples in decoding order
    pub stts: Option<TimeToSampleBox>,
    /// Offsets from the decoding timestamps of the samples to their composition timestamps
    pub ctts: Option<CompositionOffsetBox>,
    /// How the composition timeline relates to the decoding timeline
    pub cslg: Option<CompositionToDecodeBox>,
    /// Chunk of each sample and the sample entry that describes it
    pub stsc: Option<SampleToChunkBox>,
    /// Number of samples and size of each
    pub sample_size: Option<SampleSizeVariant>,
    /// Offset of the start of each chunk
    pub chunk_offset: Option<ChunkOffsetVariant>,
    /// Sync samples, every sample being one where it is [`None`]
    pub stss: Option<SyncSampleBox>,
    /// Sync samples that seeking may use in place of the samples they shadow
    pub stsh: Option<ShadowSyncSampleBox>,
    /// Padding bits at the end of each sample
    pub padb: Option<PaddingBitsBox>,
    /// Degradation priority of each sample
    pub stdp: Option<DegradationPriorityBox>,
    /// Dependencies of each sample
    pub sdtp: Option<SampleDependencyTypeBox>,
    /// Sub-samples of the samples, one box for each value of `flags`
    pub subs: Vec<SubSampleInformationBox>,
    /// Properties of sub-samples that the `subs` of the same `flags` refer to
    pub ssrt: Vec<SubSampleReferenceTableBox>,
    /// Sizes of the sample auxiliary information, one box for each type and stream
    pub saiz: Vec<SampleAuxiliaryInformationSizesBox>,
    /// Offsets of the sample auxiliary information, one box for each type and stream
    pub saio: Vec<SampleAuxiliaryInformationOffsetsBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for SampleTableBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stbl");
}

impl BoxDecode for SampleTableBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            stts: children.take_zero_or_one()?,
            ctts: children.take_zero_or_one_if_decoded()?,
            cslg: children.take_zero_or_one_if_decoded()?,
            stsc: children.take_zero_or_one()?,
            sample_size: children.take_zero_or_one_variant()?,
            chunk_offset: children.take_zero_or_one_variant()?,
            stss: children.take_zero_or_one_if_decoded()?,
            stsh: children.take_zero_or_one_if_decoded()?,
            padb: children.take_zero_or_one_if_decoded()?,
            stdp: children.take_zero_or_one_if_decoded()?,
            sdtp: children.take_zero_or_one_if_decoded()?,
            subs: children.take_zero_or_more()?,
            ssrt: children.take_zero_or_more()?,
            saiz: children.take_zero_or_more()?,
            saio: children.take_zero_or_more()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for SampleTableBox {
    fn payload_len(&self) -> u64 {
        let sample_size_len = self
            .sample_size
            .as_ref()
            .map_or(0, |variant| match variant {
                SampleSizeVariant::SampleSize(stsz) => stsz.encoded_len(),
                SampleSizeVariant::CompactSampleSize(stz2) => stz2.encoded_len(),
            });
        let chunk_offset_len = self
            .chunk_offset
            .as_ref()
            .map_or(0, |variant| match variant {
                ChunkOffsetVariant::ChunkOffset(stco) => stco.encoded_len(),
                ChunkOffsetVariant::ChunkLargeOffset(co64) => co64.encoded_len(),
            });

        [
            self.stts.as_ref().map_or(0, BoxEncode::encoded_len),
            self.ctts.as_ref().map_or(0, BoxEncode::encoded_len),
            self.cslg.as_ref().map_or(0, BoxEncode::encoded_len),
            self.stsc.as_ref().map_or(0, BoxEncode::encoded_len),
            sample_size_len,
            chunk_offset_len,
            self.stss.as_ref().map_or(0, BoxEncode::encoded_len),
            self.stsh.as_ref().map_or(0, BoxEncode::encoded_len),
            self.padb.as_ref().map_or(0, BoxEncode::encoded_len),
            self.stdp.as_ref().map_or(0, BoxEncode::encoded_len),
            self.sdtp.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.subs.iter().map(BoxEncode::encoded_len))
        .chain(self.ssrt.iter().map(BoxEncode::encoded_len))
        .chain(self.saiz.iter().map(BoxEncode::encoded_len))
        .chain(self.saio.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.stts.is_some()
            || self.stsc.is_some()
            || self.sample_size.is_some()
            || self.chunk_offset.is_some()
        {
            if self.stts.is_none() {
                return Err(Error::missing_mandatory_box(TimeToSampleBox::BOX_TYPE));
            }
            if self.stsc.is_none() {
                return Err(Error::missing_mandatory_box(SampleToChunkBox::BOX_TYPE));
            }
            if self.sample_size.is_none() {
                return Err(Error::missing_variant_box(SampleSizeVariant::VARIANTS));
            }
            if self.chunk_offset.is_none() {
                return Err(Error::missing_variant_box(ChunkOffsetVariant::VARIANTS));
            }
        }

        let mut rest = writer.take_remainder();
        if let Some(stts) = &self.stts {
            rest = stts.encode(rest)?;
        }
        if let Some(ctts) = &self.ctts {
            rest = ctts.encode(rest)?;
        }
        if let Some(cslg) = &self.cslg {
            rest = cslg.encode(rest)?;
        }
        if let Some(stsc) = &self.stsc {
            rest = stsc.encode(rest)?;
        }
        rest = match &self.sample_size {
            Some(SampleSizeVariant::SampleSize(stsz)) => stsz.encode(rest)?,
            Some(SampleSizeVariant::CompactSampleSize(stz2)) => stz2.encode(rest)?,
            None => rest,
        };
        rest = match &self.chunk_offset {
            Some(ChunkOffsetVariant::ChunkOffset(stco)) => stco.encode(rest)?,
            Some(ChunkOffsetVariant::ChunkLargeOffset(co64)) => co64.encode(rest)?,
            None => rest,
        };
        if let Some(stss) = &self.stss {
            rest = stss.encode(rest)?;
        }
        if let Some(stsh) = &self.stsh {
            rest = stsh.encode(rest)?;
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
        for subs in &self.subs {
            rest = subs.encode(rest)?;
        }
        for ssrt in &self.ssrt {
            rest = ssrt.encode(rest)?;
        }
        for saiz in &self.saiz {
            rest = saiz.encode(rest)?;
        }
        for saio in &self.saio {
            rest = saio.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, BoxVariants, ChildBoxes, Error, OtherBoxes,
    };

    use super::SampleTableBox;
    use crate::chunk_offset::{ChunkOffsetBox, ChunkOffsetVariant};
    use crate::encoded;
    use crate::sample_size::{SampleSizeBox, SampleSizeVariant, SampleSizes};
    use crate::stsc::{SampleToChunkBox, SampleToChunkEntry};
    use crate::stss::SyncSampleBox;
    use crate::stts::{TimeToSampleBox, TimeToSampleEntry};

    /// `stsd` of no entry, a box `stbl` holds no field for
    const STSD: &[u8] = b"\0\0\0\x10stsd\0\0\0\0\0\0\0\0";

    /// `stts` of 2 samples of duration 10
    const STTS: &[u8] = b"\0\0\0\x18stts\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0\x0a";

    /// `stsc` of chunks of 2 samples from chunk 1, of sample entry 1
    const STSC: &[u8] = b"\0\0\0\x1cstsc\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\0\x02\0\0\0\x01";

    /// `stsz` of 2 samples of 100 bytes each
    const STSZ: &[u8] = b"\0\0\0\x14stsz\0\0\0\0\0\0\0\x64\0\0\0\x02";

    /// `stco` of one chunk at 48
    const STCO: &[u8] = b"\0\0\0\x14stco\0\0\0\0\0\0\0\x01\0\0\0\x30";

    /// `stss` marking the sample 1
    const STSS: &[u8] = b"\0\0\0\x14stss\0\0\0\0\0\0\0\x01\0\0\0\x01";

    /// `stss` of version 1, which `stbl` keeps as found
    const STSS_VERSION_1: &[u8] = b"\0\0\0\x10stss\x01\0\0\0\0\0\0\0";

    fn sample_table(stss: Option<SyncSampleBox>, other_boxes: &[u8]) -> SampleTableBox {
        SampleTableBox {
            stts: Some(TimeToSampleBox {
                entries: vec![TimeToSampleEntry {
                    sample_count: 2,
                    sample_delta: 10,
                }],
            }),
            stsc: Some(SampleToChunkBox {
                entries: vec![SampleToChunkEntry {
                    first_chunk: 1,
                    samples_per_chunk: 2,
                    sample_description_index: 1,
                }],
            }),
            sample_size: Some(SampleSizeVariant::SampleSize(SampleSizeBox {
                sample_sizes: SampleSizes::Constant {
                    sample_size: NonZeroU32::new(100).unwrap(),
                    sample_count: 2,
                },
            })),
            chunk_offset: Some(ChunkOffsetVariant::ChunkOffset(ChunkOffsetBox {
                chunk_offsets: vec![48],
            })),
            stss,
            other_boxes: OtherBoxes::from(ChildBoxes::decode(other_boxes).unwrap()),
            ..SampleTableBox::default()
        }
    }

    #[test]
    fn reads_its_tables_and_keeps_the_child_it_has_no_field_for() {
        let payload = [STSD, STTS, STSC, STSZ, STCO, STSS].concat();

        assert_eq!(
            SampleTableBox::decode_payload(&payload),
            Ok(sample_table(
                Some(SyncSampleBox {
                    sample_numbers: vec![1]
                }),
                STSD
            ))
        );
    }

    #[test]
    fn writes_its_tables_in_the_order_of_the_specification_then_the_child_it_has_no_field_for() {
        let sync_sample = SyncSampleBox {
            sample_numbers: vec![1],
        };

        assert_eq!(
            encoded(&sample_table(Some(sync_sample), STSD)),
            [
                b"\0\0\0\x88stbl".as_slice(),
                STTS,
                STSC,
                STSZ,
                STCO,
                STSS,
                STSD
            ]
            .concat()
        );
    }

    #[test]
    fn an_optional_table_of_a_version_it_does_not_read_is_kept_as_found() {
        let payload = [STTS, STSC, STSZ, STCO, STSS_VERSION_1].concat();

        assert_eq!(
            SampleTableBox::decode_payload(&payload),
            Ok(sample_table(None, STSS_VERSION_1))
        );
    }

    #[test]
    fn a_mandatory_table_that_does_not_read_is_refused() {
        let payload = [
            b"\0\0\0\x10stts\x01\0\0\0\0\0\0\0".as_slice(),
            STSC,
            STSZ,
            STCO,
        ]
        .concat();

        assert_eq!(
            SampleTableBox::decode_payload(&payload),
            Err(Error::unsupported_version(1).in_container(TimeToSampleBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_box_of_no_table_reads_and_writes_back() {
        let (read, _) = SampleTableBox::decode(b"\0\0\0\x08stbl").unwrap();

        assert_eq!(
            (encoded(&read), read),
            (b"\0\0\0\x08stbl".to_vec(), SampleTableBox::default())
        );
    }

    #[test]
    fn a_box_missing_some_of_the_mandatory_tables_reads_but_is_refused_when_written() {
        let payload = [STTS, STSC, STSZ].concat();
        let read = SampleTableBox::decode_payload(&payload).unwrap();

        assert_eq!(
            read.encode(&mut [0; 80]),
            Err(Error::missing_variant_box(ChunkOffsetVariant::VARIANTS))
        );
    }
}
