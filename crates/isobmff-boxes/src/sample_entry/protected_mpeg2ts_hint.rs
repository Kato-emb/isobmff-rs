//! [`ProtectedMpeg2TransportStreamSampleEntry`], the sample entry of an MPEG-2 TS hint track whose packets carry protected media

use alloc::vec::Vec;

use isobmff_core::{Error, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::mpeg2ts_hint::{InitialSampleTimeBox, OdBox, PatBox, PmtBox, TsTimingBox};
use crate::rinf::RestrictedSchemeInfoBox;
use crate::sample_entry::mpeg2ts_hint::{Mpeg2TsEntry, Mpeg2TsParts, read_entry};
use crate::sample_entry::{AnyEntryBoxes, SampleEntry};
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;

/// [`ProtectedMpeg2TransportStreamSampleEntry`] (`pm2t`), ISO/IEC 14496-12 §9.3.5
///
/// The sample entry of an MPEG-2 TS hint track whose packets carry protected
/// media, laid out as an
/// [`Mpeg2TsReceptionSampleEntry`](crate::Mpeg2TsReceptionSampleEntry) is,
/// with the `sinf` that names the protection scheme and the original format.
/// The track itself is in the clear. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// `hinttrackversion` and `highestcompatibleversion`, which the Syntax sets
/// to 1, are read past and written as 1.
///
/// # Children
///
/// The children are laid out in three parts as those of an
/// [`Mpeg2TsReceptionSampleEntry`](crate::Mpeg2TsReceptionSampleEntry) are.
/// The `sinf` it requires is among the boxes any sample entry may hold.
#[doc(alias = "pm2t", alias = "ProtectedMPEG2TransportStreamSampleEntry")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ProtectedMpeg2TransportStreamSampleEntry {
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

impl ProtectedMpeg2TransportStreamSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"pm2t");
}

impl TryFrom<&SampleEntry> for ProtectedMpeg2TransportStreamSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `pm2t`.
    /// * The other failures of
    ///   [`Mpeg2TsReceptionSampleEntry::try_from`](crate::Mpeg2TsReceptionSampleEntry::try_from).
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
            sinf,
            scrb,
            rinf,
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
            sinf,
            scrb,
            rinf,
            btrt,
            extra_boxes,
        })
    }
}

impl TryFrom<&ProtectedMpeg2TransportStreamSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `hint` into the bytes of an entry of the format `pm2t` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(hint: &ProtectedMpeg2TransportStreamSampleEntry) -> Result<Self, Error> {
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
            any_entry: AnyEntryBoxes {
                sinf: &hint.sinf,
                scrb: &hint.scrb,
                rinf: hint.rinf.as_ref(),
                btrt: hint.btrt.as_ref(),
            },
            extra_boxes: &hint.extra_boxes,
        }
        .write(ProtectedMpeg2TransportStreamSampleEntry::FORMAT)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, FourCc};

    use super::ProtectedMpeg2TransportStreamSampleEntry;
    use crate::mpeg2ts_hint::PatBox;
    use crate::sample_entry::SampleEntry;
    use crate::sinf::ProtectionSchemeInfoBox;

    /// Fields of version 1, four bytes before each packet, none after, precomputed
    const FIELDS: &[u8] = b"\0\x01\0\x01\x04\0\x80";

    /// `tPAT` of the PID 0 and one byte of section
    const TPAT: &[u8] = b"\0\0\0\x0btPAT\0\0\0";

    /// `sinf` of the original format `rm2t`
    const SINF: &[u8] = b"\0\0\0\x14sinf\0\0\0\x0cfrmarm2t";

    fn sample_entry(remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"pm2t"),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    #[test]
    fn a_protected_entry_writes_its_protection_scheme_after_the_boxes_of_its_class() {
        let entry = sample_entry(&[FIELDS, SINF, TPAT]);
        let protected = ProtectedMpeg2TransportStreamSampleEntry {
            data_reference_index: 1,
            preceding_bytes_len: 4,
            precomputed_only_flag: true,
            tpat: Some(PatBox {
                pid: 0,
                section_data: vec![0],
            }),
            sinf: vec![ProtectionSchemeInfoBox::decode(SINF).unwrap().0],
            ..ProtectedMpeg2TransportStreamSampleEntry::default()
        };

        assert_eq!(
            (
                ProtectedMpeg2TransportStreamSampleEntry::try_from(&entry),
                SampleEntry::try_from(&protected)
            ),
            (
                Ok(protected.clone()),
                Ok(sample_entry(&[FIELDS, TPAT, SINF]))
            )
        );
    }
}
