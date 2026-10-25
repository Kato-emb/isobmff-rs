//! [`CompositionToDecodeBox`]

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::version_width;

/// [`CompositionToDecodeBox`] (`cslg`), ISO/IEC 14496-12 §8.6.1.4
///
/// How the composition timeline of a track relates to its decoding timeline
/// where composition offsets are signed. The box is advisory: its values can
/// be computed by inspecting the samples, and apply to the entire media, not
/// just what the edits select.
///
/// The box is written in the `version` it holds, 32-bit fields under version
/// 0 and 64-bit ones under version 1. A value that version 0 does not carry is
/// refused when the box is written, with
/// [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange).
#[doc(alias = "cslg")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct CompositionToDecodeBox {
    /// Version of the box: 0 for 32-bit fields, 1 for 64-bit ones
    pub version: u8,
    /// Shift that, added to every composition timestamp, puts it at or after
    /// the decoding timestamp of its sample
    pub composition_to_dts_shift: i64,
    /// Smallest composition offset of the track
    pub least_decode_to_display_delta: i64,
    /// Largest composition offset of the track
    pub greatest_decode_to_display_delta: i64,
    /// Smallest composition timestamp of any sample of the track
    pub composition_start_time: i64,
    /// Composition timestamp plus composition duration of the sample of the
    /// largest composition timestamp; 0 when it is unknown
    pub composition_end_time: i64,
}

impl BoxDefinition for CompositionToDecodeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"cslg");
}

impl BoxDecode for CompositionToDecodeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let width = version_width(version)?;

        Ok(Self {
            version,
            composition_to_dts_shift: reader.read_signed(width)?,
            least_decode_to_display_delta: reader.read_signed(width)?,
            greatest_decode_to_display_delta: reader.read_signed(width)?,
            composition_start_time: reader.read_signed(width)?,
            composition_end_time: reader.read_signed(width)?,
        })
    }
}

impl BoxEncode for CompositionToDecodeBox {
    fn payload_len(&self) -> u64 {
        if self.version == 0 {
            4 + 5 * 4
        } else {
            4 + 5 * 8
        }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;

        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_signed(width, self.composition_to_dts_shift)?;
        writer.write_signed(width, self.least_decode_to_display_delta)?;
        writer.write_signed(width, self.greatest_decode_to_display_delta)?;
        writer.write_signed(width, self.composition_start_time)?;
        writer.write_signed(width, self.composition_end_time)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::CompositionToDecodeBox;
    use crate::encoded;

    /// `cslg` of version 0: shift 10, deltas -10 and 30, composition from 0 to 140
    const CSLG_COMPACT: &[u8] =
        b"\0\0\0\x20cslg\0\0\0\0\0\0\0\x0a\xff\xff\xff\xf6\0\0\0\x1e\0\0\0\0\0\0\0\x8c";

    /// The same `cslg` in version 1
    const CSLG_EXTENDED: &[u8] = b"\0\0\0\x34cslg\x01\0\0\0\
        \0\0\0\0\0\0\0\x0a\xff\xff\xff\xff\xff\xff\xff\xf6\0\0\0\0\0\0\0\x1e\
        \0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x8c";

    fn composition_to_decode(version: u8) -> CompositionToDecodeBox {
        CompositionToDecodeBox {
            version,
            composition_to_dts_shift: 10,
            least_decode_to_display_delta: -10,
            greatest_decode_to_display_delta: 30,
            composition_start_time: 0,
            composition_end_time: 140,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                CompositionToDecodeBox::decode(CSLG_COMPACT),
                CompositionToDecodeBox::decode(CSLG_EXTENDED)
            ),
            (
                Ok((composition_to_decode(0), b"".as_slice())),
                Ok((composition_to_decode(1), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                encoded(&composition_to_decode(0)),
                encoded(&composition_to_decode(1))
            ),
            (CSLG_COMPACT.to_vec(), CSLG_EXTENDED.to_vec())
        );
    }

    #[test]
    fn a_value_version_0_does_not_carry_is_refused_when_written() {
        let mut wide = composition_to_decode(0);
        wide.composition_end_time = 1 << 31;

        assert_eq!(
            wide.encode(&mut [0; 32]),
            Err(Error::out_of_range(1 << 31, FieldWidth::Compact))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            CompositionToDecodeBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::truncated_payload(12, 8))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            CompositionToDecodeBox::decode_payload(b"\x02\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }
}
