//! [`ColourInformationBox`], the colour information of the pictures of a video

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
};

/// `colour_type` of on-screen colours
const NCLX: FourCc = FourCc::new(*b"nclx");

/// `colour_type` of a restricted ICC profile
const RICC: FourCc = FourCc::new(*b"rICC");

/// `colour_type` of an unrestricted ICC profile
const PROF: FourCc = FourCc::new(*b"prof");

/// Bit of `full_range_flag`, the most significant of the byte it shares with `reserved`
const FULL_RANGE_FLAG: u8 = 0x80;

/// [`ColourInformationBox`] (`colr`), ISO/IEC 14496-12 §12.1.5
///
/// Colour information of the pictures, advisory, of the type `colour_type`
/// names: one variant for each type ISO/IEC 14496-12 lays out, and
/// [`Other`](Self::Other) for a type it does not, whose bytes are kept. Where
/// the video bitstream also carries colour information, the box takes
/// precedence.
#[doc(alias = "colr")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ColourInformationBox {
    /// On-screen colours (`nclx`), the values ISO/IEC 23091-2 defines
    ///
    /// The 7 bits of `reserved` are read past and written as zero.
    OnScreenColours {
        /// `ColourPrimaries` of ISO/IEC 23091-2
        colour_primaries: u16,
        /// `TransferCharacteristics` of ISO/IEC 23091-2
        transfer_characteristics: u16,
        /// `MatrixCoefficients` of ISO/IEC 23091-2
        matrix_coefficients: u16,
        /// `VideoFullRangeFlag` of ISO/IEC 23091-2
        full_range_flag: bool,
    },
    /// Restricted ICC profile (`rICC`): a monochrome or three-component matrix-based input profile of ISO 15076-1
    RestrictedIccProfile {
        /// ICC profile of ISO 15076-1 or ICC.1
        icc_profile: Vec<u8>,
    },
    /// Unrestricted ICC profile (`prof`)
    UnrestrictedIccProfile {
        /// ICC profile of ISO 15076-1 or ICC.1
        icc_profile: Vec<u8>,
    },
    /// Colour information of a `colour_type` ISO/IEC 14496-12 does not lay out, which a reader may ignore
    ///
    /// A `colour_type` of `nclx`, `rICC` or `prof` here is written as given,
    /// and reads back as the variant of that type.
    Other {
        /// Type of the colour information
        colour_type: FourCc,
        /// Bytes after `colour_type`
        remaining_data: Vec<u8>,
    },
}

impl BoxDefinition for ColourInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"colr");
}

impl BoxDecode for ColourInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let colour_type = FourCc::new(*reader.read_bytes::<4>()?);

        Ok(match colour_type {
            NCLX => Self::OnScreenColours {
                colour_primaries: reader.read_u16()?,
                transfer_characteristics: reader.read_u16()?,
                matrix_coefficients: reader.read_u16()?,
                full_range_flag: reader.read_bytes::<1>()?[0] & FULL_RANGE_FLAG != 0,
            },
            RICC => Self::RestrictedIccProfile {
                icc_profile: reader.take_remainder().to_vec(),
            },
            PROF => Self::UnrestrictedIccProfile {
                icc_profile: reader.take_remainder().to_vec(),
            },
            _ => Self::Other {
                colour_type,
                remaining_data: reader.take_remainder().to_vec(),
            },
        })
    }
}

impl BoxEncode for ColourInformationBox {
    fn payload_len(&self) -> u64 {
        let rest = match self {
            Self::OnScreenColours { .. } => 3 * 2 + 1,
            Self::RestrictedIccProfile { icc_profile: bytes }
            | Self::UnrestrictedIccProfile { icc_profile: bytes }
            | Self::Other {
                remaining_data: bytes,
                ..
            } => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        };

        rest.saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let (colour_type, bytes) = match self {
            Self::OnScreenColours {
                colour_primaries,
                transfer_characteristics,
                matrix_coefficients,
                full_range_flag,
            } => {
                writer.write_bytes(NCLX.as_bytes())?;
                writer.write_u16(*colour_primaries)?;
                writer.write_u16(*transfer_characteristics)?;
                writer.write_u16(*matrix_coefficients)?;
                return writer.write_bytes(&[if *full_range_flag { FULL_RANGE_FLAG } else { 0 }]);
            }
            Self::RestrictedIccProfile { icc_profile } => (RICC, icc_profile),
            Self::UnrestrictedIccProfile { icc_profile } => (PROF, icc_profile),
            Self::Other {
                colour_type,
                remaining_data,
            } => (*colour_type, remaining_data),
        };
        writer.write_bytes(colour_type.as_bytes())?;
        writer.write_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error, FourCc};

    use super::ColourInformationBox;
    use crate::encoded;

    /// `colr` of the on-screen colours of BT.709 in full range
    const NCLX: &[u8] = b"\0\0\0\x13colrnclx\0\x01\0\x01\0\x01\x80";

    /// `colr` of an unrestricted ICC profile of three bytes
    const PROF: &[u8] = b"\0\0\0\x0fcolrprof\x01\x02\x03";

    /// `colr` of the QuickTime type `nclc`, which ISO/IEC 14496-12 does not lay out
    const NCLC: &[u8] = b"\0\0\0\x12colrnclc\0\x01\0\x01\0\x01";

    fn on_screen_colours() -> ColourInformationBox {
        ColourInformationBox::OnScreenColours {
            colour_primaries: 1,
            transfer_characteristics: 1,
            matrix_coefficients: 1,
            full_range_flag: true,
        }
    }

    #[test]
    fn reads_the_variant_of_its_colour_type() {
        assert_eq!(
            [NCLX, PROF, NCLC].map(ColourInformationBox::decode),
            [
                Ok((on_screen_colours(), b"".as_slice())),
                Ok((
                    ColourInformationBox::UnrestrictedIccProfile {
                        icc_profile: vec![1, 2, 3]
                    },
                    b"".as_slice()
                )),
                Ok((
                    ColourInformationBox::Other {
                        colour_type: FourCc::new(*b"nclc"),
                        remaining_data: vec![0, 1, 0, 1, 0, 1]
                    },
                    b"".as_slice()
                )),
            ]
        );
    }

    #[test]
    fn writes_the_bytes_of_its_colour_type() {
        let restricted = ColourInformationBox::RestrictedIccProfile {
            icc_profile: vec![1, 2, 3],
        };

        assert_eq!(
            (encoded(&on_screen_colours()), encoded(&restricted)),
            (NCLX.to_vec(), b"\0\0\0\x0fcolrrICC\x01\x02\x03".to_vec())
        );
    }

    #[test]
    fn the_reserved_bits_of_on_screen_colours_are_read_past_and_written_as_zero() {
        let (read, _) =
            ColourInformationBox::decode(b"\0\0\0\x13colrnclx\0\x01\0\x01\0\x01\x7f").unwrap();

        assert_eq!(encoded(&read), b"\0\0\0\x13colrnclx\0\x01\0\x01\0\x01\0");
    }

    #[test]
    fn on_screen_colours_ending_inside_a_field_are_refused() {
        assert_eq!(
            ColourInformationBox::decode_payload(b"nclx\0\x01\0\x01\0\x01"),
            Err(Error::truncated_payload(11, 10))
        );
    }
}
