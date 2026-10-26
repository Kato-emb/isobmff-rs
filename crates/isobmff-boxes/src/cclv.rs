//! [`ContentColourVolumeBox`], the colour volume of the pictures of a video

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// Bit of `ccv_primaries_present_flag` in the byte of flags
const PRIMARIES_PRESENT: u8 = 0x20;

/// Bit of `ccv_min_luminance_value_present_flag` in the byte of flags
const MIN_LUMINANCE_PRESENT: u8 = 0x10;

/// Bit of `ccv_max_luminance_value_present_flag` in the byte of flags
const MAX_LUMINANCE_PRESENT: u8 = 0x08;

/// Bit of `ccv_avg_luminance_value_present_flag` in the byte of flags
const AVG_LUMINANCE_PRESENT: u8 = 0x04;

/// Chromaticity coordinates of the three colour primaries of a [`ContentColourVolumeBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ContentColourVolumePrimaries {
    /// Normalized x chromaticity coordinate of each primary, in units of 0.00002
    pub ccv_primaries_x: [i32; 3],
    /// Normalized y chromaticity coordinate of each primary, in units of 0.00002
    pub ccv_primaries_y: [i32; 3],
}

/// [`ContentColourVolumeBox`] (`cclv`), ISO/IEC 14496-12 §12.1.8
///
/// Colour volume characteristics of the pictures, in terms of a nominal range,
/// as the content colour volume SEI message of ITU-T H.265 | ISO/IEC 23008-2
/// gives them. A value is [`Some`] where its present flag is set, and each
/// flag is written from whether its value is there.
///
/// The bits of `ccv_cancel_flag` and `ccv_persistence_flag`, which a sample
/// entry sets to 0, and the two `reserved` bits are read past and written as
/// zero.
#[doc(alias = "cclv")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ContentColourVolumeBox {
    /// Colour primaries of the pictures
    pub ccv_primaries: Option<ContentColourVolumePrimaries>,
    /// Smallest luminance of the pictures, in units of 0.0000001 candelas per square metre
    pub ccv_min_luminance_value: Option<u32>,
    /// Largest luminance of the pictures, in units of 0.0000001 candelas per square metre
    pub ccv_max_luminance_value: Option<u32>,
    /// Average luminance of the pictures, in units of 0.0000001 candelas per square metre
    pub ccv_avg_luminance_value: Option<u32>,
}

impl BoxDefinition for ContentColourVolumeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"cclv");
}

impl BoxDecode for ContentColourVolumeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [flags] = *reader.read_bytes::<1>()?;
        let ccv_primaries = if flags & PRIMARIES_PRESENT == 0 {
            None
        } else {
            let mut primaries = ContentColourVolumePrimaries::default();
            for (primary_x, primary_y) in primaries
                .ccv_primaries_x
                .iter_mut()
                .zip(&mut primaries.ccv_primaries_y)
            {
                *primary_x = reader.read_i32()?;
                *primary_y = reader.read_i32()?;
            }
            Some(primaries)
        };
        let mut read_if = |bit: u8| {
            if flags & bit == 0 {
                Ok(None)
            } else {
                reader.read_u32().map(Some)
            }
        };

        Ok(Self {
            ccv_primaries,
            ccv_min_luminance_value: read_if(MIN_LUMINANCE_PRESENT)?,
            ccv_max_luminance_value: read_if(MAX_LUMINANCE_PRESENT)?,
            ccv_avg_luminance_value: read_if(AVG_LUMINANCE_PRESENT)?,
        })
    }
}

impl BoxEncode for ContentColourVolumeBox {
    fn payload_len(&self) -> u64 {
        [
            self.ccv_primaries.map(|_| 3 * 8),
            self.ccv_min_luminance_value.map(|_| 4),
            self.ccv_max_luminance_value.map(|_| 4),
            self.ccv_avg_luminance_value.map(|_| 4),
        ]
        .into_iter()
        .flatten()
        .fold(1, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let luminance_values = [
            (self.ccv_min_luminance_value, MIN_LUMINANCE_PRESENT),
            (self.ccv_max_luminance_value, MAX_LUMINANCE_PRESENT),
            (self.ccv_avg_luminance_value, AVG_LUMINANCE_PRESENT),
        ];
        let flags = [(self.ccv_primaries.is_some(), PRIMARIES_PRESENT)]
            .into_iter()
            .chain(luminance_values.map(|(value, bit)| (value.is_some(), bit)))
            .filter(|(present, _)| *present)
            .fold(0, |flags, (_, bit)| flags | bit);
        writer.write_bytes(&[flags])?;
        if let Some(primaries) = &self.ccv_primaries {
            for (primary_x, primary_y) in primaries
                .ccv_primaries_x
                .iter()
                .zip(&primaries.ccv_primaries_y)
            {
                writer.write_i32(*primary_x)?;
                writer.write_i32(*primary_y)?;
            }
        }
        for value in luminance_values.iter().filter_map(|(value, _)| *value) {
            writer.write_u32(value)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, Error};

    use super::{ContentColourVolumeBox, ContentColourVolumePrimaries};
    use crate::encoded;

    /// `cclv` of the primaries of BT.709 and an average luminance of 100 cd/m²
    const CCLV: &[u8] = b"\0\0\0\x25cclv\x24\
        \0\0\x7d\0\0\0\x41\x1a\0\0\x3a\x98\0\0\x75\x30\0\0\x1d\x4c\0\0\x0b\xb8\x3b\x9a\xca\0";

    fn content_colour_volume() -> ContentColourVolumeBox {
        ContentColourVolumeBox {
            ccv_primaries: Some(ContentColourVolumePrimaries {
                ccv_primaries_x: [32000, 15000, 7500],
                ccv_primaries_y: [16666, 30000, 3000],
            }),
            ccv_avg_luminance_value: Some(1_000_000_000),
            ..ContentColourVolumeBox::default()
        }
    }

    #[test]
    fn reads_the_values_whose_present_flag_is_set() {
        assert_eq!(
            ContentColourVolumeBox::decode(CCLV),
            Ok((content_colour_volume(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_a_present_flag_for_each_value_it_holds() {
        assert_eq!(encoded(&content_colour_volume()), CCLV);
    }

    #[test]
    fn the_cancel_persistence_and_reserved_bits_are_read_past_and_written_as_zero() {
        let (read, _) = ContentColourVolumeBox::decode(b"\0\0\0\x0dcclv\xd3\0\0\0\x07").unwrap();
        let expected = ContentColourVolumeBox {
            ccv_min_luminance_value: Some(7),
            ..ContentColourVolumeBox::default()
        };

        assert_eq!(
            (encoded(&read), read),
            (b"\0\0\0\x0dcclv\x10\0\0\0\x07".to_vec(), expected)
        );
    }

    #[test]
    fn a_payload_ending_inside_a_value_whose_flag_is_set_is_refused() {
        assert_eq!(
            ContentColourVolumeBox::decode_payload(b"\x08\0\0"),
            Err(Error::truncated_payload(5, 3))
        );
    }
}
