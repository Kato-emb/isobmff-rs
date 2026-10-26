//! [`SampleGroupDescriptionBox`]

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    FullBoxFlags, FullBoxHeader,
};

use crate::sample_group_entry::SampleGroupDescriptionEntry;
use crate::table::{read_entries, write_entry_count};

/// Flag mask of `static_group_description`
const STATIC_GROUP_DESCRIPTION: u32 = 0x1;

/// Flag mask of `static_mapping`
const STATIC_MAPPING: u32 = 0x2;

/// [`SampleGroupDescriptionBox`] (`sgpd`), ISO/IEC 14496-12 §8.9.3
///
/// The sample group description entries of one sample grouping, each of
/// which describes the samples of one sample group. A sample group
/// description of version 3 is essential: a reader shall not process a track
/// that holds one whose `grouping_type` it does not recognize.
///
/// The box is written in the `version` it holds, 0 to 3. Version 1 and up
/// state the length of each entry: `default_length` where every entry takes
/// as many bytes, and otherwise a `description_length` before each entry,
/// derived from the entry. Version 2 and up state a
/// `default_group_description_index`; a box of a lower version describes no
/// default, as if the field were 0.
///
/// An entry of version 0 states no length, so the box reads one only of a
/// `grouping_type` whose syntax delimits it: an entry of another
/// `grouping_type`, such as `alst` or `it35` whose fields run to the end of
/// the entry, is refused with
/// [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
/// `grouping_type`, read or written. An entry of version 1 and up that is of
/// a `grouping_type` the crate does not read, or that does not decode as its
/// `grouping_type` lays it out or fill the length stated for it, is kept as
/// its bytes.
///
/// A box is refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field,
/// where it holds an entry of another `grouping_type` than its own, a
/// `default_length` other than 0 that is not the length of every entry, or a
/// field its version does not carry that is not 0.
///
/// [`Default`] is the box of version 1, of no flag, `grouping_type` four zero
/// bytes and no entry.
#[doc(alias = "sgpd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleGroupDescriptionBox {
    /// Version of the box: 0 to 3, version 3 for an essential sample group description
    pub version: u8,
    /// Whether no track fragment of the track holds a box of this `grouping_type`
    pub static_group_description: bool,
    /// Whether no `SampleToGroupBox` of the track maps samples of this
    /// `grouping_type`, every sample mapping to the default
    pub static_mapping: bool,
    /// Criterion the samples are grouped by, which links the box to the
    /// `SampleToGroupBox` of the same `grouping_type` and decides the syntax
    /// of the entries
    pub grouping_type: FourCc,
    /// Number of bytes every entry takes, or 0 where each entry states its
    /// own; carried by version 1 and up
    pub default_length: u32,
    /// Index, from 1, of the entry that describes the samples no
    /// `SampleToGroupBox` maps, or 0 for none; carried by version 2 and up
    pub default_group_description_index: u32,
    /// The sample group description entries, in index order from 1
    pub entries: Vec<SampleGroupDescriptionEntry>,
}

impl Default for SampleGroupDescriptionBox {
    fn default() -> Self {
        Self {
            version: 1,
            static_group_description: false,
            static_mapping: false,
            grouping_type: FourCc::new([0; 4]),
            default_length: 0,
            default_group_description_index: 0,
            entries: Vec::new(),
        }
    }
}

impl SampleGroupDescriptionBox {
    /// Returns whether each entry states its own length before it
    fn states_description_length(&self) -> bool {
        self.version >= 1 && self.default_length == 0
    }
}

impl BoxDefinition for SampleGroupDescriptionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"sgpd");
}

impl BoxDecode for SampleGroupDescriptionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let version = header.version();
        if version > 3 {
            return Err(Error::unsupported_version(version));
        }
        let flags = header.flags().bits();
        let grouping_type = FourCc::new(*reader.read_bytes::<4>()?);
        let default_length = if version >= 1 { reader.read_u32()? } else { 0 };
        let default_group_description_index = if version >= 2 { reader.read_u32()? } else { 0 };
        let entry_count = reader.read_u32()?;
        let entries = if version == 0 {
            if entry_count != 0 && SampleGroupDescriptionEntry::runs_to_end(grouping_type) {
                return Err(Error::unsupported_value("grouping_type"));
            }
            read_entries(reader, entry_count, 1, |reader| {
                SampleGroupDescriptionEntry::read(grouping_type, reader)
                    .unwrap_or_else(|| Err(Error::unsupported_value("grouping_type")))
            })?
        } else {
            let minimum_len = if default_length == 0 {
                4
            } else {
                u64::from(default_length)
            };
            read_entries(reader, entry_count, minimum_len, |reader| {
                let description_length = if default_length == 0 {
                    reader.read_u32()?
                } else {
                    default_length
                };
                let bytes =
                    reader.read_slice(usize::try_from(description_length).unwrap_or(usize::MAX))?;
                let mut entry_reader = FieldReader::new(bytes);
                Ok(
                    match SampleGroupDescriptionEntry::read(grouping_type, &mut entry_reader) {
                        Some(Ok(entry)) if entry_reader.remainder().is_empty() => entry,
                        _ => SampleGroupDescriptionEntry::Other(bytes.to_vec()),
                    },
                )
            })?
        };

        Ok(Self {
            version,
            static_group_description: flags & STATIC_GROUP_DESCRIPTION != 0,
            static_mapping: flags & STATIC_MAPPING != 0,
            grouping_type,
            default_length,
            default_group_description_index,
            entries,
        })
    }
}

impl BoxEncode for SampleGroupDescriptionBox {
    fn payload_len(&self) -> u64 {
        let length_len = if self.states_description_length() {
            4
        } else {
            0
        };
        let fields_len = match self.version {
            0 => 4 + 4 + 4,
            1 => 4 + 4 + 4 + 4,
            _ => 4 + 4 + 4 + 4 + 4,
        };

        self.entries
            .iter()
            .map(|entry| entry.encoded_len().saturating_add(length_len))
            .fold(fields_len, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 3 {
            return Err(Error::unsupported_version(self.version));
        }
        if self.version == 0 && self.default_length != 0 {
            return Err(Error::invalid_value("default_length"));
        }
        if self.version < 2 && self.default_group_description_index != 0 {
            return Err(Error::invalid_value("default_group_description_index"));
        }
        if self.version == 0
            && !self.entries.is_empty()
            && SampleGroupDescriptionEntry::runs_to_end(self.grouping_type)
        {
            return Err(Error::unsupported_value("grouping_type"));
        }
        for entry in &self.entries {
            match entry.grouping_type() {
                Some(grouping_type) if grouping_type != self.grouping_type => {
                    return Err(Error::invalid_value("grouping_type"));
                }
                None if self.version == 0 => {
                    return Err(Error::unsupported_value("grouping_type"));
                }
                _ => {}
            }
            if self.default_length != 0 && entry.encoded_len() != u64::from(self.default_length) {
                return Err(Error::invalid_value("default_length"));
            }
        }

        let flags = [
            (self.static_group_description, STATIC_GROUP_DESCRIPTION),
            (self.static_mapping, STATIC_MAPPING),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |flags, (_, mask)| flags | mask);
        let flags = FullBoxFlags::new(flags).unwrap_or(FullBoxFlags::ZERO);
        writer.write_bytes(&FullBoxHeader::new(self.version, flags).to_bytes())?;
        writer.write_bytes(self.grouping_type.as_bytes())?;
        if self.version >= 1 {
            writer.write_u32(self.default_length)?;
        }
        if self.version >= 2 {
            writer.write_u32(self.default_group_description_index)?;
        }
        write_entry_count(writer, self.entries.len())?;
        for entry in &self.entries {
            if self.states_description_length() {
                let description_length = u32::try_from(entry.encoded_len())
                    .map_err(|_| Error::invalid_value("description_length"))?;
                writer.write_u32(description_length)?;
            }
            entry.write(writer)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FourCc};

    use super::SampleGroupDescriptionBox;
    use crate::encoded;
    use crate::sample_group_entry::{
        RollRecoveryEntry, SampleGroupDescriptionEntry, TemporalLevelEntry, VisualRandomAccessEntry,
    };

    /// `sgpd` of version 0 and `grouping_type` `roll`, of distances -1 and 2
    const SGPD_0: &[u8] = b"\0\0\0\x18sgpd\0\0\0\0roll\0\0\0\x02\xff\xff\0\x02";

    /// `sgpd` of version 1 and `grouping_type` `tele`, of `default_length` 1:
    /// one level independently decodable, one not
    const SGPD_1: &[u8] = b"\0\0\0\x1asgpd\x01\0\0\0tele\0\0\0\x01\0\0\0\x02\x80\0";

    /// `sgpd` of version 2, `static_group_description` and `static_mapping`,
    /// of `grouping_type` `seig`, which the crate does not read: entries of 3
    /// and 1 bytes, the second the default
    const SGPD_2: &[u8] = b"\0\0\0\x28sgpd\x02\0\0\x03seig\0\0\0\0\0\0\0\x02\0\0\0\x02\
        \0\0\0\x03\x01\x02\x03\0\0\0\x01\x04";

    /// `sgpd` of version 3 and `grouping_type` `rap `, whose first entry fills
    /// its length and whose second does not: it states 2 bytes for an entry
    /// of 1
    const SGPD_3: &[u8] = b"\0\0\0\x27sgpd\x03\0\0\0rap \0\0\0\0\0\0\0\0\0\0\0\x02\
        \0\0\0\x01\x81\0\0\0\x02\x81\x00";

    fn roll() -> SampleGroupDescriptionBox {
        SampleGroupDescriptionBox {
            version: 0,
            grouping_type: FourCc::new(*b"roll"),
            entries: vec![
                SampleGroupDescriptionEntry::RollRecovery(RollRecoveryEntry { roll_distance: -1 }),
                SampleGroupDescriptionEntry::RollRecovery(RollRecoveryEntry { roll_distance: 2 }),
            ],
            ..SampleGroupDescriptionBox::default()
        }
    }

    fn temporal_level() -> SampleGroupDescriptionBox {
        SampleGroupDescriptionBox {
            grouping_type: FourCc::new(*b"tele"),
            default_length: 1,
            entries: vec![
                SampleGroupDescriptionEntry::TemporalLevel(TemporalLevelEntry {
                    level_independently_decodable: true,
                }),
                SampleGroupDescriptionEntry::TemporalLevel(TemporalLevelEntry {
                    level_independently_decodable: false,
                }),
            ],
            ..SampleGroupDescriptionBox::default()
        }
    }

    fn unread() -> SampleGroupDescriptionBox {
        SampleGroupDescriptionBox {
            version: 2,
            static_group_description: true,
            static_mapping: true,
            grouping_type: FourCc::new(*b"seig"),
            default_group_description_index: 2,
            entries: vec![
                SampleGroupDescriptionEntry::Other(vec![1, 2, 3]),
                SampleGroupDescriptionEntry::Other(vec![4]),
            ],
            ..SampleGroupDescriptionBox::default()
        }
    }

    fn essential() -> SampleGroupDescriptionBox {
        SampleGroupDescriptionBox {
            version: 3,
            grouping_type: FourCc::new(*b"rap "),
            entries: vec![
                SampleGroupDescriptionEntry::VisualRandomAccess(VisualRandomAccessEntry {
                    num_leading_samples_known: true,
                    num_leading_samples: 1,
                }),
                SampleGroupDescriptionEntry::Other(vec![0x81, 0]),
            ],
            ..SampleGroupDescriptionBox::default()
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_every_version() {
        assert_eq!(
            [SGPD_0, SGPD_1, SGPD_2, SGPD_3].map(SampleGroupDescriptionBox::decode),
            [
                Ok((roll(), b"".as_slice())),
                Ok((temporal_level(), b"".as_slice())),
                Ok((unread(), b"".as_slice())),
                Ok((essential(), b"".as_slice()))
            ]
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_every_version() {
        assert_eq!(
            [roll(), temporal_level(), unread(), essential()].map(|value| encoded(&value)),
            [SGPD_0, SGPD_1, SGPD_2, SGPD_3].map(<[u8]>::to_vec)
        );
    }

    #[test]
    fn entries_of_version_0_whose_grouping_type_does_not_delimit_them_are_refused() {
        assert_eq!(
            [
                SampleGroupDescriptionBox::decode_payload(b"\0\0\0\0seig\0\0\0\x01\0"),
                SampleGroupDescriptionBox::decode_payload(b"\0\0\0\0it35\0\0\0\x01\x80\xb5")
            ],
            [
                Err(Error::unsupported_value("grouping_type")),
                Err(Error::unsupported_value("grouping_type"))
            ]
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            SampleGroupDescriptionBox::decode_payload(b"\x01\0\0\0tele\0\0\0\x01\0\0\0\x03\x80\0"),
            Err(Error::truncated_payload(19, 18))
        );
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_refused() {
        assert_eq!(
            SampleGroupDescriptionBox::decode_payload(
                b"\x01\0\0\0roll\0\0\0\0\0\0\0\x01\0\0\0\x02\xff"
            ),
            Err(Error::truncated_payload(22, 21))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SampleGroupDescriptionBox::decode_payload(b"\x04\0\0\0roll\0\0\0\0"),
            Err(Error::unsupported_version(4))
        );
    }

    #[test]
    fn a_box_of_entries_it_cannot_lay_out_is_refused_when_written() {
        let mut other_grouping_type = temporal_level();
        other_grouping_type.grouping_type = FourCc::new(*b"roll");
        let mut wrong_default_length = temporal_level();
        wrong_default_length.default_length = 2;
        let mut bytes_in_version_0 = unread();
        bytes_in_version_0.version = 0;
        bytes_in_version_0.default_group_description_index = 0;

        assert_eq!(
            [
                other_grouping_type,
                wrong_default_length,
                bytes_in_version_0
            ]
            .map(|value| value.encode(&mut [0; 64]).map(|_| ())),
            [
                Err(Error::invalid_value("grouping_type")),
                Err(Error::invalid_value("default_length")),
                Err(Error::unsupported_value("grouping_type"))
            ]
        );
    }
}
