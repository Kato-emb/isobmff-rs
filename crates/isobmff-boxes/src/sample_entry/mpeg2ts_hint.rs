//! [`Mpeg2TsReceptionSampleEntry`] and [`Mpeg2TsServerSampleEntry`], the sample entries of an MPEG-2 TS hint track, and the fields the MPEG-2 TS classes share

use alloc::vec::Vec;

use isobmff_core::{BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::mpeg2ts_hint::{InitialSampleTimeBox, OdBox, PatBox, PmtBox, TsTimingBox};
use crate::sample_entry::{
    HINT_TRACK_VERSION, SampleEntry, children_len, encode_children, encode_optional, from_class,
    of_format, split_children,
};

/// Length of the fields the classes lay out after `data_reference_index`
const FIELDS_LEN: u64 = 2 + 2 + 1 + 1 + 1;

/// Box types of the boxes ISO/IEC 14496-12 defines for the classes
const DEFINED: [BoxType; 6] = [
    PatBox::BOX_TYPE,
    PmtBox::BOX_TYPE,
    OdBox::BOX_TYPE,
    TsTimingBox::BOX_TYPE,
    InitialSampleTimeBox::BOX_TYPE,
    BitRateBox::BOX_TYPE,
];

/// Fields and boxes every `MPEG2TSSampleEntry` lays out after `data_reference_index`, as [`read_entry`] reads them
pub(super) struct Mpeg2TsEntry {
    pub(super) preceding_bytes_len: u8,
    pub(super) trailing_bytes_len: u8,
    pub(super) precomputed_only_flag: bool,
    pub(super) derived_boxes: OtherBoxes,
    pub(super) tpat: Option<PatBox>,
    pub(super) tpmt: Vec<PmtBox>,
    pub(super) tod: Vec<OdBox>,
    pub(super) tsti: Option<TsTimingBox>,
    pub(super) istm: Option<InitialSampleTimeBox>,
    pub(super) btrt: Option<BitRateBox>,
    pub(super) extra_boxes: OtherBoxes,
}

/// Reads the fields and the boxes of an `MPEG2TSSampleEntry` from the bytes of `entry`, an entry of the format `format`, after `data_reference_index`
///
/// The seven `reserved` bits after `precomputed_only_flag` are read past.
///
/// # Errors
///
/// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
///   `format`: the entry is of another format.
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   bytes end inside a field, counted from the end of
///   `data_reference_index`.
/// * The failures of a child that does not frame, and of a `tPAT`, `tsti`,
///   `istm` or `btrt` given more than once.
pub(super) fn read_entry(entry: &SampleEntry, format: FourCc) -> Result<Mpeg2TsEntry, Error> {
    of_format(entry, format)?;
    let mut reader = FieldReader::new(&entry.remaining_data);
    reader.read_bytes::<4>()?;
    let [preceding_bytes_len, trailing_bytes_len, flags] = *reader.read_bytes::<3>()?;
    let (derived_boxes, mut children) = split_children(reader.take_remainder(), &DEFINED)?;

    Ok(Mpeg2TsEntry {
        preceding_bytes_len,
        trailing_bytes_len,
        precomputed_only_flag: flags & 0x80 != 0,
        derived_boxes,
        tpat: children.take_zero_or_one_if_decoded()?,
        tpmt: children.take_zero_or_more_if_decoded(),
        tod: children.take_zero_or_more_if_decoded(),
        tsti: children.take_zero_or_one_if_decoded()?,
        istm: children.take_zero_or_one_if_decoded()?,
        btrt: children.take_zero_or_one_if_decoded()?,
        extra_boxes: OtherBoxes::from(children),
    })
}

/// Fields and boxes of an `MPEG2TSSampleEntry` to write, borrowed from the class that holds them, as [`Mpeg2TsParts::write`] writes them
pub(super) struct Mpeg2TsParts<'class> {
    pub(super) data_reference_index: u16,
    pub(super) preceding_bytes_len: u8,
    pub(super) trailing_bytes_len: u8,
    pub(super) precomputed_only_flag: bool,
    pub(super) derived_boxes: &'class OtherBoxes,
    pub(super) tpat: Option<&'class PatBox>,
    pub(super) tpmt: &'class [PmtBox],
    pub(super) tod: &'class [OdBox],
    pub(super) tsti: Option<&'class TsTimingBox>,
    pub(super) istm: Option<&'class InitialSampleTimeBox>,
    pub(super) btrt: Option<&'class BitRateBox>,
    pub(super) extra_boxes: &'class OtherBoxes,
}

impl Mpeg2TsParts<'_> {
    /// Writes the fields and the boxes into the bytes of an entry of the format `format` after `data_reference_index`, the `reserved` bits as zero and the boxes of ISO/IEC 14496-12 in the order of §9.3.3.3
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    pub(super) fn write(&self, format: FourCc) -> Result<SampleEntry, Error> {
        let defined = [
            self.tpat.map_or(0, BoxEncode::encoded_len),
            self.tsti.map_or(0, BoxEncode::encoded_len),
            self.istm.map_or(0, BoxEncode::encoded_len),
            self.btrt.map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.tpmt.iter().map(BoxEncode::encoded_len))
        .chain(self.tod.iter().map(BoxEncode::encoded_len));

        from_class(
            format,
            self.data_reference_index,
            children_len(self.derived_boxes, defined, self.extra_boxes).saturating_add(FIELDS_LEN),
            |writer| {
                writer.write_u16(HINT_TRACK_VERSION)?;
                writer.write_u16(HINT_TRACK_VERSION)?;
                writer.write_bytes(&[
                    self.preceding_bytes_len,
                    self.trailing_bytes_len,
                    if self.precomputed_only_flag { 0x80 } else { 0 },
                ])?;
                encode_children(
                    writer.take_remainder(),
                    self.derived_boxes,
                    |mut rest| {
                        rest = encode_optional(rest, self.tpat)?;
                        for table in self.tpmt {
                            rest = table.encode(rest)?;
                        }
                        for table in self.tod {
                            rest = table.encode(rest)?;
                        }
                        rest = encode_optional(rest, self.tsti)?;
                        rest = encode_optional(rest, self.istm)?;
                        encode_optional(rest, self.btrt)
                    },
                    self.extra_boxes,
                )
            },
        )
    }
}

/// [`Mpeg2TsReceptionSampleEntry`] (`rm2t`), ISO/IEC 14496-12 §9.3.3
///
/// The sample entry of an MPEG-2 TS reception hint track, whose samples are
/// the packets of a received MPEG-2 transport stream, with the static
/// metadata of the stream, such as its PSI tables, valid for the packets it
/// describes.
/// The bytes of a [`SampleEntry`] after `data_reference_index` are read with
/// [`TryFrom`] and written back with [`SampleEntry::try_from`].
///
/// `hinttrackversion` and `highestcompatibleversion`, which the Syntax sets
/// to 1, are read past and written as 1.
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// the boxes of §9.3.3.3, `tPAT`, the `tPMT`s, the `tOD `s, `tsti` and
/// `istm`, then `btrt`, which any sample entry may hold; and the boxes no
/// definition of the class names, in [`extra_boxes`](Self::extra_boxes). A
/// child of a type this crate does not read that lies before the first box
/// of ISO/IEC 14496-12 is read into the first part, and one that lies after
/// it into the third.
#[doc(alias = "rm2t", alias = "MPEG2TSReceptionSampleEntry")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Mpeg2TsReceptionSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Number of bytes before each MPEG-2 TS packet, such as a time code of a recording device
    pub preceding_bytes_len: u8,
    /// Number of bytes after each MPEG-2 TS packet, such as a checksum of a recording device
    pub trailing_bytes_len: u8,
    /// Whether the samples are purely precomputed
    pub precomputed_only_flag: bool,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// PAT of the stream
    pub tpat: Option<PatBox>,
    /// PMTs of the programs of the stream, in the order they came
    pub tpmt: Vec<PmtBox>,
    /// Object descriptor tables of the stream, in the order they came
    pub tod: Vec<OdBox>,
    /// How the sample times were derived
    pub tsti: Option<TsTimingBox>,
    /// Time the sample times start from
    pub istm: Option<InitialSampleTimeBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl Mpeg2TsReceptionSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"rm2t");
}

impl TryFrom<&SampleEntry> for Mpeg2TsReceptionSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `rm2t`.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   bytes end inside a field, counted from the end of
    ///   `data_reference_index`.
    /// * The failures of a child that does not frame, and of a `tPAT`,
    ///   `tsti`, `istm` or `btrt` given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let Mpeg2TsEntry {
            preceding_bytes_len,
            trailing_bytes_len,
            precomputed_only_flag,
            derived_boxes,
            tpat,
            tpmt,
            tod,
            tsti,
            istm,
            btrt,
            extra_boxes,
        } = read_entry(entry, Self::FORMAT)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            preceding_bytes_len,
            trailing_bytes_len,
            precomputed_only_flag,
            derived_boxes,
            tpat,
            tpmt,
            tod,
            tsti,
            istm,
            btrt,
            extra_boxes,
        })
    }
}

impl TryFrom<&Mpeg2TsReceptionSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `rm2t` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &Mpeg2TsReceptionSampleEntry) -> Result<Self, Error> {
        Mpeg2TsParts {
            data_reference_index: hint.data_reference_index,
            preceding_bytes_len: hint.preceding_bytes_len,
            trailing_bytes_len: hint.trailing_bytes_len,
            precomputed_only_flag: hint.precomputed_only_flag,
            derived_boxes: &hint.derived_boxes,
            tpat: hint.tpat.as_ref(),
            tpmt: &hint.tpmt,
            tod: &hint.tod,
            tsti: hint.tsti.as_ref(),
            istm: hint.istm.as_ref(),
            btrt: hint.btrt.as_ref(),
            extra_boxes: &hint.extra_boxes,
        }
        .write(Mpeg2TsReceptionSampleEntry::FORMAT)
    }
}

/// [`Mpeg2TsServerSampleEntry`] (`sm2t`), ISO/IEC 14496-12 §9.3.3
///
/// The sample entry of an MPEG-2 TS server hint track, laid out as an
/// [`Mpeg2TsReceptionSampleEntry`] is.
/// The bytes of a [`SampleEntry`] after `data_reference_index` are read with
/// [`TryFrom`] and written back with [`SampleEntry::try_from`].
///
/// `hinttrackversion` and `highestcompatibleversion`, which the Syntax sets
/// to 1, are read past and written as 1.
///
/// # Children
///
/// The children are laid out in three parts as those of an
/// [`Mpeg2TsReceptionSampleEntry`] are.
#[doc(alias = "sm2t", alias = "MPEG2TSServerSampleEntry")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Mpeg2TsServerSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Number of bytes before each MPEG-2 TS packet, such as a time code of a recording device
    pub preceding_bytes_len: u8,
    /// Number of bytes after each MPEG-2 TS packet, such as a checksum of a recording device
    pub trailing_bytes_len: u8,
    /// Whether the samples are purely precomputed
    pub precomputed_only_flag: bool,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// PAT of the stream
    pub tpat: Option<PatBox>,
    /// PMTs of the programs of the stream, in the order they came
    pub tpmt: Vec<PmtBox>,
    /// Object descriptor tables of the stream, in the order they came
    pub tod: Vec<OdBox>,
    /// How the sample times were derived
    pub tsti: Option<TsTimingBox>,
    /// Time the sample times start from
    pub istm: Option<InitialSampleTimeBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl Mpeg2TsServerSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"sm2t");
}

impl TryFrom<&SampleEntry> for Mpeg2TsServerSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `sm2t`.
    /// * The other failures of [`Mpeg2TsReceptionSampleEntry::try_from`].
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let Mpeg2TsEntry {
            preceding_bytes_len,
            trailing_bytes_len,
            precomputed_only_flag,
            derived_boxes,
            tpat,
            tpmt,
            tod,
            tsti,
            istm,
            btrt,
            extra_boxes,
        } = read_entry(entry, Self::FORMAT)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            preceding_bytes_len,
            trailing_bytes_len,
            precomputed_only_flag,
            derived_boxes,
            tpat,
            tpmt,
            tod,
            tsti,
            istm,
            btrt,
            extra_boxes,
        })
    }
}

impl TryFrom<&Mpeg2TsServerSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `sm2t` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &Mpeg2TsServerSampleEntry) -> Result<Self, Error> {
        Mpeg2TsParts {
            data_reference_index: hint.data_reference_index,
            preceding_bytes_len: hint.preceding_bytes_len,
            trailing_bytes_len: hint.trailing_bytes_len,
            precomputed_only_flag: hint.precomputed_only_flag,
            derived_boxes: &hint.derived_boxes,
            tpat: hint.tpat.as_ref(),
            tpmt: &hint.tpmt,
            tod: &hint.tod,
            tsti: hint.tsti.as_ref(),
            istm: hint.istm.as_ref(),
            btrt: hint.btrt.as_ref(),
            extra_boxes: &hint.extra_boxes,
        }
        .write(Mpeg2TsServerSampleEntry::FORMAT)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{Error, FourCc};

    use super::{Mpeg2TsReceptionSampleEntry, Mpeg2TsServerSampleEntry};
    use crate::mpeg2ts_hint::{InitialSampleTimeBox, PatBox, PmtBox};
    use crate::sample_entry::SampleEntry;

    /// Fields of version 1, four bytes before each packet, none after, precomputed
    const FIELDS: &[u8] = b"\0\x01\0\x01\x04\0\x80";

    /// `tPAT` of the PID 0 and one byte of section
    const TPAT: &[u8] = b"\0\0\0\x0btPAT\0\0\0";

    /// `tPMT` of the PID 0x100 and one byte of section
    const TPMT: &[u8] = b"\0\0\0\x0btPMT\x01\0\x02";

    /// `istm` of the initial sample time 90000
    const ISTM: &[u8] = b"\0\0\0\x10istm\0\x01\x5f\x90\0\0\0\0";

    fn sample_entry(format: &[u8; 4], remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*format),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    fn reception() -> Mpeg2TsReceptionSampleEntry {
        Mpeg2TsReceptionSampleEntry {
            data_reference_index: 1,
            preceding_bytes_len: 4,
            precomputed_only_flag: true,
            tpat: Some(PatBox {
                pid: 0,
                section_data: vec![0],
            }),
            tpmt: vec![PmtBox {
                pid: 0x100,
                section_data: vec![2],
            }],
            istm: Some(InitialSampleTimeBox {
                initial_sample_time: 90000,
            }),
            ..Mpeg2TsReceptionSampleEntry::default()
        }
    }

    #[test]
    fn a_reception_entry_writes_its_boxes_in_the_order_of_the_subclause() {
        assert_eq!(
            Mpeg2TsReceptionSampleEntry::try_from(&sample_entry(
                b"rm2t",
                &[FIELDS, ISTM, TPMT, TPAT]
            )),
            Ok(reception())
        );
        assert_eq!(
            SampleEntry::try_from(&reception()),
            Ok(sample_entry(b"rm2t", &[FIELDS, TPAT, TPMT, ISTM]))
        );
    }

    #[test]
    fn a_server_entry_reads_and_writes_back_its_fields() {
        let entry = sample_entry(b"sm2t", &[b"\0\x01\0\x01\0\x04\x7f"]);
        let server = Mpeg2TsServerSampleEntry {
            data_reference_index: 1,
            trailing_bytes_len: 4,
            ..Mpeg2TsServerSampleEntry::default()
        };

        assert_eq!(
            Mpeg2TsServerSampleEntry::try_from(&entry),
            Ok(server.clone())
        );
        assert_eq!(
            SampleEntry::try_from(&server),
            Ok(sample_entry(b"sm2t", &[b"\0\x01\0\x01\0\x04\0"]))
        );
    }

    #[test]
    fn an_entry_of_another_format_is_refused() {
        assert_eq!(
            Mpeg2TsServerSampleEntry::try_from(&sample_entry(b"rm2t", &[FIELDS])),
            Err(Error::unsupported_value("format"))
        );
    }

    #[test]
    fn an_entry_ending_inside_its_fields_is_refused() {
        assert_eq!(
            Mpeg2TsReceptionSampleEntry::try_from(&sample_entry(b"rm2t", &[b"\0\x01\0\x01\x04"])),
            Err(Error::truncated_payload(7, 5))
        );
    }
}
