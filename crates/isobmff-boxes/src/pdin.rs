//! [`ProgressiveDownloadInfoBox`] and the [`ProgressiveDownloadInfoEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::table::{TableEntry, entries_len, read_to_end, write_entries};

/// Pair of a download rate and the initial delay it suggests, one entry of a [`ProgressiveDownloadInfoBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct ProgressiveDownloadInfoEntry {
    /// Download rate, in bytes per second
    pub rate: u32,
    /// Delay to play the file after, in milliseconds, such that a download at
    /// `rate` brings all the data in time for its use
    pub initial_delay: u32,
}

impl TableEntry for ProgressiveDownloadInfoEntry {
    const ENCODED_LEN: u64 = 8;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            rate: reader.read_u32()?,
            initial_delay: reader.read_u32()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.rate)?;
        writer.write_u32(self.initial_delay)
    }
}

/// [`ProgressiveDownloadInfoBox`] (`pdin`), ISO/IEC 14496-12 §8.1.3
///
/// Pairs of a download rate and the initial delay it suggests, which a
/// receiver interpolates between, or extrapolates from, for the rate it
/// experiences.
#[doc(alias = "pdin")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct ProgressiveDownloadInfoBox {
    /// Pairs of a rate and a delay, in the order they came
    pub entries: Vec<ProgressiveDownloadInfoEntry>,
}

impl BoxDefinition for ProgressiveDownloadInfoBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"pdin");
}

impl BoxDecode for ProgressiveDownloadInfoBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        Ok(Self {
            entries: read_to_end(reader)?,
        })
    }
}

impl BoxEncode for ProgressiveDownloadInfoBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.entries).saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(0, FullBoxFlags::ZERO).to_bytes())?;
        write_entries(writer, &self.entries)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::{ProgressiveDownloadInfoBox, ProgressiveDownloadInfoEntry};
    use crate::encoded;

    /// `pdin` of two pairs: 1000 bytes per second after 5000 ms, 4000 after 0 ms
    const PDIN: &[u8] = b"\0\0\0\x1cpdin\0\0\0\0\0\0\x03\xe8\0\0\x13\x88\0\0\x0f\xa0\0\0\0\0";

    fn progressive_download_info() -> ProgressiveDownloadInfoBox {
        ProgressiveDownloadInfoBox {
            entries: vec![
                ProgressiveDownloadInfoEntry {
                    rate: 1000,
                    initial_delay: 5000,
                },
                ProgressiveDownloadInfoEntry {
                    rate: 4000,
                    initial_delay: 0,
                },
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            ProgressiveDownloadInfoBox::decode(PDIN),
            Ok((progressive_download_info(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&progressive_download_info()), PDIN);
    }

    #[test]
    fn flags_are_read_on_past_and_written_as_zero() {
        let (read, _) = ProgressiveDownloadInfoBox::decode(b"\0\0\0\x0cpdin\0\0\0\x01").unwrap();

        assert_eq!(encoded(&read), b"\0\0\0\x0cpdin\0\0\0\0");
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_refused() {
        assert_eq!(
            ProgressiveDownloadInfoBox::decode_payload(b"\0\0\0\0\0\0\x03\xe8\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            ProgressiveDownloadInfoBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
