//! [`SegmentIndexBox`] and the [`SegmentIndexReference`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::version_width;
use crate::table::{TableEntry, entries_len, read_entries, write_entries};

/// Reference to a subsegment or to a segment index, one entry of a [`SegmentIndexBox`]
///
/// `referenced_size` is a 31-bit value, `sap_type` a 3-bit one and
/// `sap_delta_time` a 28-bit one. A value wider than its bits is refused when
/// the entry is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field.
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SegmentIndexReference {
    /// Whether the reference is to a `SegmentIndexBox`, rather than to media content such as a `MovieFragmentBox`
    pub reference_type: bool,
    /// Distance in bytes from the first byte of the referenced item to the
    /// first byte of the next, or to the end of the referenced material for the
    /// last entry
    pub referenced_size: u32,
    /// Duration of the referenced subsegment, or the sum of the durations of
    /// the referenced segment index, in the timescale of the box
    pub subsegment_duration: u32,
    /// Whether the referenced subsegments start with a stream access point
    #[doc(alias = "starts_with_SAP")]
    pub starts_with_sap: bool,
    /// Type of the stream access points, 1 to 6 as Annex I specifies, or 0
    ///
    /// Table 7 of ISO/IEC 14496-12 gives its meaning together with
    /// `starts_with_sap` and `reference_type`.
    #[doc(alias = "SAP_type")]
    pub sap_type: u8,
    /// Presentation time of the first stream access point of the referenced
    /// subsegment from its earliest presentation time, 0 where it holds none
    #[doc(alias = "SAP_delta_time")]
    pub sap_delta_time: u32,
}

impl TableEntry for SegmentIndexReference {
    const ENCODED_LEN: u64 = 4 + 4 + 4;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let sized = reader.read_u32()?;
        let subsegment_duration = reader.read_u32()?;
        let stream_access = reader.read_u32()?;
        let [sap_byte, ..] = stream_access.to_be_bytes();

        Ok(Self {
            reference_type: sized >> 31 != 0,
            referenced_size: sized & 0x7fff_ffff,
            subsegment_duration,
            starts_with_sap: sap_byte >> 7 != 0,
            sap_type: sap_byte >> 4 & 0b111,
            sap_delta_time: stream_access & 0x0fff_ffff,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let fields = [
            (self.referenced_size, 0x7fff_ffff, "referenced_size"),
            (u32::from(self.sap_type), 0b111, "SAP_type"),
            (self.sap_delta_time, 0x0fff_ffff, "SAP_delta_time"),
        ];
        if let Some((_, _, field)) = fields.iter().find(|(value, widest, _)| value > widest) {
            return Err(Error::invalid_value(field));
        }

        writer.write_u32(u32::from(self.reference_type) << 31 | self.referenced_size)?;
        writer.write_u32(self.subsegment_duration)?;
        writer.write_u32(
            u32::from(self.starts_with_sap) << 31
                | u32::from(self.sap_type) << 28
                | self.sap_delta_time,
        )
    }
}

/// [`SegmentIndexBox`] (`sidx`), ISO/IEC 14496-12 §8.14.3
///
/// A compact index of one media stream, the reference stream, within the
/// media segment it applies to: how the (sub)segment divides into
/// subsegments, each referenced as media or as a further segment index, with
/// its duration and its stream access points.
///
/// The box is written in the `version` it holds: version 0 carries
/// `earliest_presentation_time` and `first_offset` in 32 bits, version 1 in
/// 64. More references than 16 bits count are refused when the box is
/// written, with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `reference_count`.
///
/// The box holds `first_offset` and each `referenced_size` as they were
/// read: they count bytes of the file from the first byte after the box,
/// which a writer of the file adjusts when the boxes move.
#[doc(alias = "sidx")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SegmentIndexBox {
    /// Version of the box, 0 or 1, which sets the width of `earliest_presentation_time` and `first_offset`
    pub version: u8,
    /// Stream ID of the reference stream, its `track_ID` in a file of this format
    #[doc(alias = "reference_ID")]
    pub reference_id: u32,
    /// Ticks per second of the times and durations of the box
    pub timescale: u32,
    /// Earliest presentation time of any content of the reference stream in the first subsegment
    pub earliest_presentation_time: u64,
    /// Distance in bytes from the anchor point, the first byte after the box, to the first byte of the indexed material
    pub first_offset: u64,
    /// References to the subsegments, contiguous in presentation time
    pub references: Vec<SegmentIndexReference>,
}

impl BoxDefinition for SegmentIndexBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sidx");
}

impl BoxDecode for SegmentIndexBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let width = version_width(version)?;
        let reference_id = reader.read_u32()?;
        let timescale = reader.read_u32()?;
        let earliest_presentation_time = reader.read_unsigned(width)?;
        let first_offset = reader.read_unsigned(width)?;
        reader.read_bytes::<2>()?;
        let reference_count = reader.read_u16()?;
        let references = read_entries(
            reader,
            u32::from(reference_count),
            SegmentIndexReference::ENCODED_LEN,
            SegmentIndexReference::read,
        )?;

        Ok(Self {
            version,
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
        let fields_len = if self.version == 1 {
            4 + 4 + 4 + 8 + 8 + 2 + 2
        } else {
            4 + 4 + 4 + 4 + 4 + 2 + 2
        };

        entries_len(&self.references).saturating_add(fields_len)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        let reference_count = u16::try_from(self.references.len())
            .map_err(|_| Error::invalid_value("reference_count"))?;

        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_u32(self.reference_id)?;
        writer.write_u32(self.timescale)?;
        writer.write_unsigned(width, self.earliest_presentation_time)?;
        writer.write_unsigned(width, self.first_offset)?;
        writer.write_bytes(&[0; 2])?;
        writer.write_u16(reference_count)?;
        write_entries(writer, &self.references)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::{SegmentIndexBox, SegmentIndexReference};
    use crate::encoded;

    /// `sidx` of version 0, stream 1, timescale 90000, from 0 at offset 0: media of 0x1234 bytes for 180000 starting with a SAP of type 1, then an index of 0x100 bytes for 90000 with a SAP of type 2 at 3000
    const SIDX_V0: &[u8] =
        b"\0\0\0\x38sidx\0\0\0\0\0\0\0\x01\0\x01\x5f\x90\0\0\0\0\0\0\0\0\0\0\0\x02\
        \0\0\x12\x34\0\x02\xbf\x20\x90\0\0\0\
        \x80\0\x01\0\0\x01\x5f\x90\x20\0\x0b\xb8";

    /// `sidx` of version 1, stream 2, timescale 1000, from 2^32 at offset 2^33, of no reference
    const SIDX_V1: &[u8] = b"\0\0\0\x28sidx\x01\0\0\0\0\0\0\x02\0\0\x03\xe8\
        \0\0\0\x01\0\0\0\0\0\0\0\x02\0\0\0\0\0\0\0\0";

    fn compact() -> SegmentIndexBox {
        SegmentIndexBox {
            version: 0,
            reference_id: 1,
            timescale: 90000,
            earliest_presentation_time: 0,
            first_offset: 0,
            references: vec![
                SegmentIndexReference {
                    reference_type: false,
                    referenced_size: 0x1234,
                    subsegment_duration: 180_000,
                    starts_with_sap: true,
                    sap_type: 1,
                    sap_delta_time: 0,
                },
                SegmentIndexReference {
                    reference_type: true,
                    referenced_size: 0x100,
                    subsegment_duration: 90000,
                    starts_with_sap: false,
                    sap_type: 2,
                    sap_delta_time: 3000,
                },
            ],
        }
    }

    fn extended() -> SegmentIndexBox {
        SegmentIndexBox {
            version: 1,
            reference_id: 2,
            timescale: 1000,
            earliest_presentation_time: 1 << 32,
            first_offset: 1 << 33,
            references: vec![],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                SegmentIndexBox::decode(SIDX_V0),
                SegmentIndexBox::decode(SIDX_V1)
            ),
            (
                Ok((compact(), b"".as_slice())),
                Ok((extended(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (encoded(&compact()), encoded(&extended())),
            (SIDX_V0.to_vec(), SIDX_V1.to_vec())
        );
    }

    #[test]
    fn values_wider_than_their_fields_are_refused_when_written() {
        let mut wide_size = compact();
        wide_size.references = vec![SegmentIndexReference {
            referenced_size: 0x8000_0000,
            ..SegmentIndexReference::default()
        }];
        let mut wide_time = compact();
        wide_time.first_offset = 1 << 32;

        assert_eq!(
            (
                wide_size.encode(&mut [0; 64]),
                wide_time.encode(&mut [0; 64])
            ),
            (
                Err(Error::invalid_value("referenced_size")),
                Err(Error::out_of_range(1 << 32, FieldWidth::Compact))
            )
        );
    }

    #[test]
    fn a_count_of_more_references_than_the_payload_holds_is_refused() {
        assert_eq!(
            SegmentIndexBox::decode_payload(SIDX_V0.get(8..40).unwrap()),
            Err(Error::truncated_payload(24 + 2 * 12, 32))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SegmentIndexBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
