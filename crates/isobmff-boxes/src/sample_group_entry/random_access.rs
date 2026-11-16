//! The entries of the sample groups that mark where decoding can start: [`RollRecoveryEntry`], [`AudioPreRollEntry`], [`AlternativeStartupEntry`], [`VisualRandomAccessEntry`], [`SapEntry`], [`VisualDrapEntry`] and [`VisualEdrapEntry`]

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter};

use super::entry::{SampleGroupDescriptionEntry, from_class, of_grouping_type, read_whole};

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

impl RollRecoveryEntry {
    /// How many bytes the entry takes
    pub(super) const ENCODED_LEN: u64 = 2;
}

impl TryFrom<&SampleGroupDescriptionEntry> for RollRecoveryEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"roll")?;
        read_whole(&entry.data, |reader| {
            Ok(Self {
                roll_distance: reader.read_i16()?,
            })
        })
    }
}

impl TryFrom<&RollRecoveryEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &RollRecoveryEntry) -> Result<Self, Error> {
        from_class(b"roll", RollRecoveryEntry::ENCODED_LEN, |writer| {
            if class.roll_distance == 0 {
                return Err(Error::invalid_value("roll_distance"));
            }
            writer.write_i16(class.roll_distance)
        })
    }
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

impl AudioPreRollEntry {
    /// How many bytes the entry takes
    pub(super) const ENCODED_LEN: u64 = 2;
}

impl TryFrom<&SampleGroupDescriptionEntry> for AudioPreRollEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"prol")?;
        read_whole(&entry.data, |reader| {
            Ok(Self {
                roll_distance: reader.read_i16()?,
            })
        })
    }
}

impl TryFrom<&AudioPreRollEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &AudioPreRollEntry) -> Result<Self, Error> {
        from_class(b"prol", AudioPreRollEntry::ENCODED_LEN, |writer| {
            if class.roll_distance <= 0 {
                return Err(Error::invalid_value("roll_distance"));
            }
            writer.write_i16(class.roll_distance)
        })
    }
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

impl TryFrom<&SampleGroupDescriptionEntry> for AlternativeStartupEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"alst")?;
        read_whole(&entry.data, |reader| {
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
        })
    }
}

impl TryFrom<&AlternativeStartupEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &AlternativeStartupEntry) -> Result<Self, Error> {
        from_class(
            b"alst",
            entries_len(&class.sample_offsets)
                .saturating_add(entries_len(&class.pieces))
                .saturating_add(2 + 2),
            |writer| {
                let roll_count = u16::try_from(class.sample_offsets.len())
                    .map_err(|_| Error::invalid_value("roll_count"))?;
                writer.write_u16(roll_count)?;
                writer.write_u16(class.first_output_sample)?;
                write_entries(writer, &class.sample_offsets)?;
                write_entries(writer, &class.pieces)
            },
        )
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
    /// How many bytes the entry takes
    pub(super) const ENCODED_LEN: u64 = 1;
}

impl TryFrom<&SampleGroupDescriptionEntry> for VisualRandomAccessEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"rap ")?;
        read_whole(&entry.data, |reader| {
            let [byte] = *reader.read_bytes::<1>()?;

            Ok(Self {
                num_leading_samples_known: byte & 0x80 != 0,
                num_leading_samples: byte & 0x7f,
            })
        })
    }
}

impl TryFrom<&VisualRandomAccessEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &VisualRandomAccessEntry) -> Result<Self, Error> {
        from_class(b"rap ", VisualRandomAccessEntry::ENCODED_LEN, |writer| {
            if class.num_leading_samples > 0x7f {
                return Err(Error::invalid_value("num_leading_samples"));
            }
            let known = if class.num_leading_samples_known {
                0x80
            } else {
                0
            };
            writer.write_bytes(&[known | class.num_leading_samples])
        })
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
    /// How many bytes the entry takes
    pub(super) const ENCODED_LEN: u64 = 1;
}

impl TryFrom<&SampleGroupDescriptionEntry> for SapEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"sap ")?;
        read_whole(&entry.data, |reader| {
            let [byte] = *reader.read_bytes::<1>()?;

            Ok(Self {
                dependent_flag: byte & 0x80 != 0,
                sap_type: byte & 0x0f,
            })
        })
    }
}

impl TryFrom<&SapEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &SapEntry) -> Result<Self, Error> {
        from_class(b"sap ", SapEntry::ENCODED_LEN, |writer| {
            if !(1..=6).contains(&class.sap_type) {
                return Err(Error::invalid_value("SAP_type"));
            }
            let dependent = if class.dependent_flag { 0x80 } else { 0 };
            writer.write_bytes(&[dependent | class.sap_type])
        })
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
    /// How many bytes the entry takes
    pub(super) const ENCODED_LEN: u64 = 4;
}

impl TryFrom<&SampleGroupDescriptionEntry> for VisualDrapEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"drap")?;
        read_whole(&entry.data, |reader| {
            let fields = reader.read_u32()?;
            if fields & 0x1fff_ffff != 0 {
                return Err(Error::unsupported_value("reserved"));
            }

            Ok(Self {
                drap_type: u8::try_from(fields >> 29).unwrap_or_default(),
            })
        })
    }
}

impl TryFrom<&VisualDrapEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &VisualDrapEntry) -> Result<Self, Error> {
        from_class(b"drap", VisualDrapEntry::ENCODED_LEN, |writer| {
            if !(1..=3).contains(&class.drap_type) {
                return Err(Error::invalid_value("DRAP_type"));
            }
            writer.write_u32(u32::from(class.drap_type) << 29)
        })
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

impl TryFrom<&SampleGroupDescriptionEntry> for VisualEdrapEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        of_grouping_type(entry, b"edrp")?;
        read_whole(&entry.data, |reader| {
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
        })
    }
}

impl TryFrom<&VisualEdrapEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(class: &VisualEdrapEntry) -> Result<Self, Error> {
        from_class(
            b"edrp",
            entries_len(&class.ref_sap_or_edrap_idx_deltas).saturating_add(1),
            |writer| {
                if !(1..=3).contains(&class.edrap_type) {
                    return Err(Error::invalid_value("edrap_type"));
                }
                let minus1 = class
                    .ref_sap_or_edrap_idx_deltas
                    .len()
                    .checked_sub(1)
                    .and_then(|minus1| u8::try_from(minus1).ok())
                    .filter(|minus1| *minus1 < 8)
                    .ok_or_else(|| Error::invalid_value("num_ref_sap_or_edrap_samples_minus1"))?;
                writer.write_bytes(&[class.edrap_type << 5 | minus1 << 2])?;
                write_entries(writer, &class.ref_sap_or_edrap_idx_deltas)
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::Error;

    use super::super::entry::{SampleGroupDescriptionEntry, assert_round_trips, entry_of};
    use super::{
        AlternativeStartupEntry, AlternativeStartupPiece, AudioPreRollEntry, RollRecoveryEntry,
        SapEntry, VisualDrapEntry, VisualEdrapEntry, VisualRandomAccessEntry,
    };

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[(
            b"roll",
            b"\xff\xfe".as_slice(),
            RollRecoveryEntry { roll_distance: -2 },
        )]);
        assert_round_trips(&[(
            b"prol",
            b"\0\x05".as_slice(),
            AudioPreRollEntry { roll_distance: 5 },
        )]);
        assert_round_trips(&[(
            b"alst",
            b"\0\x02\0\x01\0\0\0\0\0\0\x0b\xb8\0\x01\0\x02".as_slice(),
            AlternativeStartupEntry {
                first_output_sample: 1,
                sample_offsets: vec![0, 3000],
                pieces: vec![AlternativeStartupPiece {
                    num_output_samples: 1,
                    num_total_samples: 2,
                }],
            },
        )]);
        assert_round_trips(&[(
            b"rap ",
            b"\x83".as_slice(),
            VisualRandomAccessEntry {
                num_leading_samples_known: true,
                num_leading_samples: 3,
            },
        )]);
        assert_round_trips(&[(
            b"sap ",
            b"\x82".as_slice(),
            SapEntry {
                dependent_flag: true,
                sap_type: 2,
            },
        )]);
        assert_round_trips(&[(
            b"drap",
            b"\x40\0\0\0".as_slice(),
            VisualDrapEntry { drap_type: 2 },
        )]);
        assert_round_trips(&[(
            b"edrp",
            b"\x24\0\x01\0\x03".as_slice(),
            VisualEdrapEntry {
                edrap_type: 1,
                ref_sap_or_edrap_idx_deltas: vec![1, 3],
            },
        )]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [
                RollRecoveryEntry::try_from(&entry_of(b"seig", b"\xff\xfe")).map(|_| ()),
                AudioPreRollEntry::try_from(&entry_of(b"seig", b"\0\x05")).map(|_| ()),
                AlternativeStartupEntry::try_from(&entry_of(
                    b"seig",
                    b"\0\x02\0\x01\0\0\0\0\0\0\x0b\xb8\0\x01\0\x02"
                ))
                .map(|_| ()),
                VisualRandomAccessEntry::try_from(&entry_of(b"seig", b"\x83")).map(|_| ()),
                SapEntry::try_from(&entry_of(b"seig", b"\x82")).map(|_| ()),
                VisualDrapEntry::try_from(&entry_of(b"seig", b"\x40\0\0\0")).map(|_| ()),
                VisualEdrapEntry::try_from(&entry_of(b"seig", b"\x24\0\x01\0\x03")).map(|_| ()),
            ],
            [Err(Error::invalid_value("grouping_type")); 7]
        );
    }

    #[test]
    fn a_drap_or_edrp_entry_whose_reserved_bits_are_set_is_refused() {
        assert_eq!(
            [
                VisualDrapEntry::try_from(&entry_of(b"drap", b"\x40\0\0\x01")).map(|_| ()),
                VisualEdrapEntry::try_from(&entry_of(b"edrp", b"\x21\0\x01")).map(|_| ())
            ],
            [
                Err(Error::unsupported_value("reserved")),
                Err(Error::unsupported_value("reserved"))
            ]
        );
    }

    #[test]
    fn values_the_specification_does_not_allow_are_refused_when_written() {
        let refused = [
            SampleGroupDescriptionEntry::try_from(&RollRecoveryEntry { roll_distance: 0 }),
            SampleGroupDescriptionEntry::try_from(&AudioPreRollEntry { roll_distance: -1 }),
            SampleGroupDescriptionEntry::try_from(&VisualRandomAccessEntry {
                num_leading_samples_known: true,
                num_leading_samples: 0x80,
            }),
            SampleGroupDescriptionEntry::try_from(&SapEntry {
                dependent_flag: false,
                sap_type: 7,
            }),
            SampleGroupDescriptionEntry::try_from(&VisualDrapEntry { drap_type: 4 }),
            SampleGroupDescriptionEntry::try_from(&VisualEdrapEntry {
                edrap_type: 4,
                ref_sap_or_edrap_idx_deltas: vec![1],
            }),
            SampleGroupDescriptionEntry::try_from(&VisualEdrapEntry {
                edrap_type: 1,
                ref_sap_or_edrap_idx_deltas: Vec::new(),
            }),
        ];

        assert_eq!(
            refused,
            [
                Err(Error::invalid_value("roll_distance")),
                Err(Error::invalid_value("roll_distance")),
                Err(Error::invalid_value("num_leading_samples")),
                Err(Error::invalid_value("SAP_type")),
                Err(Error::invalid_value("DRAP_type")),
                Err(Error::invalid_value("edrap_type")),
                Err(Error::invalid_value("num_ref_sap_or_edrap_samples_minus1"))
            ]
        );
    }
}
