//! [`SampleSizeBox`] and the [`SampleSizes`] it states

use alloc::vec::Vec;
use core::num::NonZeroU32;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{counted_len, read_counted, write_counted};

/// Sizes of the samples of a track, as a [`SampleSizeBox`] states them
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SampleSizes {
    /// Every sample is `sample_size` bytes, and no table follows
    Constant {
        /// Size of every sample, in bytes
        sample_size: NonZeroU32,
        /// Number of samples in the track
        sample_count: u32,
    },
    /// Samples have different sizes, which the sample size table states
    Table {
        /// `entry_size` of every sample, in bytes, indexed by its number
        entry_sizes: Vec<u32>,
    },
}

impl Default for SampleSizes {
    fn default() -> Self {
        Self::Table {
            entry_sizes: Vec::new(),
        }
    }
}

/// [`SampleSizeBox`] (`stsz`), ISO/IEC 14496-12 §8.7.3.2
///
/// The number of samples of the track and the size of each, in 32 bits: one
/// size for all of them, or a table of a size per sample.
///
/// [`Default`] is the box of no sample.
#[doc(alias = "stsz")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleSizeBox {
    /// Sizes of the samples
    pub sample_sizes: SampleSizes,
}

impl BoxDefinition for SampleSizeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsz");
}

impl BoxDecode for SampleSizeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let sample_sizes = match NonZeroU32::new(reader.read_u32()?) {
            Some(sample_size) => SampleSizes::Constant {
                sample_size,
                sample_count: reader.read_u32()?,
            },
            None => SampleSizes::Table {
                entry_sizes: read_counted(reader)?,
            },
        };

        Ok(Self { sample_sizes })
    }
}

impl BoxEncode for SampleSizeBox {
    fn payload_len(&self) -> u64 {
        match &self.sample_sizes {
            SampleSizes::Constant { .. } => 4 + 4 + 4,
            SampleSizes::Table { entry_sizes } => counted_len(entry_sizes).saturating_add(4 + 4),
        }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        match &self.sample_sizes {
            SampleSizes::Constant {
                sample_size,
                sample_count,
            } => {
                writer.write_u32(sample_size.get())?;
                writer.write_u32(*sample_count)
            }
            SampleSizes::Table { entry_sizes } => {
                writer.write_u32(0)?;
                write_counted(writer, entry_sizes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use isobmff_core::{BoxDecode, Error};

    use super::{SampleSizeBox, SampleSizes};
    use crate::encoded;

    /// `stsz` of 1000 samples of 512 bytes each
    const STSZ_CONSTANT: &[u8] = b"\0\0\0\x14stsz\0\0\0\0\0\0\x02\0\0\0\x03\xe8";

    /// `stsz` of a table of three samples, of 300, 20 and `0x1_0000` bytes
    const STSZ_TABLE: &[u8] =
        b"\0\0\0\x20stsz\0\0\0\0\0\0\0\0\0\0\0\x03\0\0\x01\x2c\0\0\0\x14\0\x01\0\0";

    fn constant() -> SampleSizeBox {
        SampleSizeBox {
            sample_sizes: SampleSizes::Constant {
                sample_size: NonZeroU32::new(512).unwrap(),
                sample_count: 1000,
            },
        }
    }

    fn table() -> SampleSizeBox {
        SampleSizeBox {
            sample_sizes: SampleSizes::Table {
                entry_sizes: vec![300, 20, 0x1_0000],
            },
        }
    }

    #[test]
    fn a_sample_size_other_than_0_reads_as_the_size_of_every_sample() {
        assert_eq!(
            SampleSizeBox::decode(STSZ_CONSTANT),
            Ok((constant(), b"".as_slice()))
        );
    }

    #[test]
    fn a_sample_size_of_0_reads_the_table_of_a_size_per_sample() {
        assert_eq!(
            SampleSizeBox::decode(STSZ_TABLE),
            Ok((table(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_for_either_layout() {
        assert_eq!(
            (encoded(&constant()), encoded(&table())),
            (STSZ_CONSTANT.to_vec(), STSZ_TABLE.to_vec())
        );
    }

    #[test]
    fn a_sample_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            SampleSizeBox::decode_payload(b"\0\0\0\0\0\0\0\0\0\0\0\x03\0\0\x01\x2c"),
            Err(Error::truncated_payload(24, 16))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SampleSizeBox::decode_payload(b"\x01\0\0\0\0\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
