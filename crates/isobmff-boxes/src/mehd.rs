//! [`MovieExtendsHeaderBox`], the duration of a fragmented movie

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::version_width;

/// [`MovieExtendsHeaderBox`] (`mehd`), ISO/IEC 14496-12 §8.8.2
///
/// The overall duration of a fragmented movie, its fragments included.
/// Without this box, the duration is computed by examining each fragment.
///
/// The box is written in the `version` it holds: version 0 carries the
/// duration in 32 bits, version 1 in 64. A box read from a file keeps the
/// version it was read in.
#[doc(alias = "mehd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MovieExtendsHeaderBox {
    /// Version of the box, 0 or 1, which sets the width of the duration
    pub version: u8,
    /// Duration of the whole movie, fragments included, in the timescale of the `MovieHeaderBox`
    ///
    /// It is the duration of the longest track.
    pub fragment_duration: u64,
}

impl BoxDefinition for MovieExtendsHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mehd");
}

impl BoxDecode for MovieExtendsHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let fragment_duration = reader.read_unsigned(version_width(version)?)?;

        Ok(Self {
            version,
            fragment_duration,
        })
    }
}

impl BoxEncode for MovieExtendsHeaderBox {
    fn payload_len(&self) -> u64 {
        if self.version == 1 { 4 + 8 } else { 4 + 4 }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_unsigned(width, self.fragment_duration)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::MovieExtendsHeaderBox;
    use crate::encoded;

    /// `mehd` of version 0 and a duration of 90000
    const MEHD_V0: &[u8] = b"\0\0\0\x10mehd\0\0\0\0\0\x01\x5f\x90";

    /// `mehd` of version 1 and a duration of 2^33
    const MEHD_V1: &[u8] = b"\0\0\0\x14mehd\x01\0\0\0\0\0\0\x02\0\0\0\0";

    fn header(version: u8, fragment_duration: u64) -> MovieExtendsHeaderBox {
        MovieExtendsHeaderBox {
            version,
            fragment_duration,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                MovieExtendsHeaderBox::decode(MEHD_V0),
                MovieExtendsHeaderBox::decode(MEHD_V1)
            ),
            (
                Ok((header(0, 90000), b"".as_slice())),
                Ok((header(1, 1 << 33), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (encoded(&header(0, 90000)), encoded(&header(1, 1 << 33))),
            (MEHD_V0.to_vec(), MEHD_V1.to_vec())
        );
    }

    #[test]
    fn version_0_refuses_a_duration_wider_than_32_bits() {
        assert_eq!(
            header(0, 1 << 33).encode(&mut [0; 16]),
            Err(Error::out_of_range(1 << 33, FieldWidth::Compact))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_duration_is_refused() {
        assert_eq!(
            MovieExtendsHeaderBox::decode_payload(b"\x01\0\0\0\0\0\0\x02"),
            Err(Error::truncated_payload(12, 8))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        assert_eq!(
            (
                MovieExtendsHeaderBox::decode_payload(b"\x02\0\0\0"),
                header(2, 0).encode(&mut [0; 16])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
