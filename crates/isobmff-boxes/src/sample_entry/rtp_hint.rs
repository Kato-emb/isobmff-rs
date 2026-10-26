//! [`RtpHintSampleEntry`] and [`SrtpHintSampleEntry`], the sample entries of an RTP or SRTP server hint track, and the fields the RTP classes share

use isobmff_core::{
    BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader, FieldWriter, FourCc,
    OtherBoxes,
};

use crate::btrt::BitRateBox;
use crate::hint_entry_boxes::{SequenceOffsetBox, TimeOffsetBox, TimescaleEntryBox};
use crate::sample_entry::{
    HINT_TRACK_VERSION, SampleEntry, children_len, encode_children, encode_optional, from_class,
    of_format, split_children,
};
use crate::srpp::SrtpProcessBox;

/// Length of the fields the RTP classes lay out after `data_reference_index`: `hinttrackversion`, `highestcompatibleversion` and `maxpacketsize`
pub(super) const FIELDS_LEN: u64 = 2 + 2 + 4;

/// Fields every RTP class lays out after `data_reference_index`, and the children after them, as [`read_entry`] reads them
pub(super) struct RtpEntry<'payload> {
    pub(super) hint_track_version: u16,
    pub(super) highest_compatible_version: u16,
    pub(super) max_packet_size: u32,
    pub(super) derived_boxes: OtherBoxes,
    pub(super) children: ChildBoxes<'payload>,
}

/// Reads `hinttrackversion`, `highestcompatibleversion` and `maxpacketsize` from the bytes of `entry`, an entry of the format `format`, after `data_reference_index`, and splits the children after them before the first box of `defined`, as [`split_children`] does
///
/// # Errors
///
/// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
///   `format`: the entry is of another format.
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   bytes end inside a field, counted from the end of
///   `data_reference_index`.
/// * The failures of [`split_children`].
pub(super) fn read_entry<'payload>(
    entry: &'payload SampleEntry,
    format: FourCc,
    defined: &[BoxType],
) -> Result<RtpEntry<'payload>, Error> {
    of_format(entry, format)?;
    let mut reader = FieldReader::new(&entry.remaining_data);
    let hint_track_version = reader.read_u16()?;
    let highest_compatible_version = reader.read_u16()?;
    let max_packet_size = reader.read_u32()?;
    let (derived_boxes, children) = split_children(reader.take_remainder(), defined)?;

    Ok(RtpEntry {
        hint_track_version,
        highest_compatible_version,
        max_packet_size,
        derived_boxes,
        children,
    })
}

/// Writes `hinttrackversion`, `highestcompatibleversion` and `maxpacketsize`, the fields [`read_entry`] reads
///
/// # Errors
///
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
///   buffer ends inside a field.
pub(super) fn write_fields(
    writer: &mut FieldWriter<'_>,
    hint_track_version: u16,
    highest_compatible_version: u16,
    max_packet_size: u32,
) -> Result<(), Error> {
    writer.write_u16(hint_track_version)?;
    writer.write_u16(highest_compatible_version)?;
    writer.write_u32(max_packet_size)
}

/// Box types of the boxes ISO/IEC 14496-12 defines for [`RtpHintSampleEntry`]
const RTP_DEFINED: [BoxType; 4] = [
    TimescaleEntryBox::BOX_TYPE,
    TimeOffsetBox::BOX_TYPE,
    SequenceOffsetBox::BOX_TYPE,
    BitRateBox::BOX_TYPE,
];

/// [`RtpHintSampleEntry`] (`rtp `), ISO/IEC 14496-12 §9.1.2.1
///
/// The sample entry of an RTP server hint track, whose samples a server turns
/// into RTP packets. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// the boxes of §9.1.2.1, `tims`, which the entry holds exactly one of, then
/// `tsro` and `snro`, then `btrt`, which any sample entry may hold; and the
/// boxes no definition of the class names, in
/// [`extra_boxes`](Self::extra_boxes). A child of a type this crate does not
/// read that lies before the first box of ISO/IEC 14496-12 is read into the
/// first part, and one that lies after it into the third.
#[doc(alias = "rtp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct RtpHintSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Version of the hint track format, 1
    pub hint_track_version: u16,
    /// Oldest version of the hint track format the track is compatible with, 1
    pub highest_compatible_version: u16,
    /// Size of the largest packet the track forms, in bytes
    pub max_packet_size: u32,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Timescale of the RTP timestamps
    pub tims: TimescaleEntryBox,
    /// Offset to the RTP timestamps, in place of a random one
    pub tsro: Option<TimeOffsetBox>,
    /// Offset to the RTP sequence numbers, in place of a random one
    pub snro: Option<SequenceOffsetBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl RtpHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"rtp ");
}

impl Default for RtpHintSampleEntry {
    fn default() -> Self {
        Self {
            data_reference_index: 0,
            hint_track_version: HINT_TRACK_VERSION,
            highest_compatible_version: HINT_TRACK_VERSION,
            max_packet_size: 0,
            derived_boxes: OtherBoxes::new(),
            tims: TimescaleEntryBox::default(),
            tsro: None,
            snro: None,
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

impl TryFrom<&SampleEntry> for RtpHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `rtp `.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox):
    ///   the entry holds no `tims`.
    /// * The failures of a child that does not frame, of a `tims` that does
    ///   not read, and of a box of ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let RtpEntry {
            hint_track_version,
            highest_compatible_version,
            max_packet_size,
            derived_boxes,
            mut children,
        } = read_entry(entry, Self::FORMAT, &RTP_DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            hint_track_version,
            highest_compatible_version,
            max_packet_size,
            derived_boxes,
            tims: children.take_exactly_one()?,
            tsro: children.take_zero_or_one_if_decoded()?,
            snro: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&RtpHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `rtp ` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &RtpHintSampleEntry) -> Result<Self, Error> {
        let remaining_len = children_len(
            &hint.derived_boxes,
            [
                hint.tims.encoded_len(),
                hint.tsro.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.snro.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.btrt.as_ref().map_or(0, BoxEncode::encoded_len),
            ],
            &hint.extra_boxes,
        );

        from_class(
            RtpHintSampleEntry::FORMAT,
            hint.data_reference_index,
            remaining_len.saturating_add(FIELDS_LEN),
            |writer| {
                write_fields(
                    writer,
                    hint.hint_track_version,
                    hint.highest_compatible_version,
                    hint.max_packet_size,
                )?;
                encode_children(
                    writer.take_remainder(),
                    &hint.derived_boxes,
                    |rest| {
                        let rest = hint.tims.encode(rest)?;
                        let rest = encode_optional(rest, hint.tsro.as_ref())?;
                        let rest = encode_optional(rest, hint.snro.as_ref())?;
                        encode_optional(rest, hint.btrt.as_ref())
                    },
                    &hint.extra_boxes,
                )
            },
        )
    }
}

/// Box types of the boxes ISO/IEC 14496-12 defines for [`SrtpHintSampleEntry`]
const SRTP_DEFINED: [BoxType; 5] = [
    TimescaleEntryBox::BOX_TYPE,
    TimeOffsetBox::BOX_TYPE,
    SequenceOffsetBox::BOX_TYPE,
    SrtpProcessBox::BOX_TYPE,
    BitRateBox::BOX_TYPE,
];

/// [`SrtpHintSampleEntry`] (`srtp`), ISO/IEC 14496-12 §9.1.2.1
///
/// The sample entry of an SRTP server hint track, laid out as an
/// [`RtpHintSampleEntry`] is, with the `srpp` that tells a server the SRTP
/// processing to apply. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// # Children
///
/// The children are laid out in three parts as those of an
/// [`RtpHintSampleEntry`] are, the boxes of ISO/IEC 14496-12 being `tims`,
/// `tsro` and `snro`, then the `srpp` the entry holds exactly one of, then
/// `btrt`.
#[doc(alias = "srtp")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct SrtpHintSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Version of the hint track format, 1
    pub hint_track_version: u16,
    /// Oldest version of the hint track format the track is compatible with, 1
    pub highest_compatible_version: u16,
    /// Size of the largest packet the track forms, in bytes
    pub max_packet_size: u32,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Timescale of the RTP timestamps
    pub tims: TimescaleEntryBox,
    /// Offset to the RTP timestamps, in place of a random one
    pub tsro: Option<TimeOffsetBox>,
    /// Offset to the RTP sequence numbers, in place of a random one
    pub snro: Option<SequenceOffsetBox>,
    /// SRTP algorithms the server applies
    pub srpp: SrtpProcessBox,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl SrtpHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"srtp");
}

impl Default for SrtpHintSampleEntry {
    fn default() -> Self {
        Self {
            data_reference_index: 0,
            hint_track_version: HINT_TRACK_VERSION,
            highest_compatible_version: HINT_TRACK_VERSION,
            max_packet_size: 0,
            derived_boxes: OtherBoxes::new(),
            tims: TimescaleEntryBox::default(),
            tsro: None,
            snro: None,
            srpp: SrtpProcessBox::default(),
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

impl TryFrom<&SampleEntry> for SrtpHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `srtp`.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox):
    ///   the entry holds no `tims` or no `srpp`.
    /// * The failures of a child that does not frame, of a `tims` or an
    ///   `srpp` that does not read, and of a box of ISO/IEC 14496-12 given
    ///   more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let RtpEntry {
            hint_track_version,
            highest_compatible_version,
            max_packet_size,
            derived_boxes,
            mut children,
        } = read_entry(entry, Self::FORMAT, &SRTP_DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            hint_track_version,
            highest_compatible_version,
            max_packet_size,
            derived_boxes,
            tims: children.take_exactly_one()?,
            tsro: children.take_zero_or_one_if_decoded()?,
            snro: children.take_zero_or_one_if_decoded()?,
            srpp: children.take_exactly_one()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&SrtpHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `srtp` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &SrtpHintSampleEntry) -> Result<Self, Error> {
        let remaining_len = children_len(
            &hint.derived_boxes,
            [
                hint.tims.encoded_len(),
                hint.tsro.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.snro.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.srpp.encoded_len(),
                hint.btrt.as_ref().map_or(0, BoxEncode::encoded_len),
            ],
            &hint.extra_boxes,
        );

        from_class(
            SrtpHintSampleEntry::FORMAT,
            hint.data_reference_index,
            remaining_len.saturating_add(FIELDS_LEN),
            |writer| {
                write_fields(
                    writer,
                    hint.hint_track_version,
                    hint.highest_compatible_version,
                    hint.max_packet_size,
                )?;
                encode_children(
                    writer.take_remainder(),
                    &hint.derived_boxes,
                    |rest| {
                        let rest = hint.tims.encode(rest)?;
                        let rest = encode_optional(rest, hint.tsro.as_ref())?;
                        let rest = encode_optional(rest, hint.snro.as_ref())?;
                        let rest = hint.srpp.encode(rest)?;
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
    use isobmff_core::{BoxDecode, BoxDefinition, BoxType, ChildBoxes, Error, FourCc, OtherBoxes};

    use super::{RtpHintSampleEntry, SrtpHintSampleEntry};
    use crate::hint_entry_boxes::{SequenceOffsetBox, TimeOffsetBox, TimescaleEntryBox};
    use crate::sample_entry::SampleEntry;
    use crate::srpp::SrtpProcessBox;

    /// Fields of version 1, packets of at most 1450 bytes
    const FIELDS: &[u8] = b"\0\x01\0\x01\0\0\x05\xaa";

    /// `tims` of 90000 units a second
    const TIMS: &[u8] = b"\0\0\0\x0ctims\0\x01\x5f\x90";

    /// `tsro` of the offset 0
    const TSRO: &[u8] = b"\0\0\0\x0ctsro\0\0\0\0";

    /// `snro` of the offset 7
    const SNRO: &[u8] = b"\0\0\0\x0csnro\0\0\0\x07";

    /// `srpp` of AES in counter mode and HMAC-SHA-1 for RTP
    const SRPP: &[u8] = b"\0\0\0\x1csrpp\0\0\0\0ACM1    SHM2    ";

    /// Box of a type this crate does not read
    const UNKNOWN: &[u8] = b"\0\0\0\x09xtra\x07";

    fn sample_entry(format: &[u8; 4], remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*format),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    fn rtp() -> RtpHintSampleEntry {
        RtpHintSampleEntry {
            data_reference_index: 1,
            max_packet_size: 1450,
            tims: TimescaleEntryBox { timescale: 90000 },
            tsro: Some(TimeOffsetBox { offset: 0 }),
            snro: Some(SequenceOffsetBox { offset: 7 }),
            extra_boxes: OtherBoxes::from(ChildBoxes::decode(UNKNOWN).unwrap()),
            ..RtpHintSampleEntry::default()
        }
    }

    #[test]
    fn an_rtp_entry_reads_its_fields_and_writes_its_boxes_in_the_order_of_its_class() {
        let read = RtpHintSampleEntry::try_from(&sample_entry(
            b"rtp ",
            &[FIELDS, SNRO, TIMS, UNKNOWN, TSRO],
        ))
        .unwrap();

        assert_eq!(
            (SampleEntry::try_from(&read), read),
            (
                Ok(sample_entry(b"rtp ", &[FIELDS, TIMS, TSRO, SNRO, UNKNOWN])),
                rtp()
            )
        );
    }

    #[test]
    fn an_rtp_entry_without_its_timescale_is_refused() {
        assert_eq!(
            RtpHintSampleEntry::try_from(&sample_entry(b"rtp ", &[FIELDS, TSRO])).map(|_| ()),
            Err(Error::missing_mandatory_box(TimescaleEntryBox::BOX_TYPE))
        );
    }

    #[test]
    fn an_rtp_entry_ending_inside_its_fields_is_refused() {
        assert_eq!(
            RtpHintSampleEntry::try_from(&sample_entry(b"rtp ", &[b"\0\x01\0\x01\0"])),
            Err(Error::truncated_payload(8, 5))
        );
    }

    #[test]
    fn an_srtp_entry_writes_its_process_box_after_the_offsets() {
        let hint = SrtpHintSampleEntry {
            data_reference_index: 1,
            max_packet_size: 1450,
            tims: TimescaleEntryBox { timescale: 90000 },
            srpp: SrtpProcessBox::decode(SRPP).unwrap().0,
            ..SrtpHintSampleEntry::default()
        };

        assert_eq!(
            SrtpHintSampleEntry::try_from(&sample_entry(b"srtp", &[FIELDS, SRPP, TIMS])),
            Ok(hint.clone())
        );
        assert_eq!(
            SampleEntry::try_from(&hint),
            Ok(sample_entry(b"srtp", &[FIELDS, TIMS, SRPP]))
        );
    }

    #[test]
    fn an_srtp_entry_without_its_process_box_is_refused() {
        assert_eq!(
            SrtpHintSampleEntry::try_from(&sample_entry(b"srtp", &[FIELDS, TIMS])).map(|_| ()),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"srpp")))
        );
    }

    #[test]
    fn an_entry_of_another_format_is_refused() {
        assert_eq!(
            RtpHintSampleEntry::try_from(&sample_entry(b"srtp", &[FIELDS, TIMS, SRPP])),
            Err(Error::unsupported_value("format"))
        );
    }
}
