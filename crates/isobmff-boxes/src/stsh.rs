//! [`ShadowSyncSampleBox`] and the [`ShadowSyncSampleEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{TableEntry, counted_len, read_counted, write_counted};

/// Sample and the sync sample that may stand in for it, one entry of a [`ShadowSyncSampleBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ShadowSyncSampleEntry {
    /// Number of the sample a shadow sync is defined for
    pub shadowed_sample_number: u32,
    /// Number of the sync sample that may be used at, or before, the shadowed sample
    pub sync_sample_number: u32,
}

impl TableEntry for ShadowSyncSampleEntry {
    const ENCODED_LEN: u64 = 8;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            shadowed_sample_number: reader.read_u32()?,
            sync_sample_number: reader.read_u32()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.shadowed_sample_number)?;
        writer.write_u32(self.sync_sample_number)
    }
}

/// [`ShadowSyncSampleBox`] (`stsh`), ISO/IEC 14496-12 §8.6.3.2
///
/// Sync samples that seeking may use in place of the samples they shadow, and
/// which normal forward play ignores.
#[doc(alias = "stsh")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ShadowSyncSampleBox {
    /// Pairs of a shadowed sample and its sync sample, sorted by `shadowed_sample_number`
    pub entries: Vec<ShadowSyncSampleEntry>,
}

impl BoxDefinition for ShadowSyncSampleBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsh");
}

impl BoxDecode for ShadowSyncSampleBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            entries: read_counted(reader)?,
        })
    }
}

impl BoxEncode for ShadowSyncSampleBox {
    fn payload_len(&self) -> u64 {
        counted_len(&self.entries).saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        write_counted(writer, &self.entries)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::{ShadowSyncSampleBox, ShadowSyncSampleEntry};
    use crate::encoded;

    /// `stsh` shadowing the sample 5 with the sync sample 20
    const STSH: &[u8] = b"\0\0\0\x18stsh\0\0\0\0\0\0\0\x01\0\0\0\x05\0\0\0\x14";

    fn shadow_sync_sample() -> ShadowSyncSampleBox {
        ShadowSyncSampleBox {
            entries: vec![ShadowSyncSampleEntry {
                shadowed_sample_number: 5,
                sync_sample_number: 20,
            }],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ShadowSyncSampleBox::decode(STSH),
            Ok((shadow_sync_sample(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&shadow_sync_sample()), STSH);
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_refused() {
        assert_eq!(
            ShadowSyncSampleBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0\0\x05\0\0"),
            Err(Error::truncated_payload(16, 14))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            ShadowSyncSampleBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
