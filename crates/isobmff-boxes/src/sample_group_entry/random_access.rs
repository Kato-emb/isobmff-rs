//! The entries of the sample groups that mark where decoding can start: [`RollRecoveryEntry`], [`AudioPreRollEntry`], [`AlternativeStartupEntry`], [`VisualRandomAccessEntry`], [`SapEntry`], [`VisualDrapEntry`] and [`VisualEdrapEntry`]

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter};

use crate::table::{TableEntry, entries_len, read_entries, read_to_end, write_entries};

/// [`RollRecoveryEntry`] (`roll`), ISO/IEC 14496-12 §10.1
///
/// The samples from which decoding recovers after a number of samples. The
/// specification defines it as a `VisualRollRecoveryEntry` for video and an
/// `AudioRollRecoveryEntry` for audio, of the same syntax; the handler type
/// of the track tells which. A `roll_distance` of 0 is refused when written,
/// with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue).
#[doc(
    alias = "roll",
    alias = "VisualRollRecoveryEntry",
    alias = "AudioRollRecoveryEntry"
)]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct RollRecoveryEntry {
    /// Number of samples to decode for a sample to be decoded correctly:
    /// after the member sample where positive, before it where negative
    pub roll_distance: i16,
}

/// [`AudioPreRollEntry`] (`prol`), ISO/IEC 14496-12 §10.1
///
/// The independently decodable samples of an audio stream, from which the
/// decoder output is correct after pre-rolling by a number of samples. A
/// `roll_distance` that is not positive is refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue).
#[doc(alias = "prol")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct AudioPreRollEntry {
    /// Number of samples to decode before the decoder output is correct, positive
    pub roll_distance: i16,
}

/// Piece of an alternative startup sequence of one sample output rate, of an [`AlternativeStartupEntry`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct AlternativeStartupPiece {
    /// Number of the samples of the piece that are output
    pub num_output_samples: u16,
    /// Number of samples the piece spans, those not in the sequence included
    pub num_total_samples: u16,
}

impl TableEntry for AlternativeStartupPiece {
    const ENCODED_LEN: u64 = 4;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            num_output_samples: reader.read_u16()?,
            num_total_samples: reader.read_u16()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u16(self.num_output_samples)?;
        writer.write_u16(self.num_total_samples)
    }
}

/// [`AlternativeStartupEntry`] (`alst`), ISO/IEC 14496-12 §10.3
///
/// The samples of an alternative startup sequence, decoding which starts the
/// rendering earlier than decoding all of them. `roll_count` is the number of
/// `sample_offset`s; more than 16 bits count are refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `roll_count`.
/// The pieces run to the end of the entry, so it takes the length a
/// `SampleGroupDescriptionBox` of version 1 or more states.
#[doc(alias = "alst")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct AlternativeStartupEntry {
    /// Index, from 1, of the first sample of the sequence to output
    pub first_output_sample: u16,
    /// `sample_offset` of each sample of the sequence: its decoding time delta
    /// from its regular decoding time
    pub sample_offsets: Vec<u32>,
    /// Pieces of the sequence of one sample output rate, from the first sample output
    pub pieces: Vec<AlternativeStartupPiece>,
}

impl AlternativeStartupEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let roll_count = reader.read_u16()?;
        let first_output_sample = reader.read_u16()?;
        let sample_offsets = read_entries(
            reader,
            u32::from(roll_count),
            u32::ENCODED_LEN,
            <u32 as TableEntry>::read,
        )?;

        Ok(Self {
            first_output_sample,
            sample_offsets,
            pieces: read_to_end(reader)?,
        })
    }

    pub(super) fn encoded_len(&self) -> u64 {
        entries_len(&self.sample_offsets)
            .saturating_add(entries_len(&self.pieces))
            .saturating_add(2 + 2)
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let roll_count = u16::try_from(self.sample_offsets.len())
            .map_err(|_| Error::invalid_value("roll_count"))?;
        writer.write_u16(roll_count)?;
        writer.write_u16(self.first_output_sample)?;
        write_entries(writer, &self.sample_offsets)?;
        write_entries(writer, &self.pieces)
    }
}

/// [`VisualRandomAccessEntry`] (`rap `), ISO/IEC 14496-12 §10.4
///
/// The open random-access points. A `num_leading_samples` past 7 bits is
/// refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue).
#[doc(alias = "rap")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct VisualRandomAccessEntry {
    /// Whether `num_leading_samples` is known
    pub num_leading_samples_known: bool,
    /// Number of leading samples of each sample of the group
    pub num_leading_samples: u8,
}

impl VisualRandomAccessEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [byte] = *reader.read_bytes::<1>()?;

        Ok(Self {
            num_leading_samples_known: byte & 0x80 != 0,
            num_leading_samples: byte & 0x7f,
        })
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.num_leading_samples > 0x7f {
            return Err(Error::invalid_value("num_leading_samples"));
        }
        let known = if self.num_leading_samples_known {
            0x80
        } else {
            0
        };
        writer.write_bytes(&[known | self.num_leading_samples])
    }
}

/// [`SapEntry`] (`sap `), ISO/IEC 14496-12 §10.6
///
/// The samples whose first byte is the position of a stream access point of
/// a type, as Annex I defines them. A `sap_type` other than 1 to 6, the
/// types Annex I defines, is refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `SAP_type`.
#[doc(alias = "sap", alias = "SAPEntry")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SapEntry {
    /// Whether the reference layers of the target layers may have to be
    /// decoded to access a sample, `false` for non-layered media
    pub dependent_flag: bool,
    /// Type of the stream access points, 1 to 6
    pub sap_type: u8,
}

impl SapEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [byte] = *reader.read_bytes::<1>()?;

        Ok(Self {
            dependent_flag: byte & 0x80 != 0,
            sap_type: byte & 0x0f,
        })
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if !(1..=6).contains(&self.sap_type) {
            return Err(Error::invalid_value("SAP_type"));
        }
        let dependent = if self.dependent_flag { 0x80 } else { 0 };
        writer.write_bytes(&[dependent | self.sap_type])
    }
}

/// [`VisualDrapEntry`] (`drap`), ISO/IEC 14496-12 §10.8
///
/// The dependent random access points: samples after which all samples
/// decode correctly if the closest preceding stream access point of type 1,
/// 2 or 3 is available. An entry whose `reserved` bits are not zero, which
/// readers ignore, is kept as its bytes. A `drap_type` other than 1 to 3 is
/// refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `DRAP_type`.
#[doc(alias = "drap", alias = "VisualDRAPEntry")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct VisualDrapEntry {
    /// Type of stream access point the sample would be if it did not depend
    /// on the closest preceding one, 1 to 3
    pub drap_type: u8,
}

impl VisualDrapEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let fields = reader.read_u32()?;
        if fields & 0x1fff_ffff != 0 {
            return Err(Error::unsupported_value("reserved"));
        }

        Ok(Self {
            drap_type: u8::try_from(fields >> 29).unwrap_or_default(),
        })
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if !(1..=3).contains(&self.drap_type) {
            return Err(Error::invalid_value("DRAP_type"));
        }
        writer.write_u32(u32::from(self.drap_type) << 29)
    }
}

/// [`VisualEdrapEntry`] (`edrp`), ISO/IEC 14496-12 §10.11
///
/// The extended dependent random access points: samples after which all
/// samples decode correctly if the stream access point or extended dependent
/// random access point samples they name are available. An entry whose
/// `reserved` bits are not zero, which readers ignore, is kept as its bytes.
///
/// `num_ref_sap_or_edrap_samples_minus1` is the number of
/// `ref_sap_or_edrap_idx_delta`s, less one. An `edrap_type` other than 1 to 3, and
/// a number of deltas other than 1 to 8, are refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field.
#[doc(alias = "edrp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct VisualEdrapEntry {
    /// Type of stream access point the sample would be if it did not depend
    /// on preceding samples, 1 to 3
    pub edrap_type: u8,
    /// `ref_sap_or_edrap_idx_delta` of each sample the sample needs for
    /// reference, by how many such samples back it lies
    pub ref_sap_or_edrap_idx_deltas: Vec<u16>,
}

impl VisualEdrapEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [byte] = *reader.read_bytes::<1>()?;
        if byte & 0x03 != 0 {
            return Err(Error::unsupported_value("reserved"));
        }
        let delta_count = u32::from(byte >> 2 & 0x07) + 1;

        Ok(Self {
            edrap_type: byte >> 5,
            ref_sap_or_edrap_idx_deltas: read_entries(
                reader,
                delta_count,
                u16::ENCODED_LEN,
                <u16 as TableEntry>::read,
            )?,
        })
    }

    pub(super) fn encoded_len(&self) -> u64 {
        entries_len(&self.ref_sap_or_edrap_idx_deltas).saturating_add(1)
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if !(1..=3).contains(&self.edrap_type) {
            return Err(Error::invalid_value("edrap_type"));
        }
        let minus1 = self
            .ref_sap_or_edrap_idx_deltas
            .len()
            .checked_sub(1)
            .and_then(|minus1| u8::try_from(minus1).ok())
            .filter(|minus1| *minus1 < 8)
            .ok_or_else(|| Error::invalid_value("num_ref_sap_or_edrap_samples_minus1"))?;
        writer.write_bytes(&[self.edrap_type << 5 | minus1 << 2])?;
        write_entries(writer, &self.ref_sap_or_edrap_idx_deltas)
    }
}
