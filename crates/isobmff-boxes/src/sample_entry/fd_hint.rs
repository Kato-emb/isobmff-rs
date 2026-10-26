//! [`FdHintSampleEntry`], the sample entry of an FD hint track

use isobmff_core::{BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::hint_entry_boxes::TimescaleEntryBox;
use crate::sample_entry::{
    HINT_TRACK_VERSION, SampleEntry, children_len, encode_children, encode_optional, from_class,
    of_format, split_children,
};

/// Length of the fields the class lays out after `data_reference_index`
const FIELDS_LEN: u64 = 2 + 2 + 2 + 2;

/// Box types of the boxes ISO/IEC 14496-12 defines for the class
const DEFINED: [BoxType; 2] = [TimescaleEntryBox::BOX_TYPE, BitRateBox::BOX_TYPE];

/// [`FdHintSampleEntry`] (`fdp `), ISO/IEC 14496-12 §9.2.3
///
/// The sample entry of an FD hint track, whose samples are the instructions
/// to send the ALC/LCT or FLUTE packets of a file. The bytes of a
/// [`SampleEntry`] after `data_reference_index` are read with [`TryFrom`]
/// and written back with [`SampleEntry::try_from`].
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// `tims` then `btrt`, which any sample entry may hold; and the boxes no
/// definition of the class names, in [`extra_boxes`](Self::extra_boxes). A
/// child of a type this crate does not read that lies before the first box
/// of ISO/IEC 14496-12 is read into the first part, and one that lies after
/// it into the third.
#[doc(alias = "fdp", alias = "FDHintSampleEntry")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct FdHintSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Version of the hint track format, 1
    pub hint_track_version: u16,
    /// Oldest version of the hint track format the track is compatible with, 1
    pub highest_compatible_version: u16,
    /// Partition entry of the [`FdItemInformationBox`](crate::FdItemInformationBox) the track sends, 0 for none, such as for an FDT
    pub partition_entry_id: u16,
    /// Percentage of protection overhead the samples use, a fixed-point 8.8 number, as its 16 bits
    pub fec_overhead: u16,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Timescale of the sample times, which give the send times of the packets; without it they give none
    pub tims: Option<TimescaleEntryBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl FdHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"fdp ");
}

impl Default for FdHintSampleEntry {
    fn default() -> Self {
        Self {
            data_reference_index: 0,
            hint_track_version: HINT_TRACK_VERSION,
            highest_compatible_version: HINT_TRACK_VERSION,
            partition_entry_id: 0,
            fec_overhead: 0,
            derived_boxes: OtherBoxes::new(),
            tims: None,
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

impl TryFrom<&SampleEntry> for FdHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `fdp `.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * The failures of a child that does not frame, and of a box of
    ///   ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        of_format(entry, Self::FORMAT)?;
        let mut reader = FieldReader::new(&entry.remaining_data);
        let hint_track_version = reader.read_u16()?;
        let highest_compatible_version = reader.read_u16()?;
        let partition_entry_id = reader.read_u16()?;
        let fec_overhead = reader.read_u16()?;
        let (derived_boxes, mut children) = split_children(reader.take_remainder(), &DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            hint_track_version,
            highest_compatible_version,
            partition_entry_id,
            fec_overhead,
            derived_boxes,
            tims: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&FdHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `fdp ` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &FdHintSampleEntry) -> Result<Self, Error> {
        let remaining_len = children_len(
            &hint.derived_boxes,
            [
                hint.tims.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.btrt.as_ref().map_or(0, BoxEncode::encoded_len),
            ],
            &hint.extra_boxes,
        );

        from_class(
            FdHintSampleEntry::FORMAT,
            hint.data_reference_index,
            remaining_len.saturating_add(FIELDS_LEN),
            |writer| {
                writer.write_u16(hint.hint_track_version)?;
                writer.write_u16(hint.highest_compatible_version)?;
                writer.write_u16(hint.partition_entry_id)?;
                writer.write_u16(hint.fec_overhead)?;
                encode_children(
                    writer.take_remainder(),
                    &hint.derived_boxes,
                    |rest| {
                        let rest = encode_optional(rest, hint.tims.as_ref())?;
                        encode_optional(rest, hint.btrt.as_ref())
                    },
                    &hint.extra_boxes,
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{ChildBoxes, Error, FourCc, OtherBoxes};

    use super::FdHintSampleEntry;
    use crate::hint_entry_boxes::TimescaleEntryBox;
    use crate::sample_entry::SampleEntry;

    /// Fields of version 1, of the partition entry 1 and an overhead of 12.5 %
    const FIELDS: &[u8] = b"\0\x01\0\x01\0\x01\x0c\x80";

    /// `tims` of 1000 units a second
    const TIMS: &[u8] = b"\0\0\0\x0ctims\0\0\x03\xe8";

    /// Box of a type this crate does not read
    const UNKNOWN: &[u8] = b"\0\0\0\x09xtra\x07";

    fn sample_entry(format: &[u8; 4], remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*format),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    #[test]
    fn reads_and_writes_back_its_fields_and_boxes() {
        let entry = sample_entry(b"fdp ", &[FIELDS, UNKNOWN, TIMS]);
        let hint = FdHintSampleEntry {
            data_reference_index: 1,
            partition_entry_id: 1,
            fec_overhead: 0x0c80,
            derived_boxes: OtherBoxes::from(ChildBoxes::decode(UNKNOWN).unwrap()),
            tims: Some(TimescaleEntryBox { timescale: 1000 }),
            ..FdHintSampleEntry::default()
        };

        assert_eq!(
            (
                FdHintSampleEntry::try_from(&entry),
                SampleEntry::try_from(&hint)
            ),
            (Ok(hint.clone()), Ok(entry))
        );
    }

    #[test]
    fn an_entry_of_another_format_is_refused() {
        assert_eq!(
            FdHintSampleEntry::try_from(&sample_entry(b"rtp ", &[FIELDS, TIMS])),
            Err(Error::unsupported_value("format"))
        );
    }
}
