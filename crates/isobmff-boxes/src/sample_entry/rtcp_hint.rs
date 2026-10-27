//! [`RtcpReceptionHintSampleEntry`] and [`SrtcpReceptionHintSampleEntry`], the sample entries of a track recording an RTCP or SRTCP stream

use alloc::vec::Vec;

use isobmff_core::{Error, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::rinf::RestrictedSchemeInfoBox;
use crate::sample_entry::rtp_hint::{self, FIELDS_LEN, RtpEntry, write_fields};
use crate::sample_entry::{AnyEntryBoxes, SampleEntry, children_len, encode_children, from_class};
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;

/// Fields and boxes an RTCP or SRTCP reception hint entry lays out after `data_reference_index`, which both classes share, as [`read_entry`] reads them
struct RtcpEntry {
    max_packet_size: u32,
    derived_boxes: OtherBoxes,
    sinf: Vec<ProtectionSchemeInfoBox>,
    scrb: Vec<ScrambleSchemeInfoBox>,
    rinf: Option<RestrictedSchemeInfoBox>,
    btrt: Option<BitRateBox>,
    extra_boxes: OtherBoxes,
}

/// Reads the fields and the boxes of an RTCP or SRTCP reception hint entry from the bytes of `entry`, an entry of the format `format`, after `data_reference_index`
///
/// # Errors
///
/// * The failures of [`RtcpReceptionHintSampleEntry::try_from`], `format`
///   standing for `rtcp`.
fn read_entry(entry: &SampleEntry, format: FourCc) -> Result<RtcpEntry, Error> {
    let RtpEntry {
        max_packet_size,
        derived_boxes,
        mut children,
    } = rtp_hint::read_entry(entry, format, &[])?;

    Ok(RtcpEntry {
        max_packet_size,
        derived_boxes,
        sinf: children.take_zero_or_more_if_decoded(),
        scrb: children.take_zero_or_more_if_decoded(),
        rinf: children.take_zero_or_one_if_decoded()?,
        btrt: children.take_zero_or_one_if_decoded()?,
        extra_boxes: OtherBoxes::from(children),
    })
}

/// Fields and boxes of an RTCP or SRTCP reception hint entry to write, borrowed from the class that holds them, as [`RtcpParts::write`] writes them
struct RtcpParts<'class> {
    data_reference_index: u16,
    max_packet_size: u32,
    derived_boxes: &'class OtherBoxes,
    any_entry: AnyEntryBoxes<'class>,
    extra_boxes: &'class OtherBoxes,
}

impl RtcpParts<'_> {
    /// Writes the fields and the boxes into the bytes of an entry of the format `format` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn write(&self, format: FourCc) -> Result<SampleEntry, Error> {
        from_class(
            format,
            self.data_reference_index,
            children_len(
                self.derived_boxes,
                [self.any_entry.encoded_len()],
                self.extra_boxes,
            )
            .saturating_add(FIELDS_LEN),
            |writer| {
                write_fields(writer, self.max_packet_size)?;
                encode_children(
                    writer.take_remainder(),
                    self.derived_boxes,
                    |rest| self.any_entry.encode(rest),
                    self.extra_boxes,
                )
            },
        )
    }
}

/// [`RtcpReceptionHintSampleEntry`] (`rtcp`), ISO/IEC 14496-12 §9.4.2.3
///
/// The sample entry of an RTCP reception hint track, whose samples are the
/// RTCP packets of one stream as received. The subclause names no class: the
/// entry is laid out as an [`RtpHintSampleEntry`](crate::RtpHintSampleEntry)
/// is, with no box defined for its additional data. The bytes of a
/// [`SampleEntry`] after `data_reference_index` are read with [`TryFrom`] and
/// written back with [`SampleEntry::try_from`].
///
/// `hinttrackversion` and `highestcompatibleversion`, which the Syntax sets
/// to 1, are read past and written as 1.
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// the boxes any sample entry may hold, `sinf`, `scrb`, `rinf` and `btrt` in
/// this order; and the boxes no definition of
/// the class names, in [`extra_boxes`](Self::extra_boxes). A child of a type
/// this crate does not read that lies before every box of the second part is read into the first
/// part, and one that lies after it into the third.
#[doc(alias = "rtcp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct RtcpReceptionHintSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Size of the largest packet received, in bytes
    pub max_packet_size: u32,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
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

impl RtcpReceptionHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"rtcp");
}

impl TryFrom<&SampleEntry> for RtcpReceptionHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `rtcp`.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * The failures of a child that does not frame, and of a `rinf` or
    ///   `btrt` given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let RtcpEntry {
            max_packet_size,
            derived_boxes,
            sinf,
            scrb,
            rinf,
            btrt,
            extra_boxes,
        } = read_entry(entry, Self::FORMAT)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            max_packet_size,
            derived_boxes,
            sinf,
            scrb,
            rinf,
            btrt,
            extra_boxes,
        })
    }
}

impl TryFrom<&RtcpReceptionHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `rtcp` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &RtcpReceptionHintSampleEntry) -> Result<Self, Error> {
        RtcpParts {
            data_reference_index: hint.data_reference_index,
            max_packet_size: hint.max_packet_size,
            derived_boxes: &hint.derived_boxes,
            any_entry: AnyEntryBoxes {
                sinf: &hint.sinf,
                scrb: &hint.scrb,
                rinf: hint.rinf.as_ref(),
                btrt: hint.btrt.as_ref(),
            },
            extra_boxes: &hint.extra_boxes,
        }
        .write(RtcpReceptionHintSampleEntry::FORMAT)
    }
}

/// [`SrtcpReceptionHintSampleEntry`] (`stcp`), ISO/IEC 14496-12 §9.4.4.3
///
/// The sample entry of an SRTCP reception hint track, whose samples are the
/// SRTCP packets of one stream as received, laid out as an
/// [`RtcpReceptionHintSampleEntry`] is. The subclause names no class. The
/// `srpp` of the SRTP reception hint track the track refers to gives the
/// algorithms of the packets. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// `hinttrackversion` and `highestcompatibleversion`, which the Syntax sets
/// to 1, are read past and written as 1.
///
/// # Children
///
/// The children are laid out in three parts as those of an
/// [`RtcpReceptionHintSampleEntry`] are.
#[doc(alias = "stcp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SrtcpReceptionHintSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Size of the largest packet received, in bytes
    pub max_packet_size: u32,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
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

impl SrtcpReceptionHintSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"stcp");
}

impl TryFrom<&SampleEntry> for SrtcpReceptionHintSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `stcp`.
    /// * The other failures of [`RtcpReceptionHintSampleEntry::try_from`].
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let RtcpEntry {
            max_packet_size,
            derived_boxes,
            sinf,
            scrb,
            rinf,
            btrt,
            extra_boxes,
        } = read_entry(entry, Self::FORMAT)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            max_packet_size,
            derived_boxes,
            sinf,
            scrb,
            rinf,
            btrt,
            extra_boxes,
        })
    }
}

impl TryFrom<&SrtcpReceptionHintSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `stcp` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &SrtcpReceptionHintSampleEntry) -> Result<Self, Error> {
        RtcpParts {
            data_reference_index: hint.data_reference_index,
            max_packet_size: hint.max_packet_size,
            derived_boxes: &hint.derived_boxes,
            any_entry: AnyEntryBoxes {
                sinf: &hint.sinf,
                scrb: &hint.scrb,
                rinf: hint.rinf.as_ref(),
                btrt: hint.btrt.as_ref(),
            },
            extra_boxes: &hint.extra_boxes,
        }
        .write(SrtcpReceptionHintSampleEntry::FORMAT)
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{ChildBoxes, Error, FourCc, OtherBoxes};

    use super::{RtcpReceptionHintSampleEntry, SrtcpReceptionHintSampleEntry};
    use crate::sample_entry::SampleEntry;

    /// Fields of version 1 and no largest packet size, as recorders write them
    const FIELDS: &[u8] = b"\0\x01\0\x01\xff\xff\xff\xff";

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
    fn an_rtcp_entry_reads_and_writes_back_its_fields_and_boxes() {
        let entry = sample_entry(b"rtcp", &[FIELDS, UNKNOWN]);
        let hint = RtcpReceptionHintSampleEntry {
            data_reference_index: 1,
            max_packet_size: u32::MAX,
            derived_boxes: OtherBoxes::from(ChildBoxes::decode(UNKNOWN).unwrap()),
            ..RtcpReceptionHintSampleEntry::default()
        };

        assert_eq!(
            (
                RtcpReceptionHintSampleEntry::try_from(&entry),
                SampleEntry::try_from(&hint)
            ),
            (Ok(hint.clone()), Ok(entry))
        );
    }

    #[test]
    fn an_srtcp_entry_reads_and_writes_back_its_fields() {
        let entry = sample_entry(b"stcp", &[FIELDS]);
        let hint = SrtcpReceptionHintSampleEntry {
            data_reference_index: 1,
            max_packet_size: u32::MAX,
            ..SrtcpReceptionHintSampleEntry::default()
        };

        assert_eq!(
            (
                SrtcpReceptionHintSampleEntry::try_from(&entry),
                SampleEntry::try_from(&hint)
            ),
            (Ok(hint.clone()), Ok(entry))
        );
        assert_eq!(
            SrtcpReceptionHintSampleEntry::try_from(&sample_entry(b"rtcp", &[FIELDS])),
            Err(Error::unsupported_value("format"))
        );
    }
}
