//! [`SegmentIndexBox`] (`sidx`), ISO/IEC 14496-12 §8.16.3

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the references when version 0 carries the time and the offset in 32 bits
const FIXED_FIELDS_LEN_VERSION_0: u64 = 24;

/// Length of the fields that precede the references when version 1 carries the time and the offset in 64 bits
const FIXED_FIELDS_LEN_VERSION_1: u64 = 32;

/// Length of one reference of the table
const REFERENCE_LEN: u64 = 12;

/// Bit of the first word of a reference that states it points at another `sidx`
const REFERENCE_TYPE_BIT: u32 = 0x8000_0000;

/// Largest `referenced_size` its 31 bits hold
const REFERENCED_SIZE_MAXIMUM: u32 = 0x7fff_ffff;

/// Bit of the last word of a reference that states the subsegment starts with a SAP
const STARTS_WITH_SAP_BIT: u32 = 0x8000_0000;

/// Largest `SAP_type` its 3 bits hold
const SAP_TYPE_MAXIMUM: u8 = 0b111;

/// Largest `SAP_delta_time` its 28 bits hold
const SAP_DELTA_TIME_MAXIMUM: u32 = 0x0fff_ffff;

/// What the bytes a [`SegmentIndexReference`] points at hold
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ReferenceType {
    /// Media: a subsegment of movie fragments and the media data they address
    MediaContent,
    /// Another `sidx`, which indexes the subsegments the reference covers
    SegmentIndex,
}

/// One reference of the table a [`SegmentIndexBox`] holds
///
/// The reference states how many bytes the subsegment it points at occupies,
/// how long that subsegment lasts, and where its first stream access point
/// (SAP) lies.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SegmentIndexReference {
    reference_type: ReferenceType,
    referenced_size: u32,
    subsegment_duration: u32,
    starts_with_sap: bool,
    sap_type: u8,
    sap_delta_time: u32,
}

impl SegmentIndexReference {
    /// Creates the reference from what it points at and the SAP it states
    ///
    /// Returns `None` when `referenced_size` is past its 31 bits, `sap_type`
    /// past its 3 bits, or `sap_delta_time` past its 28 bits.
    #[must_use]
    pub const fn new(
        reference_type: ReferenceType,
        referenced_size: u32,
        subsegment_duration: u32,
        starts_with_sap: bool,
        sap_type: u8,
        sap_delta_time: u32,
    ) -> Option<Self> {
        if referenced_size > REFERENCED_SIZE_MAXIMUM
            || sap_type > SAP_TYPE_MAXIMUM
            || sap_delta_time > SAP_DELTA_TIME_MAXIMUM
        {
            return None;
        }

        Some(Self {
            reference_type,
            referenced_size,
            subsegment_duration,
            starts_with_sap,
            sap_type,
            sap_delta_time,
        })
    }

    /// Returns what the bytes this reference points at hold
    #[must_use]
    pub const fn reference_type(&self) -> ReferenceType {
        self.reference_type
    }

    /// Returns how many bytes the referenced material occupies
    #[must_use]
    pub const fn referenced_size(&self) -> u32 {
        self.referenced_size
    }

    /// Returns how long the referenced subsegment lasts, in the time scale of the box
    #[must_use]
    pub const fn subsegment_duration(&self) -> u32 {
        self.subsegment_duration
    }

    /// Returns whether the referenced subsegment starts with a SAP
    #[must_use]
    pub const fn starts_with_sap(&self) -> bool {
        self.starts_with_sap
    }

    /// Returns the type of the first SAP of the subsegment, 0 when the type is unknown or no SAP information is given
    #[must_use]
    pub const fn sap_type(&self) -> u8 {
        self.sap_type
    }

    /// Returns how far past the earliest presentation time of the subsegment its first SAP lies
    #[must_use]
    pub const fn sap_delta_time(&self) -> u32 {
        self.sap_delta_time
    }
}

/// Box that indexes the subsegments of a segment by their bytes and times
///
/// [`SegmentIndexBox`] (`sidx`), ISO/IEC 14496-12 §8.16.3. The references run
/// back to back from the anchor — the first byte after this box, in the file
/// holding it — plus
/// `first_offset`, and their presentation times run on from
/// `earliest_presentation_time` by the duration of each.
///
/// The version is not held: it selects how wide the earliest presentation time
/// and the first offset are written, so
/// [`encode_payload`](BoxEncode::encode_payload) picks the narrower one whenever
/// both fit in 32 bits. The `reference_count` field is not held either: it
/// counts the references, so it is derived on the way out, and on the way in a
/// count that disagrees with the references fails the box. The reserved bits
/// are read as nothing and written as zero.
#[doc(alias = "sidx")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SegmentIndexBox {
    reference_id: u32,
    timescale: u32,
    earliest_presentation_time: u64,
    first_offset: u64,
    references: Vec<SegmentIndexReference>,
}

impl SegmentIndexBox {
    /// Creates the box from the stream it indexes, where its references start, and the references
    ///
    /// Returns `None` when there are more references than the 16 bits of
    /// `reference_count` count.
    #[must_use]
    pub fn new(
        reference_id: u32,
        timescale: u32,
        earliest_presentation_time: u64,
        first_offset: u64,
        references: Vec<SegmentIndexReference>,
    ) -> Option<Self> {
        if references.len() > usize::from(u16::MAX) {
            return None;
        }

        Some(Self {
            reference_id,
            timescale,
            earliest_presentation_time,
            first_offset,
            references,
        })
    }

    /// Returns the ID of the stream the box indexes, a track ID in files based on ISO/IEC 14496-12
    #[must_use]
    pub const fn reference_id(&self) -> u32 {
        self.reference_id
    }

    /// Returns the units per second the times of this box count
    #[must_use]
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Returns the earliest presentation time of the first subsegment, in the time scale of the box
    #[must_use]
    pub const fn earliest_presentation_time(&self) -> u64 {
        self.earliest_presentation_time
    }

    /// Returns how many bytes after the anchor the first subsegment starts
    #[must_use]
    pub const fn first_offset(&self) -> u64 {
        self.first_offset
    }

    /// Returns the references, in the order their subsegments lie
    #[must_use]
    pub fn references(&self) -> &[SegmentIndexReference] {
        &self.references
    }

    /// Returns the version whose field width carries the time and the offset of this box
    const fn version(&self) -> u8 {
        if self.earliest_presentation_time <= u32::MAX as u64
            && self.first_offset <= u32::MAX as u64
        {
            0
        } else {
            1
        }
    }

    /// Returns the width the given version carries the time and the offset at
    const fn field_width(version: u8) -> FieldWidth {
        match version {
            0 => FieldWidth::Compact,
            _ => FieldWidth::Extended,
        }
    }
}

impl BoxDefinition for SegmentIndexBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sidx");
}

impl BoxDecode for SegmentIndexBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its references.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `reference_count` field disagrees with the references that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }

        let reference_id = reader.read_u32()?;
        let timescale = reader.read_u32()?;
        let width = Self::field_width(version);
        let earliest_presentation_time = reader.read_unsigned(width)?;
        let first_offset = reader.read_unsigned(width)?;
        let &[_, _, count_high, count_low] = reader.read_bytes::<4>()?;
        let declared = u64::from(u16::from_be_bytes([count_high, count_low]));

        let mut references = Vec::new();
        while !reader.remainder().is_empty() {
            let size_word = reader.read_u32()?;
            let subsegment_duration = reader.read_u32()?;
            let sap_bytes = reader.read_bytes::<4>()?;
            let &[sap_high, ..] = sap_bytes;
            let sap_word = u32::from_be_bytes(*sap_bytes);

            references.push(SegmentIndexReference {
                reference_type: if size_word & REFERENCE_TYPE_BIT == 0 {
                    ReferenceType::MediaContent
                } else {
                    ReferenceType::SegmentIndex
                },
                referenced_size: size_word & REFERENCED_SIZE_MAXIMUM,
                subsegment_duration,
                starts_with_sap: sap_word & STARTS_WITH_SAP_BIT != 0,
                sap_type: sap_high >> 4 & SAP_TYPE_MAXIMUM,
                sap_delta_time: sap_word & SAP_DELTA_TIME_MAXIMUM,
            });
        }

        let actual = references.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self {
            reference_id,
            timescale,
            earliest_presentation_time,
            first_offset,
            references,
        })
    }
}

impl BoxEncode for SegmentIndexBox {
    fn payload_len(&self) -> u64 {
        let fixed_fields = if self.version() == 0 {
            FIXED_FIELDS_LEN_VERSION_0
        } else {
            FIXED_FIELDS_LEN_VERSION_1
        };
        let references = (self.references.len() as u64).saturating_mul(REFERENCE_LEN);

        fixed_fields.saturating_add(references)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = self.version();

        writer.write_bytes(&FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.reference_id)?;
        writer.write_u32(self.timescale)?;
        let width = Self::field_width(version);
        writer.write_unsigned(width, self.earliest_presentation_time)?;
        writer.write_unsigned(width, self.first_offset)?;
        // Why not fail on a count past `u16`: `new` and `decode_payload` hold
        // the references to what the field counts, so the fallback is never
        // taken.
        let reference_count = u16::try_from(self.references.len()).unwrap_or(u16::MAX);
        writer.write_u32(u32::from(reference_count))?;

        for reference in &self.references {
            let reference_type_bit = match reference.reference_type {
                ReferenceType::MediaContent => 0,
                ReferenceType::SegmentIndex => REFERENCE_TYPE_BIT,
            };
            let starts_with_sap_bit = if reference.starts_with_sap {
                STARTS_WITH_SAP_BIT
            } else {
                0
            };

            writer.write_u32(reference_type_bit | reference.referenced_size)?;
            writer.write_u32(reference.subsegment_duration)?;
            writer.write_u32(
                starts_with_sap_bit
                    | u32::from(reference.sap_type) << 28
                    | reference.sap_delta_time,
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{ReferenceType, SegmentIndexBox, SegmentIndexReference};

    /// Reference to a subsegment of `referenced_size` bytes that starts with a SAP of type 1
    fn media_reference(referenced_size: u32) -> SegmentIndexReference {
        SegmentIndexReference::new(
            ReferenceType::MediaContent,
            referenced_size,
            3_000,
            true,
            1,
            0,
        )
        .unwrap()
    }

    /// Index of two subsegments starting at the given time and offset
    fn segment_index(earliest_presentation_time: u64, first_offset: u64) -> SegmentIndexBox {
        SegmentIndexBox::new(
            1,
            90_000,
            earliest_presentation_time,
            first_offset,
            vec![media_reference(1_000), media_reference(2_000)],
        )
        .unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(segment_index: &SegmentIndexBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(segment_index.payload_len()).unwrap()];
        segment_index.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_reference_lays_its_bit_fields_out_as_the_spec_packs_them() {
        let index = SegmentIndexBox::new(
            1,
            90_000,
            0,
            0,
            vec![
                SegmentIndexReference::new(
                    ReferenceType::SegmentIndex,
                    0x7fff_ffff,
                    3_000,
                    true,
                    5,
                    0x0fff_ffff,
                )
                .unwrap(),
            ],
        )
        .unwrap();

        let payload = encoded_payload(&index);

        assert_eq!(
            payload.get(24..).unwrap(),
            b"\xff\xff\xff\xff\0\0\x0b\xb8\xdf\xff\xff\xff"
        );
        assert_eq!(SegmentIndexBox::decode_payload(&payload).unwrap(), index);
    }

    #[test]
    fn times_and_an_offset_within_32_bits_are_written_at_version_0() {
        let payload = encoded_payload(&segment_index(u64::from(u32::MAX), u64::from(u32::MAX)));

        assert_eq!(
            payload.get(..24).unwrap(),
            b"\0\0\0\0\0\0\0\x01\0\x01\x5f\x90\xff\xff\xff\xff\xff\xff\xff\xff\0\0\0\x02"
        );
    }

    #[test]
    fn a_time_or_an_offset_past_32_bits_moves_the_box_to_version_1() {
        for (earliest_presentation_time, first_offset) in
            [(u64::from(u32::MAX) + 1, 0), (0, u64::from(u32::MAX) + 1)]
        {
            let payload = encoded_payload(&segment_index(earliest_presentation_time, first_offset));

            assert_eq!(payload.first(), Some(&1));
        }
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it_at_either_version() {
        for index in [
            segment_index(90_000, 0),
            segment_index(u64::from(u32::MAX) + 1, u64::from(u32::MAX) + 1),
        ] {
            let payload = encoded_payload(&index);

            assert_eq!(SegmentIndexBox::decode_payload(&payload).unwrap(), index);
        }
    }

    #[test]
    fn a_box_read_at_version_1_with_values_within_32_bits_is_written_back_at_version_0() {
        let mut payload = b"\x01\0\0\0\0\0\0\x01\0\x01\x5f\x90".to_vec();
        payload.extend_from_slice(&90_000_u64.to_be_bytes());
        payload.extend_from_slice(&0_u64.to_be_bytes());
        payload.extend_from_slice(b"\0\0\0\0");

        let index = SegmentIndexBox::decode_payload(&payload).unwrap();

        assert_eq!(
            index,
            SegmentIndexBox::new(1, 90_000, 90_000, 0, Vec::new()).unwrap()
        );
        assert_eq!(encoded_payload(&index).first(), Some(&0));
    }

    #[test]
    fn a_count_that_disagrees_with_the_references_is_rejected() {
        let mut payload = encoded_payload(&segment_index(0, 0));
        payload
            .get_mut(22..24)
            .unwrap()
            .copy_from_slice(&3_u16.to_be_bytes());

        assert_eq!(
            SegmentIndexBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(3, 2))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = encoded_payload(&segment_index(0, 0));
        *payload.first_mut().unwrap() = 2;

        assert_eq!(
            SegmentIndexBox::decode_payload(&payload),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_field_past_its_bits_builds_no_reference() {
        let past_the_bits = [(0x8000_0000, 0, 0), (0, 8, 0), (0, 0, 0x1000_0000)];

        for (referenced_size, sap_type, sap_delta_time) in past_the_bits {
            assert_eq!(
                SegmentIndexReference::new(
                    ReferenceType::MediaContent,
                    referenced_size,
                    0,
                    false,
                    sap_type,
                    sap_delta_time,
                ),
                None
            );
        }
    }

    #[test]
    fn more_references_than_the_count_holds_build_no_box() {
        let references = vec![media_reference(1); usize::from(u16::MAX) + 1];

        assert_eq!(SegmentIndexBox::new(1, 90_000, 0, 0, references), None);
    }
}
