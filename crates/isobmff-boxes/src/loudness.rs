//! [`TrackLoudnessInfo`] and [`AlbumLoudnessInfo`], the loudness characteristics of a track and of its album

use alloc::vec;
use alloc::vec::Vec;
use core::ops::RangeInclusive;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::table::read_entries;

/// Bytes a loudness base takes at least in version 0, which has no `EQ_set_ID`: its fields up to `measurement_count`
const BASE_LEN_VERSION_0: u64 = 2 + 3 + 1 + 1;

/// Returns the bytes a loudness base takes at least in the version `version`, its `EQ_set_ID` included from version 1 on
const fn base_len(version: u8) -> u64 {
    if version >= 1 {
        BASE_LEN_VERSION_0 + 1
    } else {
        BASE_LEN_VERSION_0
    }
}

/// Bytes one measurement takes
const MEASUREMENT_LEN: u64 = 3;

/// Largest value of a 6-bit field
const SIX_BITS: u8 = 0x3f;

/// Largest value of a 7-bit field
const SEVEN_BITS: u8 = 0x7f;

/// Largest value of a 5-bit field
const FIVE_BITS: u8 = 0x1f;

/// Largest value of a 4-bit field
const FOUR_BITS: u8 = 0x0f;

/// Values of a signed 12-bit field
const SIGNED_TWELVE_BITS: RangeInclusive<i16> = -0x800..=0x7ff;

/// Type of the audio scene a loudness declaration of version 2 describes, a `loudnessInfoType` of ISO/IEC 23008-3
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LoudnessInfoType {
    /// `loudness_info_type` 0, the value unless the loudness processing supports another
    Type0,
    /// `loudness_info_type` 1, of a group of metadata elements
    Type1 {
        /// Identifier of the group of metadata elements of ISO/IEC 23008-3, 7 bits
        mae_group_id: u8,
    },
    /// `loudness_info_type` 2, of a group of metadata elements
    Type2 {
        /// Identifier of the group of metadata elements of ISO/IEC 23008-3, 7 bits
        mae_group_id: u8,
    },
    /// `loudness_info_type` 3, of a group preset
    Type3 {
        /// Identifier of the group preset of ISO/IEC 23008-3, 5 bits
        mae_group_preset_id: u8,
    },
}

/// One measurement of a [`LoudnessBase`], its values as ISO/IEC 23003-4 codes them
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct LoudnessMeasurement {
    /// Index of the measurement method
    pub method_definition: u8,
    /// Value the method measured, aligned to its least significant bit where the method uses fewer than 8 bits
    pub method_value: u8,
    /// Index of the measurement system, 4 bits
    pub measurement_system: u8,
    /// Reliability of the value, 4 bits: 0 unknown, 1 reported but unverified, 2 a ceiling not to exceed, 3 measured and accurate
    pub reliability: u8,
}

/// Loudness characteristics of the content under one combination of EQ, downmix and DRC, ISO/IEC 14496-12 §12.2.7
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct LoudnessBase {
    /// EQ the characteristics are after, matching an `EQ_set_ID` of the `UniDrcConfigExtension` of the track, or 0 for none; 6 bits, version 1 and 2 alone
    pub eq_set_id: u8,
    /// Downmix the characteristics are after, matching the `downmix_ID` of a `dmix` of the sample entry, or 0 for none; 7 bits
    pub downmix_id: u8,
    /// DRC the characteristics are after, matching a DRC box of the sample entry, or 0 for none; 6 bits
    pub drc_set_id: u8,
    /// Sample peak level as ISO/IEC 23003-4 codes it, a signed 12-bit value
    pub bs_sample_peak_level: i16,
    /// True peak level as ISO/IEC 23003-4 codes it, a signed 12-bit value
    pub bs_true_peak_level: i16,
    /// Index of the measurement system of the true peak level, 4 bits
    pub measurement_system_for_tp: u8,
    /// Reliability of the true peak level, 4 bits, as [`LoudnessMeasurement::reliability`] states it
    pub reliability_for_tp: u8,
    /// Measurements, in the order they came
    pub measurements: Vec<LoudnessMeasurement>,
}

/// Returns the signed value of the 12 bits of `bits`
fn signed_twelve_bits(bits: u32) -> i16 {
    let [_, _, high, low] = (bits & 0xfff).to_be_bytes();
    u16::from_be_bytes([high, low])
        .wrapping_shl(4)
        .cast_signed()
        .wrapping_shr(4)
}

/// Returns the 12 bits of the signed 12-bit value `value`
///
/// # Errors
///
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) naming `field`:
///   `value` is outside a signed 12-bit field.
fn twelve_bits(value: i16, field: &'static str) -> Result<u32, Error> {
    if !SIGNED_TWELVE_BITS.contains(&value) {
        return Err(Error::invalid_value(field));
    }

    Ok(u32::from(value.cast_unsigned()) & 0xfff)
}

/// Returns `value` if it fits in the bits `largest` sets
///
/// # Errors
///
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) naming `field`:
///   `value` is larger than `largest`.
fn fitting(value: u8, largest: u8, field: &'static str) -> Result<u8, Error> {
    if value > largest {
        return Err(Error::invalid_value(field));
    }

    Ok(value)
}

/// Fields of a `LoudnessBaseBox`, which [`TrackLoudnessInfo`] and [`AlbumLoudnessInfo`] share
struct LoudnessBaseFields {
    version: u8,
    loudness_info_type: Option<LoudnessInfoType>,
    loudness_bases: Vec<LoudnessBase>,
}

impl LoudnessBaseFields {
    /// Reads the payload of a `LoudnessBaseBox`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a field, or holds fewer bytes than its counts
    ///   state.
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion):
    ///   the version is not 0, 1 or 2.
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let (loudness_info_type, loudness_base_count) = match version {
            0 => (None, 1),
            1 => (None, reader.read_bytes::<1>()?[0] & SIX_BITS),
            2 => {
                let [packed] = *reader.read_bytes::<1>()?;
                let loudness_info_type = match packed >> 6 {
                    0 => LoudnessInfoType::Type0,
                    1 => LoudnessInfoType::Type1 {
                        mae_group_id: reader.read_bytes::<1>()?[0] & SEVEN_BITS,
                    },
                    2 => LoudnessInfoType::Type2 {
                        mae_group_id: reader.read_bytes::<1>()?[0] & SEVEN_BITS,
                    },
                    _ => LoudnessInfoType::Type3 {
                        mae_group_preset_id: reader.read_bytes::<1>()?[0] & FIVE_BITS,
                    },
                };
                (Some(loudness_info_type), packed & SIX_BITS)
            }
            _ => return Err(Error::unsupported_version(version)),
        };
        let loudness_bases = read_entries(
            reader,
            u32::from(loudness_base_count),
            base_len(version),
            |reader| {
                let eq_set_id = if version >= 1 {
                    reader.read_bytes::<1>()?[0] & SIX_BITS
                } else {
                    0
                };
                let ids = reader.read_u16()?;
                let [peak_high, peak_middle, peak_low] = *reader.read_bytes::<3>()?;
                let peaks = u32::from_be_bytes([0, peak_high, peak_middle, peak_low]);
                let [true_peak_system] = *reader.read_bytes::<1>()?;
                let [measurement_count] = *reader.read_bytes::<1>()?;
                let measurements = read_entries(
                    reader,
                    u32::from(measurement_count),
                    MEASUREMENT_LEN,
                    |reader| {
                        let [method_definition, method_value, system] =
                            *reader.read_bytes::<3>()?;
                        Ok(LoudnessMeasurement {
                            method_definition,
                            method_value,
                            measurement_system: system >> 4,
                            reliability: system & FOUR_BITS,
                        })
                    },
                )?;
                let [ids_high, ids_low] = ids.to_be_bytes();
                Ok(LoudnessBase {
                    eq_set_id,
                    downmix_id: ((ids_high << 2) | (ids_low >> 6)) & SEVEN_BITS,
                    drc_set_id: ids_low & SIX_BITS,
                    bs_sample_peak_level: signed_twelve_bits(peaks >> 12),
                    bs_true_peak_level: signed_twelve_bits(peaks),
                    measurement_system_for_tp: true_peak_system >> 4,
                    reliability_for_tp: true_peak_system & FOUR_BITS,
                    measurements,
                })
            },
        )?;

        Ok(Self {
            version,
            loudness_info_type,
            loudness_bases,
        })
    }
}

/// Returns the length of the payload of a `LoudnessBaseBox` of these fields
fn payload_len(
    version: u8,
    loudness_info_type: Option<&LoudnessInfoType>,
    loudness_bases: &[LoudnessBase],
) -> u64 {
    let header_len: u64 = match (version, loudness_info_type) {
        (0, _) => 0,
        (
            2,
            Some(
                LoudnessInfoType::Type1 { .. }
                | LoudnessInfoType::Type2 { .. }
                | LoudnessInfoType::Type3 { .. },
            ),
        ) => 2,
        _ => 1,
    };
    loudness_bases
        .iter()
        .map(|base| {
            u64::try_from(base.measurements.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(MEASUREMENT_LEN)
                .saturating_add(base_len(version))
        })
        .fold(header_len.saturating_add(4), u64::saturating_add)
}

/// Writes the payload of a `LoudnessBaseBox` of these fields
///
/// # Errors
///
/// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion):
///   the version is not 0, 1 or 2.
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) naming the field
///   of the syntax: a version 0 of other than one loudness base or of an
///   `EQ_set_ID`; a `loudness_info_type` given in other than version 2, or not
///   given in version 2; a count or a value wider than its field.
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
///   buffer ends inside a field.
fn write(
    writer: &mut FieldWriter<'_>,
    version: u8,
    loudness_info_type: Option<&LoudnessInfoType>,
    loudness_bases: &[LoudnessBase],
) -> Result<(), Error> {
    if version > 2 {
        return Err(Error::unsupported_version(version));
    }
    let loudness_base_count = u8::try_from(loudness_bases.len())
        .ok()
        .filter(|count| *count <= SIX_BITS)
        .ok_or(Error::invalid_value("loudness_base_count"))?;
    if version == 0 && loudness_base_count != 1 {
        return Err(Error::invalid_value("loudness_base_count"));
    }
    if version == 0 && loudness_bases.iter().any(|base| base.eq_set_id != 0) {
        return Err(Error::invalid_value("EQ_set_ID"));
    }
    writer.write_bytes(&FullBoxHeader::new(version, FullBoxFlags::ZERO).to_bytes())?;
    match (version, loudness_info_type) {
        (0, None) => {}
        (1, None) => writer.write_bytes(&[loudness_base_count])?,
        (2, Some(loudness_info_type)) => {
            let (type_bits, id) = match *loudness_info_type {
                LoudnessInfoType::Type0 => (0, None),
                LoudnessInfoType::Type1 { mae_group_id } => {
                    (1, Some(fitting(mae_group_id, SEVEN_BITS, "mae_group_ID")?))
                }
                LoudnessInfoType::Type2 { mae_group_id } => {
                    (2, Some(fitting(mae_group_id, SEVEN_BITS, "mae_group_ID")?))
                }
                LoudnessInfoType::Type3 {
                    mae_group_preset_id,
                } => (
                    3,
                    Some(fitting(
                        mae_group_preset_id,
                        FIVE_BITS,
                        "mae_group_preset_ID",
                    )?),
                ),
            };
            writer.write_bytes(&[(type_bits << 6) | loudness_base_count])?;
            if let Some(id) = id {
                writer.write_bytes(&[id])?;
            }
        }
        _ => return Err(Error::invalid_value("loudness_info_type")),
    }
    for base in loudness_bases {
        if version >= 1 {
            writer.write_bytes(&[fitting(base.eq_set_id, SIX_BITS, "EQ_set_ID")?])?;
        }
        let downmix_id = fitting(base.downmix_id, SEVEN_BITS, "downmix_ID")?;
        let drc_set_id = fitting(base.drc_set_id, SIX_BITS, "DRC_set_ID")?;
        writer.write_u16((u16::from(downmix_id) << 6) | u16::from(drc_set_id))?;
        let peaks = (twelve_bits(base.bs_sample_peak_level, "bs_sample_peak_level")? << 12)
            | twelve_bits(base.bs_true_peak_level, "bs_true_peak_level")?;
        let [_, peak_high, peak_middle, peak_low] = peaks.to_be_bytes();
        writer.write_bytes(&[peak_high, peak_middle, peak_low])?;
        let measurement_system_for_tp = fitting(
            base.measurement_system_for_tp,
            FOUR_BITS,
            "measurement_system_for_TP",
        )?;
        let reliability_for_tp = fitting(base.reliability_for_tp, FOUR_BITS, "reliability_for_TP")?;
        let measurement_count = u8::try_from(base.measurements.len())
            .map_err(|_| Error::invalid_value("measurement_count"))?;
        writer.write_bytes(&[
            (measurement_system_for_tp << 4) | reliability_for_tp,
            measurement_count,
        ])?;
        for measurement in &base.measurements {
            let measurement_system = fitting(
                measurement.measurement_system,
                FOUR_BITS,
                "measurement_system",
            )?;
            let reliability = fitting(measurement.reliability, FOUR_BITS, "reliability")?;
            writer.write_bytes(&[
                measurement.method_definition,
                measurement.method_value,
                (measurement_system << 4) | reliability,
            ])?;
        }
    }

    Ok(())
}

/// [`TrackLoudnessInfo`] (`tlou`), ISO/IEC 14496-12 §12.2.7
///
/// Loudness characteristics of the track, one [`LoudnessBase`] for each
/// combination of EQ, downmix and DRC it declares them after.
///
/// The box reads versions 0, 1 and 2, and is written in the `version` it
/// holds. Version 0 declares exactly one loudness base, of no `EQ_set_ID`;
/// version 2 adds the [`LoudnessInfoType`] of the audio scene. A box that
/// breaks these limits, or holds a value wider than its field, is refused
/// with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) when it is
/// written. The `flags` and the `reserved` bits are read past and written as
/// zero.
///
/// # Untrusted input
///
/// The box holds each count against the payload before reading what it
/// counts, so what it allocates is bounded by the length of the payload,
/// which the caller sets.
#[doc(alias = "tlou")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackLoudnessInfo {
    /// Version of the box, 0, 1 or 2
    pub version: u8,
    /// Type of the audio scene described, [`Some`] in version 2 alone
    pub loudness_info_type: Option<LoudnessInfoType>,
    /// Loudness characteristics, in the order they came
    pub loudness_bases: Vec<LoudnessBase>,
}

impl Default for TrackLoudnessInfo {
    fn default() -> Self {
        Self {
            version: 0,
            loudness_info_type: None,
            loudness_bases: vec![LoudnessBase::default()],
        }
    }
}

impl BoxDefinition for TrackLoudnessInfo {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tlou");
}

impl BoxDecode for TrackLoudnessInfo {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let fields = LoudnessBaseFields::read(reader)?;

        Ok(Self {
            version: fields.version,
            loudness_info_type: fields.loudness_info_type,
            loudness_bases: fields.loudness_bases,
        })
    }
}

impl BoxEncode for TrackLoudnessInfo {
    fn payload_len(&self) -> u64 {
        payload_len(
            self.version,
            self.loudness_info_type.as_ref(),
            &self.loudness_bases,
        )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write(
            writer,
            self.version,
            self.loudness_info_type.as_ref(),
            &self.loudness_bases,
        )
    }
}

/// [`AlbumLoudnessInfo`] (`alou`), ISO/IEC 14496-12 §12.2.7
///
/// Loudness characteristics of the whole album the song of the track belongs
/// to, laid out, limited and bounded as those of a [`TrackLoudnessInfo`] are.
#[doc(alias = "alou")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AlbumLoudnessInfo {
    /// Version of the box, 0, 1 or 2
    pub version: u8,
    /// Type of the audio scene described, [`Some`] in version 2 alone
    pub loudness_info_type: Option<LoudnessInfoType>,
    /// Loudness characteristics, in the order they came
    pub loudness_bases: Vec<LoudnessBase>,
}

impl Default for AlbumLoudnessInfo {
    fn default() -> Self {
        Self {
            version: 0,
            loudness_info_type: None,
            loudness_bases: vec![LoudnessBase::default()],
        }
    }
}

impl BoxDefinition for AlbumLoudnessInfo {
    const BOX_TYPE: BoxType = BoxType::compact(*b"alou");
}

impl BoxDecode for AlbumLoudnessInfo {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let fields = LoudnessBaseFields::read(reader)?;

        Ok(Self {
            version: fields.version,
            loudness_info_type: fields.loudness_info_type,
            loudness_bases: fields.loudness_bases,
        })
    }
}

impl BoxEncode for AlbumLoudnessInfo {
    fn payload_len(&self) -> u64 {
        payload_len(
            self.version,
            self.loudness_info_type.as_ref(),
            &self.loudness_bases,
        )
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write(
            writer,
            self.version,
            self.loudness_info_type.as_ref(),
            &self.loudness_bases,
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{
        AlbumLoudnessInfo, LoudnessBase, LoudnessInfoType, LoudnessMeasurement, TrackLoudnessInfo,
    };
    use crate::encoded;

    /// `tlou` of version 0: after downmix 3 and DRC 2, sample peak -1 and true peak 100, of one measurement
    pub(crate) const TLOU: &[u8] = b"\0\0\0\x16tlou\0\0\0\0\0\xc2\xff\xf0\x64\x23\x01\x01\x80\x23";

    /// `alou` of version 2, of the group of metadata elements 5, of two loudness bases of no measurement
    pub(crate) const ALOU: &[u8] = b"\0\0\0\x1ealou\x02\0\0\0\x42\x05\
        \x01\0\0\0\0\0\0\0\
        \0\0\0\0\0\0\0\0";

    pub(crate) fn track_loudness() -> TrackLoudnessInfo {
        TrackLoudnessInfo {
            version: 0,
            loudness_info_type: None,
            loudness_bases: vec![LoudnessBase {
                downmix_id: 3,
                drc_set_id: 2,
                bs_sample_peak_level: -1,
                bs_true_peak_level: 100,
                measurement_system_for_tp: 2,
                reliability_for_tp: 3,
                measurements: vec![LoudnessMeasurement {
                    method_definition: 1,
                    method_value: 0x80,
                    measurement_system: 2,
                    reliability: 3,
                }],
                ..LoudnessBase::default()
            }],
        }
    }

    pub(crate) fn album_loudness() -> AlbumLoudnessInfo {
        AlbumLoudnessInfo {
            version: 2,
            loudness_info_type: Some(LoudnessInfoType::Type1 { mae_group_id: 5 }),
            loudness_bases: vec![
                LoudnessBase {
                    eq_set_id: 1,
                    ..LoudnessBase::default()
                },
                LoudnessBase::default(),
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_the_version_it_was_written_in() {
        assert_eq!(
            (
                TrackLoudnessInfo::decode(TLOU),
                AlbumLoudnessInfo::decode(ALOU)
            ),
            (
                Ok((track_loudness(), b"".as_slice())),
                Ok((album_loudness(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (encoded(&track_loudness()), encoded(&album_loudness())),
            (TLOU.to_vec(), ALOU.to_vec())
        );
    }

    #[test]
    fn the_reserved_bits_are_read_past_and_written_as_zero() {
        let (read, _) = TrackLoudnessInfo::decode(
            b"\0\0\0\x16tlou\0\0\0\0\xe0\xc2\xff\xf0\x64\x23\x01\x01\x80\x23",
        )
        .unwrap();

        assert_eq!((encoded(&read), read), (TLOU.to_vec(), track_loudness()));
    }

    #[test]
    fn a_count_the_payload_cannot_hold_is_refused_before_a_loudness_base_is_read() {
        assert_eq!(
            TrackLoudnessInfo::decode_payload(b"\x01\0\0\0\x02\0\0\0\0\0\0\0\0"),
            Err(Error::truncated_payload(21, 13))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            TrackLoudnessInfo::decode_payload(TLOU.get(8..20).unwrap()),
            Err(Error::truncated_payload(14, 12))
        );
    }

    #[test]
    fn a_version_other_than_0_1_or_2_is_refused() {
        assert_eq!(
            AlbumLoudnessInfo::decode_payload(b"\x03\0\0\0\x01"),
            Err(Error::unsupported_version(3))
        );
    }

    #[test]
    fn a_version_0_of_other_than_one_loudness_base_is_refused_when_written() {
        let mut loudness = track_loudness();
        loudness.loudness_bases.push(LoudnessBase::default());

        assert_eq!(
            loudness.encode(&mut [0; 32]),
            Err(Error::invalid_value("loudness_base_count"))
        );
    }

    #[test]
    fn a_type_of_audio_scene_outside_version_2_is_refused_when_written() {
        let loudness = TrackLoudnessInfo {
            version: 1,
            loudness_info_type: Some(LoudnessInfoType::Type0),
            ..TrackLoudnessInfo::default()
        };

        assert_eq!(
            loudness.encode(&mut [0; 32]),
            Err(Error::invalid_value("loudness_info_type"))
        );
    }

    #[test]
    fn a_peak_level_wider_than_12_bits_is_refused_when_written() {
        let mut loudness = track_loudness();
        loudness.loudness_bases = vec![LoudnessBase {
            bs_sample_peak_level: 0x800,
            ..LoudnessBase::default()
        }];

        assert_eq!(
            loudness.encode(&mut [0; 32]),
            Err(Error::invalid_value("bs_sample_peak_level"))
        );
    }
}
