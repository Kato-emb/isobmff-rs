//! [`TrackFragmentBaseMediaDecodeTimeBox`], the decoding timestamp a track fragment starts at

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::version_width;

/// [`TrackFragmentBaseMediaDecodeTimeBox`] (`tfdt`), ISO/IEC 14496-12 §8.8.12
///
/// The absolute decoding timestamp, on the decoding timeline, of the first
/// sample in decoding order in the track fragment.
///
/// Where the time exceeds the sum of the sample durations of the samples in
/// the preceding movie and movie fragments, the duration of the last sample
/// before the track fragment is extended so that the sum equals it; the time
/// is never less than that sum. Where no sample precedes the track fragment
/// in the track, the time is the decoding timestamp of the first sample of
/// the track. A track fragment with no samples may so set the duration of the
/// last sample.
///
/// The box is written in the `version` it holds: version 0 carries the time
/// in 32 bits, version 1 in 64. A box read from a file keeps the version it
/// was read in.
#[doc(alias = "tfdt")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentBaseMediaDecodeTimeBox {
    /// Version of the box, 0 or 1, which sets the width of the time
    pub version: u8,
    /// Sum of the decode durations of all earlier samples in the media, in the timescale of the media
    ///
    /// It does not include the samples of the track fragment.
    #[doc(alias = "baseMediaDecodeTime")]
    pub base_media_decode_time: u64,
}

impl BoxDefinition for TrackFragmentBaseMediaDecodeTimeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tfdt");
}

impl BoxDecode for TrackFragmentBaseMediaDecodeTimeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let base_media_decode_time = reader.read_unsigned(version_width(version)?)?;

        Ok(Self {
            version,
            base_media_decode_time,
        })
    }
}

impl BoxEncode for TrackFragmentBaseMediaDecodeTimeBox {
    fn payload_len(&self) -> u64 {
        if self.version == 1 { 4 + 8 } else { 4 + 4 }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_unsigned(width, self.base_media_decode_time)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::TrackFragmentBaseMediaDecodeTimeBox;
    use crate::encoded;

    /// `tfdt` of version 0 and a time of 90000
    const TFDT_V0: &[u8] = b"\0\0\0\x10tfdt\0\0\0\0\0\x01\x5f\x90";

    /// `tfdt` of version 1 and a time of 2^33
    const TFDT_V1: &[u8] = b"\0\0\0\x14tfdt\x01\0\0\0\0\0\0\x02\0\0\0\0";

    fn decode_time(
        version: u8,
        base_media_decode_time: u64,
    ) -> TrackFragmentBaseMediaDecodeTimeBox {
        TrackFragmentBaseMediaDecodeTimeBox {
            version,
            base_media_decode_time,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                TrackFragmentBaseMediaDecodeTimeBox::decode(TFDT_V0),
                TrackFragmentBaseMediaDecodeTimeBox::decode(TFDT_V1)
            ),
            (
                Ok((decode_time(0, 90000), b"".as_slice())),
                Ok((decode_time(1, 1 << 33), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (
                encoded(&decode_time(0, 90000)),
                encoded(&decode_time(1, 1 << 33))
            ),
            (TFDT_V0.to_vec(), TFDT_V1.to_vec())
        );
    }

    #[test]
    fn version_0_refuses_a_time_wider_than_32_bits() {
        assert_eq!(
            decode_time(0, 1 << 33).encode(&mut [0; 16]),
            Err(Error::out_of_range(1 << 33, FieldWidth::Compact))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_time_is_refused() {
        assert_eq!(
            TrackFragmentBaseMediaDecodeTimeBox::decode_payload(b"\x01\0\0\0\0\0\0\x02"),
            Err(Error::truncated_payload(12, 8))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        assert_eq!(
            (
                TrackFragmentBaseMediaDecodeTimeBox::decode_payload(b"\x02\0\0\0"),
                decode_time(2, 0).encode(&mut [0; 16])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
