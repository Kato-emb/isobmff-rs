//! [`ReceivedRtpHintSampleEntry`] and [`ProtectedRtpReceptionHintSampleEntry`], the sample entries of a track recording an RTP stream

use alloc::vec::Vec;

use isobmff_core::{BoxDefinition, BoxEncode, BoxType, Error, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::hint_entry_boxes::{
    ReceivedSsrcBox, TimeOffsetBox, TimescaleEntryBox, TimestampSynchronyBox,
};
use crate::rinf::RestrictedSchemeInfoBox;
use crate::sample_entry::rtp_hint::{FIELDS_LEN, RtpEntry, read_entry, write_fields};
use crate::sample_entry::{
    AnyEntryBoxes, SampleEntry, children_len, encode_children, encode_optional, from_class,
};
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;

/// Box types of the boxes ISO/IEC 14496-12 defines for both classes
const DEFINED: [BoxType; 4] = [
    TimescaleEntryBox::BOX_TYPE,
    TimeOffsetBox::BOX_TYPE,
    TimestampSynchronyBox::BOX_TYPE,
    ReceivedSsrcBox::BOX_TYPE,
];

/// [`ReceivedRtpHintSampleEntry`] (`rrtp`), ISO/IEC 14496-12 §9.4.1.2
///
/// The sample entry of an RTP reception hint track, whose samples are the RTP
/// packets of one stream as received. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// `hinttrackversion` and `highestcompatibleversion`, which the Syntax sets
/// to 1, are read past and written as 1.
///
/// # Children
///
/// The children are laid out in three parts as those of an
/// [`RtpHintSampleEntry`](crate::RtpHintSampleEntry) are, the boxes of
/// ISO/IEC 14496-12 being the `tims` the entry holds exactly one of, then
/// `tsro`, `tssy` and `rssr`, then `sinf`, `scrb`, `rinf` and `btrt`.
#[doc(alias = "rrtp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ReceivedRtpHintSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Size of the largest packet received, in bytes
    pub max_packet_size: u32,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Clock frequency of the RTP timestamps of the stream
    pub tims: TimescaleEntryBox,
    /// First RTP timestamp of the stream in reception order, 0 where absent
    pub tsro: Option<TimeOffsetBox>,
    /// Whether the RTP timestamps are synchronized with those of the other reception hint tracks, 0 where absent
    pub tssy: Option<TimestampSynchronyBox>,
    /// SSRC of the packets
    pub rssr: Option<ReceivedSsrcBox>,
    /// Protection schemes of a protected sample entry, alternative descriptions of the same protection
    pub sinf: Vec<ProtectionSchemeInfoBox>,
    /// Scrambling schemes applied to the samples
    pub scrb: Vec<ScrambleSchemeInfoBox>,
    /// Restriction of a restricted sample entry, or advisory information on the restriction of one whose type was not transformed
    pub rinf: Option<RestrictedSchemeInfoBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl ReceivedRtpHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"rrtp");
}

impl TryFrom<&SampleEntry> for ReceivedRtpHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `rrtp`.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox):
    ///   the entry holds no `tims`.
    /// * The failures of a child that does not frame, of a `tims` that does
    ///   not read, and of a box of ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let RtpEntry {
            max_packet_size,
            derived_boxes,
            mut children,
        } = read_entry(entry, Self::FORMAT, &DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            max_packet_size,
            derived_boxes,
            tims: children.take_exactly_one()?,
            tsro: children.take_zero_or_one_if_decoded()?,
            tssy: children.take_zero_or_one_if_decoded()?,
            rssr: children.take_zero_or_one_if_decoded()?,
            sinf: children.take_zero_or_more_if_decoded(),
            scrb: children.take_zero_or_more_if_decoded(),
            rinf: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&ReceivedRtpHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `rrtp` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &ReceivedRtpHintSampleEntry) -> Result<Self, Error> {
        let any_entry = AnyEntryBoxes {
            sinf: &hint.sinf,
            scrb: &hint.scrb,
            rinf: hint.rinf.as_ref(),
            btrt: hint.btrt.as_ref(),
        };
        let remaining_len = children_len(
            &hint.derived_boxes,
            [
                hint.tims.encoded_len(),
                hint.tsro.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.tssy.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.rssr.as_ref().map_or(0, BoxEncode::encoded_len),
                any_entry.encoded_len(),
            ],
            &hint.extra_boxes,
        );

        from_class(
            ReceivedRtpHintSampleEntry::FORMAT,
            hint.data_reference_index,
            remaining_len.saturating_add(FIELDS_LEN),
            |writer| {
                write_fields(writer, hint.max_packet_size)?;
                encode_children(
                    writer.take_remainder(),
                    &hint.derived_boxes,
                    |rest| {
                        let rest = hint.tims.encode(rest)?;
                        let rest = encode_optional(rest, hint.tsro.as_ref())?;
                        let rest = encode_optional(rest, hint.tssy.as_ref())?;
                        let rest = encode_optional(rest, hint.rssr.as_ref())?;
                        any_entry.encode(rest)
                    },
                    &hint.extra_boxes,
                )
            },
        )
    }
}

/// [`ProtectedRtpReceptionHintSampleEntry`] (`prtp`), ISO/IEC 14496-12 §9.4.5
///
/// The sample entry of an RTP reception hint track whose packets carry
/// protected media, laid out as a [`ReceivedRtpHintSampleEntry`] is, with the
/// `sinf` that names the protection scheme and the original format. The
/// track itself is in the clear. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// `hinttrackversion` and `highestcompatibleversion`, which the Syntax sets
/// to 1, are read past and written as 1.
///
/// # Children
///
/// The children are laid out in three parts as those of a
/// [`ReceivedRtpHintSampleEntry`] are. The `sinf` it requires
/// is among the boxes any sample entry may hold.
#[doc(alias = "prtp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ProtectedRtpReceptionHintSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Size of the largest packet received, in bytes
    pub max_packet_size: u32,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Clock frequency of the RTP timestamps of the stream
    pub tims: TimescaleEntryBox,
    /// First RTP timestamp of the stream in reception order, 0 where absent
    pub tsro: Option<TimeOffsetBox>,
    /// Whether the RTP timestamps are synchronized with those of the other reception hint tracks, 0 where absent
    pub tssy: Option<TimestampSynchronyBox>,
    /// SSRC of the packets
    pub rssr: Option<ReceivedSsrcBox>,
    /// Protection schemes of a protected sample entry, alternative descriptions of the same protection
    pub sinf: Vec<ProtectionSchemeInfoBox>,
    /// Scrambling schemes applied to the samples
    pub scrb: Vec<ScrambleSchemeInfoBox>,
    /// Restriction of a restricted sample entry, or advisory information on the restriction of one whose type was not transformed
    pub rinf: Option<RestrictedSchemeInfoBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl ProtectedRtpReceptionHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"prtp");
}

impl TryFrom<&SampleEntry> for ProtectedRtpReceptionHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `prtp`.
    /// * The other failures of [`ReceivedRtpHintSampleEntry::try_from`].
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let RtpEntry {
            max_packet_size,
            derived_boxes,
            mut children,
        } = read_entry(entry, Self::FORMAT, &DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            max_packet_size,
            derived_boxes,
            tims: children.take_exactly_one()?,
            tsro: children.take_zero_or_one_if_decoded()?,
            tssy: children.take_zero_or_one_if_decoded()?,
            rssr: children.take_zero_or_one_if_decoded()?,
            sinf: children.take_zero_or_more_if_decoded(),
            scrb: children.take_zero_or_more_if_decoded(),
            rinf: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&ProtectedRtpReceptionHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `prtp` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &ProtectedRtpReceptionHintSampleEntry) -> Result<Self, Error> {
        let any_entry = AnyEntryBoxes {
            sinf: &hint.sinf,
            scrb: &hint.scrb,
            rinf: hint.rinf.as_ref(),
            btrt: hint.btrt.as_ref(),
        };
        let remaining_len = children_len(
            &hint.derived_boxes,
            [
                hint.tims.encoded_len(),
                hint.tsro.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.tssy.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.rssr.as_ref().map_or(0, BoxEncode::encoded_len),
                any_entry.encoded_len(),
            ],
            &hint.extra_boxes,
        );

        from_class(
            ProtectedRtpReceptionHintSampleEntry::FORMAT,
            hint.data_reference_index,
            remaining_len.saturating_add(FIELDS_LEN),
            |writer| {
                write_fields(writer, hint.max_packet_size)?;
                encode_children(
                    writer.take_remainder(),
                    &hint.derived_boxes,
                    |rest| {
                        let rest = hint.tims.encode(rest)?;
                        let rest = encode_optional(rest, hint.tsro.as_ref())?;
                        let rest = encode_optional(rest, hint.tssy.as_ref())?;
                        let rest = encode_optional(rest, hint.rssr.as_ref())?;
                        any_entry.encode(rest)
                    },
                    &hint.extra_boxes,
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxDefinition, Error, FourCc};

    use super::{ProtectedRtpReceptionHintSampleEntry, ReceivedRtpHintSampleEntry};
    use crate::hint_entry_boxes::{TimeOffsetBox, TimescaleEntryBox, TimestampSynchronyBox};
    use crate::sample_entry::SampleEntry;
    use crate::sinf::ProtectionSchemeInfoBox;

    /// Fields of version 1 and no largest packet size, as recorders write them
    const FIELDS: &[u8] = b"\0\x01\0\x01\xff\xff\xff\xff";

    /// `tims` of 8000 units a second
    const TIMS: &[u8] = b"\0\0\0\x0ctims\0\0\x1f\x40";

    /// `tsro` of the first RTP timestamp 1328080
    const TSRO: &[u8] = b"\0\0\0\x0ctsro\0\x14\x43\xd0";

    /// `tssy` of timestamps as received
    const TSSY: &[u8] = b"\0\0\0\x09tssy\x01";

    /// `sinf` of the original format `rrtp`
    const SINF: &[u8] = b"\0\0\0\x14sinf\0\0\0\x0cfrmarrtp";

    fn sample_entry(format: &[u8; 4], remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*format),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    fn received() -> ReceivedRtpHintSampleEntry {
        ReceivedRtpHintSampleEntry {
            data_reference_index: 1,
            max_packet_size: u32::MAX,
            tims: TimescaleEntryBox { timescale: 8000 },
            tsro: Some(TimeOffsetBox { offset: 1_328_080 }),
            tssy: Some(TimestampSynchronyBox { timestamp_sync: 1 }),
            ..ReceivedRtpHintSampleEntry::default()
        }
    }

    #[test]
    fn a_received_rtp_entry_writes_its_boxes_in_the_order_of_its_class() {
        assert_eq!(
            ReceivedRtpHintSampleEntry::try_from(&sample_entry(
                b"rrtp",
                &[FIELDS, TSSY, TSRO, TIMS]
            )),
            Ok(received())
        );
        assert_eq!(
            SampleEntry::try_from(&received()),
            Ok(sample_entry(b"rrtp", &[FIELDS, TIMS, TSRO, TSSY]))
        );
    }

    #[test]
    fn a_received_rtp_entry_without_its_timescale_is_refused() {
        assert_eq!(
            ReceivedRtpHintSampleEntry::try_from(&sample_entry(b"rrtp", &[FIELDS, TSRO]))
                .map(|_| ()),
            Err(Error::missing_mandatory_box(TimescaleEntryBox::BOX_TYPE))
        );
    }

    #[test]
    fn a_protected_entry_writes_its_protection_scheme_after_the_boxes_of_its_class() {
        let entry = sample_entry(b"prtp", &[FIELDS, SINF, TIMS]);
        let hint = ProtectedRtpReceptionHintSampleEntry {
            data_reference_index: 1,
            max_packet_size: u32::MAX,
            tims: TimescaleEntryBox { timescale: 8000 },
            sinf: vec![ProtectionSchemeInfoBox::decode(SINF).unwrap().0],
            ..ProtectedRtpReceptionHintSampleEntry::default()
        };

        assert_eq!(
            (
                ProtectedRtpReceptionHintSampleEntry::try_from(&entry),
                SampleEntry::try_from(&hint)
            ),
            (
                Ok(hint.clone()),
                Ok(sample_entry(b"prtp", &[FIELDS, TIMS, SINF]))
            )
        );
        assert_eq!(
            ProtectedRtpReceptionHintSampleEntry::try_from(&sample_entry(b"rrtp", &[FIELDS, TIMS]))
                .map(|_| ()),
            Err(Error::unsupported_value("format"))
        );
    }
}
