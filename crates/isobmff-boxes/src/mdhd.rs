//! [`MediaHeaderBox`], the media-independent information of the media in a track

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::data_types::LanguageCode;
use crate::full_box::version_width;

/// [`MediaHeaderBox`] (`mdhd`), ISO/IEC 14496-12 §8.4.2
///
/// Information that is media-independent and relevant to the media in a
/// track.
///
/// The box is written in the `version` it holds: version 0 carries the times
/// and the duration in 32 bits, version 1 in 64. A box read from a file keeps
/// the version it was read in.
///
/// [`Default`] is the box of version 0 with `language` [`LanguageCode::UND`]
/// and every other field zero.
#[doc(alias = "mdhd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct MediaHeaderBox {
    /// Version of the box, 0 or 1, which sets the width of the times and the duration
    pub version: u8,
    /// Creation time of the media in the track, in seconds since midnight, Jan. 1, 1904, in UTC time
    pub creation_time: u64,
    /// Most recent time the media in the track was modified, in seconds since midnight, Jan. 1, 1904, in UTC time
    pub modification_time: u64,
    /// Number of time units that pass in one second, for the media
    pub timescale: u32,
    /// Duration of the media, in `timescale` units
    ///
    /// All bits 1 — `u32::MAX` in version 0, `u64::MAX` in version 1 — means
    /// the duration cannot be determined.
    pub duration: u64,
    /// Language of the media
    pub language: LanguageCode,
}

impl BoxDefinition for MediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdhd");
}

impl BoxDecode for MediaHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let width = version_width(version)?;
        let creation_time = reader.read_unsigned(width)?;
        let modification_time = reader.read_unsigned(width)?;
        let timescale = reader.read_u32()?;
        let duration = reader.read_unsigned(width)?;
        let language = LanguageCode::from_bits(reader.read_u16()?);
        reader.read_bytes::<2>()?;

        Ok(Self {
            version,
            creation_time,
            modification_time,
            timescale,
            duration,
            language,
        })
    }
}

impl BoxEncode for MediaHeaderBox {
    fn payload_len(&self) -> u64 {
        let times_len: u64 = if self.version == 1 {
            8 + 8 + 4 + 8
        } else {
            4 + 4 + 4 + 4
        };

        times_len.saturating_add(4 + 2 + 2)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_unsigned(width, self.creation_time)?;
        writer.write_unsigned(width, self.modification_time)?;
        writer.write_u32(self.timescale)?;
        writer.write_unsigned(width, self.duration)?;
        writer.write_u16(self.language.bits())?;
        writer.write_bytes(&[0; 2])
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::MediaHeaderBox;
    use crate::data_types::LanguageCode;
    use crate::encoded;

    /// `mdhd` of version 0: created at 1, modified at 2, timescale 48000, duration 96000 and language `eng`
    const MDHD_V0: &[u8] =
        b"\0\0\0\x20mdhd\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\xbb\x80\0\x01\x77\0\x15\xc7\0\0";

    /// `mdhd` of version 1: created at 1, modified at 2, timescale 48000, duration 2^33 and language `eng`
    const MDHD_V1: &[u8] = b"\0\0\0\x2cmdhd\x01\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\x02\
        \0\0\xbb\x80\0\0\0\x02\0\0\0\0\x15\xc7\0\0";

    fn media_header(version: u8, duration: u64) -> MediaHeaderBox {
        MediaHeaderBox {
            version,
            creation_time: 1,
            modification_time: 2,
            timescale: 48000,
            duration,
            language: LanguageCode::new(*b"eng").unwrap(),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                MediaHeaderBox::decode(MDHD_V0),
                MediaHeaderBox::decode(MDHD_V1)
            ),
            (
                Ok((media_header(0, 96000), b"".as_slice())),
                Ok((media_header(1, 1 << 33), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (
                encoded(&media_header(0, 96000)),
                encoded(&media_header(1, 1 << 33))
            ),
            (MDHD_V0.to_vec(), MDHD_V1.to_vec())
        );
    }

    #[test]
    fn the_reserved_bit_and_pre_defined_are_read_past_and_written_as_zero() {
        let (read, _) = MediaHeaderBox::decode(
            b"\0\0\0\x20mdhd\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\xbb\x80\0\x01\x77\0\x95\xc7\x12\x34",
        )
        .unwrap();

        assert_eq!(encoded(&read), MDHD_V0);
    }

    #[test]
    fn a_language_that_spells_no_letters_is_kept_as_found() {
        let bytes = b"\0\0\0\x20mdhd\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\xbb\x80\0\x01\x77\0\0\0\0\0";
        let (read, _) = MediaHeaderBox::decode(bytes).unwrap();

        assert_eq!(
            (read.language.letters(), encoded(&read)),
            (*b"```", bytes.to_vec())
        );
    }

    #[test]
    fn version_0_refuses_a_duration_wider_than_32_bits() {
        assert_eq!(
            media_header(0, 1 << 33).encode(&mut [0; 32]),
            Err(Error::out_of_range(1 << 33, FieldWidth::Compact))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_language_is_refused() {
        assert_eq!(
            MediaHeaderBox::decode_payload(MDHD_V0.get(8..29).unwrap()),
            Err(Error::truncated_payload(22, 21))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        assert_eq!(
            (
                MediaHeaderBox::decode_payload(b"\x02\0\0\0"),
                media_header(2, 96000).encode(&mut [0; 32])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
