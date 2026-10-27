//! [`FdItemInformationBox`] and its [`PartitionEntry`]s, the partitioning of source files and the FD sessions they are sent in

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, OtherBoxes,
};

use crate::fpar::FilePartitionBox;
use crate::full_box::{read_version_zero, write_version_zero};
use crate::gitn::GroupIdToNameBox;
use crate::reservoir::{FecReservoirBox, FileReservoirBox};
use crate::segr::FdSessionGroupBox;

/// [`PartitionEntry`] (`paen`), ISO/IEC 14496-12 §8.12.2
///
/// One partitioning of a source file: how it is split into source blocks and
/// symbols and FEC encoded, and the items that hold its FEC and File
/// reservoirs.
///
/// The box writes `fpar`, `fecr` and `fire` in this order, whatever order they
/// were read in: the order of its Syntax. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them. An `fecr`
/// or `fire` that does not read is kept there too, and reads back [`None`].
#[doc(alias = "paen")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct PartitionEntry {
    /// Source file and its partitioning into source blocks and symbols, the `blocks_and_symbols` of the Syntax
    pub fpar: FilePartitionBox,
    /// Items that hold the FEC reservoirs of the source blocks, the `FEC_symbol_locations` of the Syntax
    pub fecr: Option<FecReservoirBox>,
    /// Items that hold the File reservoirs of the source blocks, the `File_symbol_locations` of the Syntax
    pub fire: Option<FileReservoirBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for PartitionEntry {
    const BOX_TYPE: BoxType = BoxType::compact(*b"paen");
}

impl BoxDecode for PartitionEntry {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            fpar: children.take_exactly_one()?,
            fecr: children.take_zero_or_one_if_decoded()?,
            fire: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for PartitionEntry {
    fn payload_len(&self) -> u64 {
        [
            self.fpar.encoded_len(),
            self.fecr.as_ref().map_or(0, BoxEncode::encoded_len),
            self.fire.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.fpar.encode(writer.take_remainder())?;
        if let Some(fecr) = &self.fecr {
            rest = fecr.encode(rest)?;
        }
        if let Some(fire) = &self.fire {
            rest = fire.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

/// [`FdItemInformationBox`] (`fiin`), ISO/IEC 14496-12 §8.12.2
///
/// How the source files a `MetaBox` holds as items are partitioned for file
/// delivery over ALC/LCT or FLUTE, and how the FD hint tracks are combined
/// into FD sessions. The partition entries are numbered from 1 in their order.
///
/// A box whose `entry_count` differs from the number of `paen` it holds is
/// refused with [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch),
/// and so is a box holding a `paen` that does not read.
///
/// The box writes the `paen`s, `segr` and `gitn` in this order, whatever order
/// they were read in: the order of its Syntax. Children of other types are
/// kept in [`other_boxes`](Self::other_boxes), and written back after them.
/// A `segr` or `gitn` that does not read is kept there too, and reads back
/// [`None`]. More than 65535 partition entries are refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `entry_count`
/// when the box is written.
#[doc(alias = "fiin", alias = "FDItemInformationBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct FdItemInformationBox {
    /// Partitionings of the source files, in the order they came, the `partition_entries` of the Syntax
    pub paen: Vec<PartitionEntry>,
    /// FD sessions and the file groups and FD hint tracks of each, the `session_info` of the Syntax
    pub segr: Option<FdSessionGroupBox>,
    /// Names of the file groups, the `group_id_to_name` of the Syntax
    pub gitn: Option<GroupIdToNameBox>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for FdItemInformationBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"fiin");
}

impl BoxDecode for FdItemInformationBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let entry_count = reader.read_u16()?;
        let mut children = ChildBoxes::decode(reader.take_remainder())?;
        let paen: Vec<PartitionEntry> = children.take_zero_or_more()?;
        let read = u64::try_from(paen.len()).unwrap_or(u64::MAX);
        if u64::from(entry_count) != read {
            return Err(Error::entry_count_mismatch(u64::from(entry_count), read));
        }

        Ok(Self {
            paen,
            segr: children.take_zero_or_one_if_decoded()?,
            gitn: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for FdItemInformationBox {
    fn payload_len(&self) -> u64 {
        [
            4 + 2,
            self.segr.as_ref().map_or(0, BoxEncode::encoded_len),
            self.gitn.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.paen.iter().map(BoxEncode::encoded_len))
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        writer.write_u16(
            u16::try_from(self.paen.len()).map_err(|_| Error::invalid_value("entry_count"))?,
        )?;
        let mut rest = writer.take_remainder();
        for paen in &self.paen {
            rest = paen.encode(rest)?;
        }
        if let Some(segr) = &self.segr {
            rest = segr.encode(rest)?;
        }
        if let Some(gitn) = &self.gitn {
            rest = gitn.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, ChildBoxes, Error, OtherBoxes};

    use super::{FdItemInformationBox, PartitionEntry};
    use crate::encoded;
    use crate::fpar::FilePartitionBox;
    use crate::gitn::GroupIdToNameBox;
    use crate::reservoir::{FecReservoirBox, FileReservoirBox};
    use crate::segr::{FdSessionGroupBox, SessionGroup};

    /// `fpar` of version 0 for item 1, every other field zero, empty scheme-specific info and no entry
    const FPAR: &[u8] = b"\0\0\0\x1dfpar\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

    /// `fecr` of version 0 and no entry
    const FECR: &[u8] = b"\0\0\0\x0efecr\0\0\0\0\0\0";

    /// `fire` of version 0 and no entry
    const FIRE: &[u8] = b"\0\0\0\x0efire\0\0\0\0\0\0";

    /// `segr` of one session group, of no file group, over the hint track 3
    const SEGR: &[u8] = b"\0\0\0\x11segr\0\x01\0\0\x01\0\0\0\x03";

    /// `gitn` of no entry
    const GITN: &[u8] = b"\0\0\0\x0egitn\0\0\0\0\0\0";

    /// `free` holding nothing, a box neither box holds a field for
    const FREE: &[u8] = b"\0\0\0\x08free";

    fn partition_entry() -> PartitionEntry {
        PartitionEntry {
            fpar: FilePartitionBox {
                item_id: 1,
                ..FilePartitionBox::default()
            },
            fecr: Some(FecReservoirBox::default()),
            fire: Some(FileReservoirBox::default()),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    fn item_information() -> FdItemInformationBox {
        FdItemInformationBox {
            paen: vec![partition_entry(), partition_entry()],
            segr: Some(FdSessionGroupBox {
                session_groups: vec![SessionGroup {
                    group_ids: vec![],
                    hint_track_ids: vec![3],
                }],
            }),
            gitn: Some(GroupIdToNameBox::default()),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(FREE).unwrap()),
        }
    }

    #[test]
    fn a_partition_entry_reads_its_children_in_any_order_and_writes_them_in_the_order_of_its_syntax()
     {
        let read_order = [b"\0\0\0\x49paen".as_slice(), FREE, FIRE, FECR, FPAR].concat();
        let syntax_order = [b"\0\0\0\x49paen".as_slice(), FPAR, FECR, FIRE, FREE].concat();

        assert_eq!(
            PartitionEntry::decode(&read_order),
            Ok((partition_entry(), b"".as_slice()))
        );
        assert_eq!(encoded(&partition_entry()), syntax_order);
    }

    #[test]
    fn a_partition_entry_without_its_file_partition_box_is_refused() {
        assert_eq!(
            PartitionEntry::decode_payload(FECR).map_err(|error| error.kind()),
            Err(Error::missing_mandatory_box(FilePartitionBox::BOX_TYPE).kind())
        );
    }

    #[test]
    fn reads_its_children_in_any_order_and_keeps_the_child_it_has_no_field_for() {
        let paen = [b"\0\0\0\x49paen".as_slice(), FPAR, FECR, FIRE, FREE].concat();
        let bytes = [
            b"\0\0\0\xc7fiin\0\0\0\0\0\x02".as_slice(),
            GITN,
            &paen,
            FREE,
            SEGR,
            &paen,
        ]
        .concat();

        assert_eq!(
            FdItemInformationBox::decode(&bytes),
            Ok((item_information(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_partition_entries_then_segr_then_gitn_then_the_child_it_has_no_field_for() {
        let paen = [b"\0\0\0\x49paen".as_slice(), FPAR, FECR, FIRE, FREE].concat();

        assert_eq!(
            encoded(&item_information()),
            [
                b"\0\0\0\xc7fiin\0\0\0\0\0\x02".as_slice(),
                &paen,
                &paen,
                SEGR,
                GITN,
                FREE
            ]
            .concat()
        );
    }

    #[test]
    fn an_entry_count_other_than_the_number_of_partition_entries_is_refused() {
        assert_eq!(
            FdItemInformationBox::decode_payload(b"\0\0\0\0\0\x07"),
            Err(Error::entry_count_mismatch(7, 0))
        );
    }

    #[test]
    fn a_partition_entry_that_does_not_read_fails_the_box() {
        let bytes = [
            b"\0\0\0\x18fiin\0\0\0\0\0\x01".as_slice(),
            b"\0\0\0\x0apaen\0\0",
        ]
        .concat();

        assert!(FdItemInformationBox::decode(&bytes).is_err());
    }

    #[test]
    fn a_payload_ending_inside_entry_count_is_refused() {
        assert_eq!(
            FdItemInformationBox::decode_payload(b"\0\0\0\0\0"),
            Err(Error::truncated_payload(6, 5))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            FdItemInformationBox::decode_payload(b"\x01\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
