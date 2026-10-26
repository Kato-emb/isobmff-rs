//! [`FdSessionGroupBox`], the FD sessions of a file and the file groups and FD hint tracks of each

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::table::{TableEntry, entries_len, read_entries, write_entries};

/// Session group of an [`FdSessionGroupBox`]: the file groups it complies with and the FD hint tracks it sends over
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SessionGroup {
    /// File groups the session group complies with
    ///
    /// More than 255 are refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `entry_count` when the box is written.
    pub group_ids: Vec<u32>,
    /// Track identifiers of the FD hint tracks of the session group, one per LCT channel, the first of which is the base channel
    ///
    /// None, or more than 65535, are refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `num_channels_in_session_group` when the box is written: the count
    /// is positive.
    pub hint_track_ids: Vec<u32>,
}

/// Bytes a session group takes at least: its `entry_count` and `num_channels_in_session_group`
const SESSION_GROUP_MINIMUM_LEN: u64 = 1 + 2;

/// [`FdSessionGroupBox`] (`segr`), ISO/IEC 14496-12 §8.12.5
///
/// The FD sessions of a file, each sending over all the FD hint tracks its
/// session group lists. The first session group is the default choice.
///
/// More than 65535 session groups are refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `num_session_groups` when the box is written.
///
/// # Untrusted input
///
/// The box holds each count against the payload before it reads what the
/// count counts, so what it allocates is bounded by the length of the
/// payload, which the caller sets.
#[doc(alias = "segr", alias = "FDSessionGroupBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct FdSessionGroupBox {
    /// Session groups, in the order they came
    pub session_groups: Vec<SessionGroup>,
}

impl BoxDefinition for FdSessionGroupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"segr");
}

impl BoxDecode for FdSessionGroupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let num_session_groups = reader.read_u16()?;
        let session_groups = read_entries(
            reader,
            u32::from(num_session_groups),
            SESSION_GROUP_MINIMUM_LEN,
            |reader| {
                let [entry_count] = *reader.read_bytes::<1>()?;
                let group_ids =
                    read_entries(reader, u32::from(entry_count), u32::ENCODED_LEN, u32::read)?;
                let num_channels_in_session_group = reader.read_u16()?;
                let hint_track_ids = read_entries(
                    reader,
                    u32::from(num_channels_in_session_group),
                    u32::ENCODED_LEN,
                    u32::read,
                )?;

                Ok(SessionGroup {
                    group_ids,
                    hint_track_ids,
                })
            },
        )?;

        Ok(Self { session_groups })
    }
}

impl BoxEncode for FdSessionGroupBox {
    fn payload_len(&self) -> u64 {
        self.session_groups
            .iter()
            .map(|group| {
                entries_len(&group.group_ids)
                    .saturating_add(entries_len(&group.hint_track_ids))
                    .saturating_add(SESSION_GROUP_MINIMUM_LEN)
            })
            .fold(2, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u16(
            u16::try_from(self.session_groups.len())
                .map_err(|_| Error::invalid_value("num_session_groups"))?,
        )?;
        for group in &self.session_groups {
            writer.write_bytes(&[u8::try_from(group.group_ids.len())
                .map_err(|_| Error::invalid_value("entry_count"))?])?;
            write_entries(writer, &group.group_ids)?;
            writer.write_u16(
                u16::try_from(group.hint_track_ids.len())
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or_else(|| Error::invalid_value("num_channels_in_session_group"))?,
            )?;
            write_entries(writer, &group.hint_track_ids)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{FdSessionGroupBox, SessionGroup};
    use crate::encoded;

    /// `segr` of two session groups: file groups 1 and 2 over the hint track 3, then no file group over the hint tracks 4 and 5
    const SEGR: &[u8] =
        b"\0\0\0\x24segr\0\x02\x02\0\0\0\x01\0\0\0\x02\0\x01\0\0\0\x03\0\0\x02\0\0\0\x04\0\0\0\x05";

    fn session_groups() -> FdSessionGroupBox {
        FdSessionGroupBox {
            session_groups: vec![
                SessionGroup {
                    group_ids: vec![1, 2],
                    hint_track_ids: vec![3],
                },
                SessionGroup {
                    group_ids: vec![],
                    hint_track_ids: vec![4, 5],
                },
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            FdSessionGroupBox::decode(SEGR),
            Ok((session_groups(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&session_groups()), SEGR);
    }

    #[test]
    fn a_session_group_of_no_hint_track_is_refused_when_written() {
        let session_groups = FdSessionGroupBox {
            session_groups: vec![SessionGroup::default()],
        };

        assert_eq!(
            session_groups.encode(&mut [0; 13]).map(|_| ()),
            Err(Error::invalid_value("num_channels_in_session_group"))
        );
    }

    #[test]
    fn a_count_of_more_group_ids_than_the_payload_holds_is_refused() {
        assert_eq!(
            FdSessionGroupBox::decode_payload(b"\0\x01\x02\0\0\0\x01\0\0"),
            Err(Error::truncated_payload(11, 9))
        );
    }

    #[test]
    fn a_count_of_more_session_groups_than_the_payload_holds_is_refused() {
        assert_eq!(
            FdSessionGroupBox::decode_payload(b"\0\x02\0\0\x01"),
            Err(Error::truncated_payload(8, 5))
        );
    }
}
