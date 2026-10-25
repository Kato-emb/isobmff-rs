//! [`SyncSampleBox`]

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{counted_len, read_counted, write_counted};

/// [`SyncSampleBox`] (`stss`), ISO/IEC 14496-12 §8.6.2
///
/// The sync samples of the track, by sample number. Where a track has no such
/// box, every sample is a sync sample; a box of no entry says there is none.
/// Not every sync sample need be marked here, but every sample marked is one.
#[doc(alias = "stss")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SyncSampleBox {
    /// `sample_number` of every sync sample marked, in strictly increasing order
    pub sample_numbers: Vec<u32>,
}

impl BoxDefinition for SyncSampleBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stss");
}

impl BoxDecode for SyncSampleBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            sample_numbers: read_counted(reader)?,
        })
    }
}

impl BoxEncode for SyncSampleBox {
    fn payload_len(&self) -> u64 {
        counted_len(&self.sample_numbers).saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        write_counted(writer, &self.sample_numbers)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::SyncSampleBox;
    use crate::encoded;

    /// `stss` marking the samples 1 and 8
    const STSS: &[u8] = b"\0\0\0\x18stss\0\0\0\0\0\0\0\x02\0\0\0\x01\0\0\0\x08";

    fn sync_sample() -> SyncSampleBox {
        SyncSampleBox {
            sample_numbers: vec![1, 8],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SyncSampleBox::decode(STSS),
            Ok((sync_sample(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&sync_sample()), STSS);
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            SyncSampleBox::decode_payload(b"\0\0\0\0\0\0\0\x02\0\0\0\x01"),
            Err(Error::truncated_payload(16, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SyncSampleBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
