//! [`ReceivedSrtpHintSampleEntry`], the sample entry of a track recording an SRTP stream

use isobmff_core::{BoxDefinition, BoxEncode, BoxType, Error, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::hint_entry_boxes::{
    ReceivedCryptoContextIdBox, ReceivedSsrcBox, RolloverCounterBox, TimeOffsetBox,
    TimescaleEntryBox, TimestampSynchronyBox,
};
use crate::sample_entry::rtp_hint::{FIELDS_LEN, RtpEntry, read_entry, write_fields};
use crate::sample_entry::{
    SampleEntry, children_len, encode_children, encode_optional, from_class,
};
use crate::srpp::SrtpProcessBox;

/// Box types of the boxes ISO/IEC 14496-12 defines for the class
const DEFINED: [BoxType; 8] = [
    TimescaleEntryBox::BOX_TYPE,
    TimeOffsetBox::BOX_TYPE,
    TimestampSynchronyBox::BOX_TYPE,
    ReceivedSsrcBox::BOX_TYPE,
    ReceivedCryptoContextIdBox::BOX_TYPE,
    RolloverCounterBox::BOX_TYPE,
    SrtpProcessBox::BOX_TYPE,
    BitRateBox::BOX_TYPE,
];

/// [`ReceivedSrtpHintSampleEntry`] (`rsrp`), ISO/IEC 14496-12 §9.4.3.2
///
/// The sample entry of an SRTP reception hint track, whose samples are the
/// SRTP packets of one stream as received, their payloads encrypted. It is
/// laid out as a [`ReceivedRtpHintSampleEntry`](crate::ReceivedRtpHintSampleEntry)
/// is, with the boxes that recover the cryptographic context and the `srpp`
/// of the algorithms that were applied. The bytes of a [`SampleEntry`] after
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
/// ISO/IEC 14496-12 being the `tims` the entry holds exactly one of, `tsro`,
/// `tssy`, the `rssr` it holds exactly one of, `ccid`, `sroc`, the `srpp` it
/// holds exactly one of, then `btrt`.
#[doc(alias = "rsrp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ReceivedSrtpHintSampleEntry {
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
    pub rssr: ReceivedSsrcBox,
    /// Session the packets were received over, from which the cryptographic context is recovered
    pub ccid: Option<ReceivedCryptoContextIdBox>,
    /// Rollover counter of the packets
    pub sroc: Option<RolloverCounterBox>,
    /// SRTP algorithms that were applied to the packets
    pub srpp: SrtpProcessBox,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl ReceivedSrtpHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"rsrp");
}

impl TryFrom<&SampleEntry> for ReceivedSrtpHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `rsrp`.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox):
    ///   the entry holds no `tims`, no `rssr` or no `srpp`.
    /// * The failures of a child that does not frame, of a `tims`, an `rssr`
    ///   or an `srpp` that does not read, and of a box of ISO/IEC 14496-12
    ///   given more than once.
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
            rssr: children.take_exactly_one()?,
            ccid: children.take_zero_or_one_if_decoded()?,
            sroc: children.take_zero_or_one_if_decoded()?,
            srpp: children.take_exactly_one()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&ReceivedSrtpHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `rsrp` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &ReceivedSrtpHintSampleEntry) -> Result<Self, Error> {
        let remaining_len = children_len(
            &hint.derived_boxes,
            [
                hint.tims.encoded_len(),
                hint.tsro.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.tssy.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.rssr.encoded_len(),
                hint.ccid.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.sroc.as_ref().map_or(0, BoxEncode::encoded_len),
                hint.srpp.encoded_len(),
                hint.btrt.as_ref().map_or(0, BoxEncode::encoded_len),
            ],
            &hint.extra_boxes,
        );

        from_class(
            ReceivedSrtpHintSampleEntry::FORMAT,
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
                        let rest = hint.rssr.encode(rest)?;
                        let rest = encode_optional(rest, hint.ccid.as_ref())?;
                        let rest = encode_optional(rest, hint.sroc.as_ref())?;
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
    use isobmff_core::{BoxDecode, BoxDefinition, Error, FourCc};

    use super::ReceivedSrtpHintSampleEntry;
    use crate::hint_entry_boxes::{
        ReceivedSsrcBox, RolloverCounterBox, TimeOffsetBox, TimescaleEntryBox,
    };
    use crate::sample_entry::SampleEntry;
    use crate::srpp::SrtpProcessBox;

    /// Fields of version 1, packets of at most 1500 bytes
    const FIELDS: &[u8] = b"\0\x01\0\x01\0\0\x05\xdc";

    /// `tims` of 90000 units a second
    const TIMS: &[u8] = b"\0\0\0\x0ctims\0\x01\x5f\x90";

    /// `tsro` of the first RTP timestamp 1328080
    const TSRO: &[u8] = b"\0\0\0\x0ctsro\0\x14\x43\xd0";

    /// `rssr` of the SSRC 0x12345678
    const RSSR: &[u8] = b"\0\0\0\x0crssr\x12\x34\x56\x78";

    /// `sroc` of the rollover counter 2
    const SROC: &[u8] = b"\0\0\0\x0csroc\0\0\0\x02";

    /// `srpp` of AES in counter mode and HMAC-SHA-1 for RTP
    const SRPP: &[u8] = b"\0\0\0\x1csrpp\0\0\0\0ACM1    SHM2    ";

    fn sample_entry(remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"rsrp"),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    #[test]
    fn writes_its_boxes_in_the_order_of_its_class_whatever_order_they_were_read_in() {
        let hint = ReceivedSrtpHintSampleEntry {
            data_reference_index: 1,
            max_packet_size: 1500,
            tims: TimescaleEntryBox { timescale: 90000 },
            tsro: Some(TimeOffsetBox { offset: 1_328_080 }),
            rssr: ReceivedSsrcBox { ssrc: 0x1234_5678 },
            sroc: Some(RolloverCounterBox {
                rollover_counter: 2,
            }),
            srpp: SrtpProcessBox::decode(SRPP).unwrap().0,
            ..ReceivedSrtpHintSampleEntry::default()
        };

        assert_eq!(
            ReceivedSrtpHintSampleEntry::try_from(&sample_entry(&[
                FIELDS, SRPP, SROC, RSSR, TSRO, TIMS
            ])),
            Ok(hint.clone())
        );
        assert_eq!(
            SampleEntry::try_from(&hint),
            Ok(sample_entry(&[FIELDS, TIMS, TSRO, RSSR, SROC, SRPP]))
        );
    }

    #[test]
    fn an_entry_without_its_ssrc_is_refused() {
        assert_eq!(
            ReceivedSrtpHintSampleEntry::try_from(&sample_entry(&[FIELDS, TIMS, SRPP])).map(|_| ()),
            Err(Error::missing_mandatory_box(ReceivedSsrcBox::BOX_TYPE))
        );
    }
}
