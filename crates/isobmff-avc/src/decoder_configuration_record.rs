//! [`AVCDecoderConfigurationRecord`], ISO/IEC 14496-15 §5.3.3.1

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter};

/// Version of the record this crate reads and writes, the only one defined
const CONFIGURATION_VERSION: u8 = 1;

/// Profiles §5.3.3.1 lays the chroma format and bit depth fields out for
const HIGH_PROFILES: [u8; 4] = [100, 110, 122, 144];

/// Most parameter sets a record can count, in the 5 bits it has for SPSs
const MAX_SEQUENCE_PARAMETER_SETS: usize = 31;

/// Length of the fields that precede the sequence parameter sets
const FIXED_FIELDS_LEN: u64 = 6;

/// Byte a NAL unit header takes, in front of the profile fields of an SPS
const NAL_UNIT_HEADER_LEN: usize = 1;

/// Length in bytes of the `NALUnitLength` field in front of every NAL unit of
/// a sample, minus one
///
/// `lengthSizeMinusOne`, ISO/IEC 14496-15 §5.3.3.1. The field is two bits
/// wide, and the spec allows 0, 1, or 3 — a length of one, two, or four bytes;
/// [`new`](Self::new) refuses 2. A file that states 2 all the same reads, and
/// [`length_size_minus_one`](Self::length_size_minus_one) reports it.
#[doc(alias = "lengthSizeMinusOne")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LengthSizeMinusOne(u8);

impl LengthSizeMinusOne {
    /// A `NALUnitLength` field of four bytes, the length most files use
    pub const FOUR_BYTES: Self = Self(3);

    /// Creates the value from the field as the spec writes it
    ///
    /// Returns `None` for a value past the two bits, or for 2, which the spec
    /// does not allow.
    #[must_use]
    pub const fn new(length_size_minus_one: u8) -> Option<Self> {
        match length_size_minus_one {
            0 | 1 | 3 => Some(Self(length_size_minus_one)),
            _other => None,
        }
    }

    /// Returns the field as the spec writes it
    #[must_use]
    pub const fn length_size_minus_one(self) -> u8 {
        self.0
    }

    /// Returns the length in bytes of the `NALUnitLength` field
    #[must_use]
    pub const fn nal_unit_length_len(self) -> u8 {
        self.0.saturating_add(1)
    }
}

/// Fields the record carries for a High, High 10, High 4:2:2, or High 4:4:4
/// Predictive profile
///
/// ISO/IEC 14496-15 §5.3.3.1 lays these out only when `AVCProfileIndication`
/// is 100, 110, 122, or 144. Deriving them means reading the SPS, which lies
/// in ISO/IEC 14496-10; a caller building a record hands them over.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct HighProfileFields {
    chroma_format: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    sequence_parameter_set_ext: Vec<Vec<u8>>,
}

impl HighProfileFields {
    /// Creates the fields from the chroma format and bit depths the SPS states,
    /// and the SPS extension NAL units
    ///
    /// Returns `None` when `chroma_format` is past its two bits, a bit depth is
    /// past its three, there are more than 255 extensions, or one is longer
    /// than `u16` can state.
    #[must_use]
    pub fn new(
        chroma_format: u8,
        bit_depth_luma_minus8: u8,
        bit_depth_chroma_minus8: u8,
        sequence_parameter_set_ext: Vec<Vec<u8>>,
    ) -> Option<Self> {
        if chroma_format > 0b11 || bit_depth_luma_minus8 > 0b111 || bit_depth_chroma_minus8 > 0b111
        {
            return None;
        }
        if !nal_units_fit(&sequence_parameter_set_ext, usize::from(u8::MAX)) {
            return None;
        }

        Some(Self {
            chroma_format,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
            sequence_parameter_set_ext,
        })
    }

    /// Returns the `chroma_format_idc` of the SPS
    #[must_use]
    pub const fn chroma_format(&self) -> u8 {
        self.chroma_format
    }

    /// Returns the `bit_depth_luma_minus8` of the SPS
    #[must_use]
    pub const fn bit_depth_luma_minus8(&self) -> u8 {
        self.bit_depth_luma_minus8
    }

    /// Returns the `bit_depth_chroma_minus8` of the SPS
    #[must_use]
    pub const fn bit_depth_chroma_minus8(&self) -> u8 {
        self.bit_depth_chroma_minus8
    }

    /// Returns the SPS extension NAL units
    #[must_use]
    pub fn sequence_parameter_set_ext(&self) -> &[Vec<u8>] {
        &self.sequence_parameter_set_ext
    }

    fn encoded_len(&self) -> u64 {
        4_u64.saturating_add(nal_units_len(&self.sequence_parameter_set_ext))
    }

    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let chroma_format = reader.read_bytes::<1>()?[0] & 0b11;
        let bit_depth_luma_minus8 = reader.read_bytes::<1>()?[0] & 0b111;
        let bit_depth_chroma_minus8 = reader.read_bytes::<1>()?[0] & 0b111;
        let count = usize::from(reader.read_bytes::<1>()?[0]);
        let sequence_parameter_set_ext = decode_nal_units(reader, count)?;

        Ok(Self {
            chroma_format,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
            sequence_parameter_set_ext,
        })
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[0b1111_1100 | self.chroma_format])?;
        writer.write_bytes(&[0b1111_1000 | self.bit_depth_luma_minus8])?;
        writer.write_bytes(&[0b1111_1000 | self.bit_depth_chroma_minus8])?;
        writer.write_bytes(&[count_byte(self.sequence_parameter_set_ext.len())])?;
        encode_nal_units(writer, &self.sequence_parameter_set_ext)
    }
}

/// Configuration a decoder of the AVC stream starts from
///
/// [`AVCDecoderConfigurationRecord`], ISO/IEC 14496-15 §5.3.3.1. Every field
/// is held but `configurationVersion`, which is 1 in the only definition of
/// the record; another version fails to read. The parameter sets are held as
/// the NAL units ISO/IEC 14496-10 lays out, header byte first, and are not
/// read.
///
/// The record is not a box: [`AVCConfigurationBox`](crate::AVCConfigurationBox)
/// carries it as the whole of its payload.
///
/// # Examples
///
/// ```
/// use isobmff_avc::{AVCDecoderConfigurationRecord, LengthSizeMinusOne};
///
/// // An SPS and a PPS as an encoder emits them, header byte first
/// let sps = vec![0x67, 0x42, 0xc0, 0x1e, 0xd9];
/// let pps = vec![0x68, 0xce, 0x3c, 0x80];
///
/// // The profile fields are taken from the SPS
/// let record = AVCDecoderConfigurationRecord::from_parameter_sets(
///     LengthSizeMinusOne::FOUR_BYTES,
///     vec![sps],
///     vec![pps],
///     None,
/// )
/// .unwrap();
///
/// assert_eq!(record.avc_profile_indication(), 0x42);
/// assert_eq!(record.profile_compatibility(), 0xc0);
/// assert_eq!(record.avc_level_indication(), 0x1e);
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AVCDecoderConfigurationRecord {
    avc_profile_indication: u8,
    profile_compatibility: u8,
    avc_level_indication: u8,
    length_size_minus_one: LengthSizeMinusOne,
    sequence_parameter_sets: Vec<Vec<u8>>,
    picture_parameter_sets: Vec<Vec<u8>>,
    high_profile_fields: Option<HighProfileFields>,
}

impl AVCDecoderConfigurationRecord {
    /// Creates the record from every field it states
    ///
    /// `high_profile_fields` may be given only for a profile §5.3.3.1 lays the
    /// fields out for — 100, 110, 122, or 144. For such a profile it may be
    /// left out, as many files leave it; the record is then written without
    /// the fields, as it was read.
    ///
    /// Returns `None` when the fields are given for another profile, when
    /// there are more than 31 SPSs or 255 PPSs, or when a parameter set is
    /// longer than `u16` can state.
    #[must_use]
    pub fn new(
        avc_profile_indication: u8,
        profile_compatibility: u8,
        avc_level_indication: u8,
        length_size_minus_one: LengthSizeMinusOne,
        sequence_parameter_sets: Vec<Vec<u8>>,
        picture_parameter_sets: Vec<Vec<u8>>,
        high_profile_fields: Option<HighProfileFields>,
    ) -> Option<Self> {
        if high_profile_fields.is_some() && !HIGH_PROFILES.contains(&avc_profile_indication) {
            return None;
        }
        if !nal_units_fit(&sequence_parameter_sets, MAX_SEQUENCE_PARAMETER_SETS)
            || !nal_units_fit(&picture_parameter_sets, usize::from(u8::MAX))
        {
            return None;
        }

        Some(Self {
            avc_profile_indication,
            profile_compatibility,
            avc_level_indication,
            length_size_minus_one,
            sequence_parameter_sets,
            picture_parameter_sets,
            high_profile_fields,
        })
    }

    /// Creates the record from the parameter sets an encoder emitted, taking
    /// the profile fields from the first SPS
    ///
    /// §5.3.3.1.3 defines `AVCProfileIndication`, `profile_compatibility`, and
    /// `AVCLevelIndication` as the three bytes that follow the NAL unit header
    /// of an SPS, so the first SPS supplies them. Nothing further of the SPS
    /// is read: `high_profile_fields` is the caller's to supply, as
    /// [`new`](Self::new) states.
    ///
    /// Returns `None` when there is no SPS or the first is shorter than those
    /// four bytes, and whenever [`new`](Self::new) would.
    #[must_use]
    pub fn from_parameter_sets(
        length_size_minus_one: LengthSizeMinusOne,
        sequence_parameter_sets: Vec<Vec<u8>>,
        picture_parameter_sets: Vec<Vec<u8>>,
        high_profile_fields: Option<HighProfileFields>,
    ) -> Option<Self> {
        let [
            avc_profile_indication,
            profile_compatibility,
            avc_level_indication,
        ] = *sequence_parameter_sets
            .first()?
            .get(NAL_UNIT_HEADER_LEN..NAL_UNIT_HEADER_LEN + 3)?
            .first_chunk::<3>()?;

        Self::new(
            avc_profile_indication,
            profile_compatibility,
            avc_level_indication,
            length_size_minus_one,
            sequence_parameter_sets,
            picture_parameter_sets,
            high_profile_fields,
        )
    }

    /// Returns the profile code of ISO/IEC 14496-10, `profile_idc`
    #[must_use]
    pub const fn avc_profile_indication(&self) -> u8 {
        self.avc_profile_indication
    }

    /// Returns the byte between `profile_idc` and `level_idc` of the SPS, the
    /// `constraint_set` flags
    #[must_use]
    pub const fn profile_compatibility(&self) -> u8 {
        self.profile_compatibility
    }

    /// Returns the level code of ISO/IEC 14496-10, `level_idc`
    #[must_use]
    pub const fn avc_level_indication(&self) -> u8 {
        self.avc_level_indication
    }

    /// Returns the length of the `NALUnitLength` field of every sample
    #[must_use]
    pub const fn length_size_minus_one(&self) -> LengthSizeMinusOne {
        self.length_size_minus_one
    }

    /// Returns the SPS NAL units, the initial set of SPSs for decoding
    #[must_use]
    pub fn sequence_parameter_sets(&self) -> &[Vec<u8>] {
        &self.sequence_parameter_sets
    }

    /// Returns the PPS NAL units, the initial set of PPSs for decoding
    #[must_use]
    pub fn picture_parameter_sets(&self) -> &[Vec<u8>] {
        &self.picture_parameter_sets
    }

    /// Returns the fields a High profile record carries, when it carries them
    #[must_use]
    pub const fn high_profile_fields(&self) -> Option<&HighProfileFields> {
        self.high_profile_fields.as_ref()
    }

    /// Returns the length the record occupies
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        let high_profile = self
            .high_profile_fields
            .as_ref()
            .map_or(0, HighProfileFields::encoded_len);

        FIXED_FIELDS_LEN
            .saturating_add(nal_units_len(&self.sequence_parameter_sets))
            .saturating_add(1)
            .saturating_add(nal_units_len(&self.picture_parameter_sets))
            .saturating_add(high_profile)
    }

    /// Reads the record off the front of `reader`
    ///
    /// The record ends where the payload does: for a High profile, the fields
    /// §5.3.3.1 lays out after the PPSs are read when bytes remain and taken
    /// as absent when none do, since files that leave them off are common.
    ///
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
    ///   record declares a `configurationVersion` other than 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a field or a parameter set.
    pub fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [
            configuration_version,
            avc_profile_indication,
            profile_compatibility,
            avc_level_indication,
            length_size_minus_one,
            sequence_parameter_set_count,
        ] = *reader.read_bytes::<6>()?;
        if configuration_version != CONFIGURATION_VERSION {
            return Err(Error::unsupported_version(configuration_version));
        }

        let sequence_parameter_sets =
            decode_nal_units(reader, usize::from(sequence_parameter_set_count & 0b1_1111))?;
        let picture_parameter_set_count = usize::from(reader.read_bytes::<1>()?[0]);
        let picture_parameter_sets = decode_nal_units(reader, picture_parameter_set_count)?;

        let high_profile_fields =
            if HIGH_PROFILES.contains(&avc_profile_indication) && !reader.remainder().is_empty() {
                Some(HighProfileFields::decode_fields(reader)?)
            } else {
                None
            };

        Ok(Self {
            avc_profile_indication,
            profile_compatibility,
            avc_level_indication,
            length_size_minus_one: LengthSizeMinusOne(length_size_minus_one & 0b11),
            sequence_parameter_sets,
            picture_parameter_sets,
            high_profile_fields,
        })
    }

    /// Writes the record into the front of `writer`
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): `writer`
    ///   has less than [`encoded_len`](Self::encoded_len) bytes left.
    pub fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[
            CONFIGURATION_VERSION,
            self.avc_profile_indication,
            self.profile_compatibility,
            self.avc_level_indication,
            0b1111_1100 | self.length_size_minus_one.0,
            0b1110_0000 | count_byte(self.sequence_parameter_sets.len()),
        ])?;
        encode_nal_units(writer, &self.sequence_parameter_sets)?;
        writer.write_bytes(&[count_byte(self.picture_parameter_sets.len())])?;
        encode_nal_units(writer, &self.picture_parameter_sets)?;

        match &self.high_profile_fields {
            Some(fields) => fields.encode_fields(writer),
            None => Ok(()),
        }
    }
}

/// Reports whether `nal_units` are few enough to count and short enough to
/// measure in the fields the record has for them
fn nal_units_fit(nal_units: &[Vec<u8>], max_count: usize) -> bool {
    nal_units.len() <= max_count
        && nal_units
            .iter()
            .all(|nal_unit| u16::try_from(nal_unit.len()).is_ok())
}

/// Returns `count` as the byte the record counts NAL units in
fn count_byte(count: usize) -> u8 {
    // Why not fail: every constructor bounds the count to what the byte holds,
    // so a degenerate value stands in for the panic the lints forbid.
    u8::try_from(count).unwrap_or(u8::MAX)
}

/// Returns the length `nal_units` occupy, each behind its 16-bit length
fn nal_units_len(nal_units: &[Vec<u8>]) -> u64 {
    nal_units.iter().fold(0_u64, |total, nal_unit| {
        total
            .saturating_add(2)
            .saturating_add(nal_unit.len() as u64)
    })
}

/// Reads `count` NAL units, each behind its 16-bit length
fn decode_nal_units(reader: &mut FieldReader<'_>, count: usize) -> Result<Vec<Vec<u8>>, Error> {
    let mut nal_units = Vec::with_capacity(count);
    for _ in 0..count {
        let length = usize::from(reader.read_u16()?);
        nal_units.push(reader.read_slice(length)?.to_vec());
    }

    Ok(nal_units)
}

/// Writes `nal_units`, each behind its 16-bit length
fn encode_nal_units(writer: &mut FieldWriter<'_>, nal_units: &[Vec<u8>]) -> Result<(), Error> {
    for nal_unit in nal_units {
        // Why not fail: every constructor bounds a NAL unit to `u16`, so a
        // degenerate value stands in for the panic the lints forbid.
        writer.write_u16(u16::try_from(nal_unit.len()).unwrap_or(u16::MAX))?;
        writer.write_slice(nal_unit)?;
    }

    Ok(())
}
