//! [`GroupsListBox`], the entity groups of a meta box

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, FieldReader, FieldWriter,
    FourCc, FullBoxFlags, FullBoxHeader,
};

use crate::box_type_argument;
use crate::free::{Free, FreeType, Skip};
use crate::full_box::read_version_zero;
use crate::prsl::PreselectionGroupBox;
use crate::table::{entries_len, read_entries, write_entries, write_entry_count};

/// [`EntityToGroupBox`], ISO/IEC 14496-12 §8.15.3
///
/// An entity group: items, and tracks or track groups at the file level,
/// that share the characteristic or relationship its `grouping_type` names.
/// The type is the box type of the box, which [`GroupsListBox`] reads into
/// [`grouping_type`](Self::grouping_type) and writes from it, so a box of any
/// type, registered or not, reads. [`PreselectionGroupBox`], of the type
/// `prsl`, is a type of its own.
///
/// The box reads version 0 alone, the version ISO/IEC 14496-12 states unless
/// the definition of a `grouping_type` states another.
///
/// The box has no box type of its own, so it implements neither
/// [`BoxDecode`] nor [`BoxDefinition`]: its [`BoxEncode`] writes its payload,
/// and a [`GroupsListBox`] writes the header.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EntityToGroupBox {
    /// Type of the grouping, such as `altr`
    ///
    /// It is written as the box type, so a `grouping_type` of `uuid` is
    /// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue)
    /// for `grouping_type` when the box is written. A box written under `prsl`
    /// reads back as a [`PreselectionGroupBox`], and one under `free` or
    /// `skip` as the bytes of free space.
    pub grouping_type: FourCc,
    /// Flags of the box, kept as read
    ///
    /// Bits 0 to 11 are reserved, and bits 12 to 23 are zero unless the
    /// definition of the `grouping_type` gives them a meaning.
    pub flags: FullBoxFlags,
    /// Identifier of the entity group, which no other entity group, item or, at the file level, track shares
    pub group_id: u32,
    /// `item_ID`, `track_ID`, `track_group_id` or `group_id` of each entity of the group
    pub entity_ids: Vec<u32>,
    /// Bytes after the entities, which the definition of the `grouping_type` lays out
    pub remaining_data: Vec<u8>,
}

impl Default for EntityToGroupBox {
    fn default() -> Self {
        Self {
            grouping_type: FourCc::new([0; 4]),
            flags: FullBoxFlags::ZERO,
            group_id: 0,
            entity_ids: Vec::new(),
            remaining_data: Vec::new(),
        }
    }
}

/// Reads `group_id` and the entities of an entity group, each with the count before it
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside a field, or holds fewer `entity_id`s than
///   `num_entities_in_group`.
pub(crate) fn read_entities(reader: &mut FieldReader<'_>) -> Result<(u32, Vec<u32>), Error> {
    let group_id = reader.read_u32()?;
    let num_entities_in_group = reader.read_u32()?;
    let entity_ids = read_entries(reader, num_entities_in_group, 4, |reader| reader.read_u32())?;

    Ok((group_id, entity_ids))
}

/// Returns the bytes `group_id`, `num_entities_in_group` and the `entity_id`s take
pub(crate) fn entities_len(entity_ids: &[u32]) -> u64 {
    entries_len(entity_ids).saturating_add(4 + 4)
}

/// Writes `group_id`, `num_entities_in_group` and the `entity_id`s
///
/// # Errors
///
/// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange): there are more
///   entities than 32 bits count.
pub(crate) fn write_entities(
    writer: &mut FieldWriter<'_>,
    group_id: u32,
    entity_ids: &[u32],
) -> Result<(), Error> {
    writer.write_u32(group_id)?;
    write_entry_count(writer, entity_ids.len())?;
    write_entries(writer, entity_ids)
}

impl EntityToGroupBox {
    /// Reads the payload of a box of the box type `grouping_type`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a field, or holds fewer `entity_id`s than
    ///   `num_entities_in_group`.
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion):
    ///   the version is not 0.
    fn decode_payload(grouping_type: FourCc, payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let flags = read_version_zero(&mut reader)?;
        let (group_id, entity_ids) = read_entities(&mut reader)?;

        Ok(Self {
            grouping_type,
            flags,
            group_id,
            entity_ids,
            remaining_data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for EntityToGroupBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.remaining_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(entities_len(&self.entity_ids))
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(0, self.flags).to_bytes())?;
        write_entities(writer, self.group_id, &self.entity_ids)?;
        writer.write_slice(&self.remaining_data)
    }
}

/// One child of a [`GroupsListBox`]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum EntityToGroup {
    /// Entity group of the `grouping_type` the box type gives
    Typed(EntityToGroupBox),
    /// Preselection, the entity group of the type `prsl`
    Preselection(PreselectionGroupBox),
    /// Free space, child under a `uuid` box type, or one that does not decode, kept as its bytes
    Other(AnyBox),
}

/// [`GroupsListBox`] (`grpl`), ISO/IEC 14496-12 §8.15.2
///
/// The entity groups of a `MetaBox`, one [`EntityToGroupBox`] each.
///
/// Every child under a compact box type is read as an [`EntityToGroupBox`] of
/// that type, but for `prsl`, which is read as a [`PreselectionGroupBox`]. A
/// `free` or `skip` child, which a box may hold anywhere, a child under a
/// `uuid` box type, and one that does not decode, such as one of a version it
/// does not read, is kept as its bytes. The children are kept,
/// and written back, in the order they came.
///
/// # Untrusted input
///
/// The box reads each child as it frames it, and allocates room for a child
/// only once it has read it, so what it allocates is bounded by the length of
/// the payload, which the caller sets.
#[doc(alias = "grpl")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct GroupsListBox {
    /// Entity groups, in the order they came
    pub entity_groups: Vec<EntityToGroup>,
}

impl BoxDefinition for GroupsListBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"grpl");
}

impl BoxDecode for GroupsListBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut entity_groups = Vec::new();
        for child in Boxes::new(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            let payload = child.payload();
            let read = match box_type {
                PreselectionGroupBox::BOX_TYPE => PreselectionGroupBox::decode_payload(payload)
                    .map(EntityToGroup::Preselection)
                    .ok(),
                Free::FREE_TYPE | Skip::FREE_TYPE | BoxType::Extended(_) => None,
                BoxType::Compact(grouping_type) => {
                    EntityToGroupBox::decode_payload(grouping_type.four_cc(), payload)
                        .map(EntityToGroup::Typed)
                        .ok()
                }
            };
            entity_groups.push(read.unwrap_or_else(|| {
                EntityToGroup::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self { entity_groups })
    }
}

impl BoxEncode for GroupsListBox {
    fn payload_len(&self) -> u64 {
        self.entity_groups
            .iter()
            .map(|child| match child {
                EntityToGroup::Typed(typed) => box_type_argument::encoded_len(typed),
                EntityToGroup::Preselection(preselection) => preselection.encoded_len(),
                EntityToGroup::Other(other) => other.encoded_len(),
            })
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for child in &self.entity_groups {
            rest = match child {
                EntityToGroup::Typed(typed) => {
                    box_type_argument::encode(typed.grouping_type, "grouping_type", typed, rest)?
                }
                EntityToGroup::Preselection(preselection) => preselection.encode(rest)?,
                EntityToGroup::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error, FourCc, FullBoxFlags};

    use super::{EntityToGroup, EntityToGroupBox, GroupsListBox};
    use crate::encoded;
    use crate::prsl::PreselectionGroupBox;

    /// `altr` group 10 of the entities 1 and 2
    const ALTR: &[u8] = b"\0\0\0\x1caltr\0\0\0\0\0\0\0\x0a\0\0\0\x02\0\0\0\x01\0\0\0\x02";

    /// Group of a type ISO/IEC 14496-12 does not register, with flags 0x001000, of entity 3 and two bytes after it
    const XGRP: &[u8] = b"\0\0\0\x1axgrp\0\0\x10\0\0\0\0\x0b\0\0\0\x01\0\0\0\x03\xab\xcd";

    /// `prsl` group 12 of entity 1, with no optional field
    const PRSL: &[u8] = b"\0\0\0\x18prsl\0\0\0\0\0\0\0\x0c\0\0\0\x01\0\0\0\x01";

    /// `eqiv` group of a version it does not read
    const EQIV_V1: &[u8] = b"\0\0\0\x18eqiv\x01\0\0\0\0\0\0\x0d\0\0\0\0\0\0\0\0";

    /// `free` of twelve zero bytes, which would read as an entity group of no entity
    const FREE: &[u8] = b"\0\0\0\x14free\0\0\0\0\0\0\0\0\0\0\0\0";

    fn groups() -> GroupsListBox {
        GroupsListBox {
            entity_groups: vec![
                EntityToGroup::Typed(EntityToGroupBox {
                    grouping_type: FourCc::new(*b"altr"),
                    group_id: 10,
                    entity_ids: vec![1, 2],
                    ..EntityToGroupBox::default()
                }),
                EntityToGroup::Typed(EntityToGroupBox {
                    grouping_type: FourCc::new(*b"xgrp"),
                    flags: FullBoxFlags::new(0x1000).unwrap(),
                    group_id: 11,
                    entity_ids: vec![3],
                    remaining_data: vec![0xab, 0xcd],
                }),
                EntityToGroup::Preselection(PreselectionGroupBox {
                    group_id: 12,
                    entity_ids: vec![1],
                    ..PreselectionGroupBox::default()
                }),
                EntityToGroup::Other(AnyBox::from_raw_bytes(
                    BoxType::compact(*b"eqiv"),
                    b"\x01\0\0\0\0\0\0\x0d\0\0\0\0\0\0\0\0".to_vec(),
                )),
                EntityToGroup::Other(AnyBox::from_raw_bytes(
                    BoxType::compact(*b"free"),
                    vec![0; 12],
                )),
            ],
        }
    }

    #[test]
    fn reads_every_child_as_the_entity_group_its_type_names_and_keeps_free_space_in_the_order_they_came()
     {
        let bytes = [
            b"\0\0\0\x82grpl".as_slice(),
            ALTR,
            XGRP,
            PRSL,
            EQIV_V1,
            FREE,
        ]
        .concat();

        assert_eq!(
            GroupsListBox::decode(&bytes),
            Ok((groups(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_each_child_under_its_type_in_the_order_it_holds_them() {
        assert_eq!(
            encoded(&groups()),
            [
                b"\0\0\0\x82grpl".as_slice(),
                ALTR,
                XGRP,
                PRSL,
                EQIV_V1,
                FREE
            ]
            .concat()
        );
    }

    #[test]
    fn a_group_ending_inside_its_entities_is_kept_as_its_bytes() {
        let expected = GroupsListBox {
            entity_groups: vec![EntityToGroup::Other(AnyBox::from_raw_bytes(
                BoxType::compact(*b"altr"),
                b"\0\0\0\0\0\0\0\x0a\0\0\0\x02\0\0\0\x01".to_vec(),
            ))],
        };

        assert_eq!(
            GroupsListBox::decode_payload(b"\0\0\0\x18altr\0\0\0\0\0\0\0\x0a\0\0\0\x02\0\0\0\x01"),
            Ok(expected)
        );
    }

    #[test]
    fn an_entity_group_of_the_type_uuid_is_refused_when_written() {
        let groups = GroupsListBox {
            entity_groups: vec![EntityToGroup::Typed(EntityToGroupBox {
                grouping_type: FourCc::new(*b"uuid"),
                ..EntityToGroupBox::default()
            })],
        };

        assert_eq!(
            groups.encode(&mut [0; 32]),
            Err(Error::invalid_value("grouping_type"))
        );
    }
}
