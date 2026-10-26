//! [`TrackExtendsBox`], the defaults the movie fragments of a track use

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::data_types::SampleFlags;
use crate::full_box::read_version_zero;

/// Flag mask of `is_framerate_estimate`
const IS_FRAMERATE_ESTIMATE: u8 = 0x01;

/// [`TrackExtendsBox`] (`trex`), ISO/IEC 14496-12 §8.8.3
///
/// The default values the movie fragments of one track use, so that each
/// `TrackFragmentBox` need not repeat them.
///
/// Of the `flags`, the one the specification defines is held, and the others
/// are read past and written as zero.
#[doc(alias = "trex")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackExtendsBox {
    /// Whether `media timescale / default_sample_duration` estimates the rate of samples per second of the track
    ///
    /// The estimate is informative; where it is set, `default_sample_duration`
    /// is not zero.
    pub is_framerate_estimate: bool,
    /// `track_ID` of the track in the `MovieBox` the defaults are for
    pub track_id: u32,
    /// Index of the sample entry that describes, by default, the samples in the track fragments
    pub default_sample_description_index: u32,
    /// Default duration of the samples in the track fragments
    pub default_sample_duration: u32,
    /// Default size of the samples in the track fragments
    pub default_sample_size: u32,
    /// Default flags of the samples in the track fragments
    pub default_sample_flags: SampleFlags,
}

impl BoxDefinition for TrackExtendsBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trex");
}

impl BoxDecode for TrackExtendsBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [.., flags] = read_version_zero(reader)?.bits().to_be_bytes();

        Ok(Self {
            is_framerate_estimate: flags & IS_FRAMERATE_ESTIMATE != 0,
            track_id: reader.read_u32()?,
            default_sample_description_index: reader.read_u32()?,
            default_sample_duration: reader.read_u32()?,
            default_sample_size: reader.read_u32()?,
            default_sample_flags: SampleFlags::from_bits(reader.read_u32()?),
        })
    }
}

impl BoxEncode for TrackExtendsBox {
    fn payload_len(&self) -> u64 {
        4 + 4 * 5
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let flags = if self.is_framerate_estimate {
            IS_FRAMERATE_ESTIMATE
        } else {
            0
        };
        writer.write_bytes(&[0, 0, 0, flags])?;
        writer.write_u32(self.track_id)?;
        writer.write_u32(self.default_sample_description_index)?;
        writer.write_u32(self.default_sample_duration)?;
        writer.write_u32(self.default_sample_size)?;
        writer.write_u32(self.default_sample_flags.to_bits()?)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::TrackExtendsBox;
    use crate::data_types::SampleFlags;
    use crate::encoded;

    /// `trex` of track 1, a framerate estimate of description 1, duration 1001, size 0 and flags `0x0101_0000`
    const TREX: &[u8] =
        b"\0\0\0\x20trex\0\0\0\x01\0\0\0\x01\0\0\0\x01\0\0\x03\xe9\0\0\0\0\x01\x01\0\0";

    fn track_extends() -> TrackExtendsBox {
        TrackExtendsBox {
            is_framerate_estimate: true,
            track_id: 1,
            default_sample_description_index: 1,
            default_sample_duration: 1001,
            default_sample_size: 0,
            default_sample_flags: SampleFlags::from_bits(0x0101_0000),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            TrackExtendsBox::decode(TREX),
            Ok((track_extends(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&track_extends()), TREX);
    }

    #[test]
    fn a_flag_the_specification_does_not_define_is_read_past_and_written_as_zero() {
        let (read, _) = TrackExtendsBox::decode(
            b"\0\0\0\x20trex\0\x80\0\x01\0\0\0\x01\0\0\0\x01\0\0\x03\xe9\0\0\0\0\x01\x01\0\0",
        )
        .unwrap();

        assert_eq!(encoded(&read), TREX);
    }

    #[test]
    fn a_payload_ending_inside_the_default_sample_flags_is_refused() {
        assert_eq!(
            TrackExtendsBox::decode_payload(TREX.get(8..30).unwrap()),
            Err(Error::truncated_payload(24, 22))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            TrackExtendsBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
