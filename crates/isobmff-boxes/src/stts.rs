//! [`TimeToSampleBox`] and the [`TimeToSampleEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{TableEntry, counted_len, read_counted, write_counted};

/// Run of consecutive samples of one duration, one entry of a [`TimeToSampleBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TimeToSampleEntry {
    /// Number of consecutive samples that have the duration `sample_delta`
    pub sample_count: u32,
    /// Difference between the decoding timestamp of the next sample and that
    /// of each of these, in the timescale of the media
    pub sample_delta: u32,
}

impl TableEntry for TimeToSampleEntry {
    const ENCODED_LEN: u64 = 8;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            sample_count: reader.read_u32()?,
            sample_delta: reader.read_u32()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.sample_count)?;
        writer.write_u32(self.sample_delta)
    }
}

/// [`TimeToSampleBox`] (`stts`), ISO/IEC 14496-12 §8.6.1.2
///
/// The durations of the samples in decoding order, as runs of samples of one
/// duration: the decoding timestamp of a sample is the sum of the
/// `sample_delta`s before it, from 0.
///
/// Where no sample follows the last one in decoding order, its
/// `sample_delta` "should be set equal to duration subtracted by the
/// composition timestamp of the last sample in composition time order" when
/// that sample is also last in composition time order and the `duration` of
/// the `MediaHeaderBox` is known; otherwise its value is unspecified.
#[doc(alias = "stts")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TimeToSampleBox {
    /// Runs of samples, in decoding order
    pub entries: Vec<TimeToSampleEntry>,
}

impl BoxDefinition for TimeToSampleBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stts");
}

impl BoxDecode for TimeToSampleBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            entries: read_counted(reader)?,
        })
    }
}

impl BoxEncode for TimeToSampleBox {
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

    use super::{TimeToSampleBox, TimeToSampleEntry};
    use crate::encoded;

    /// `stts` of 14 samples of duration 10 and then one of duration 0
    const STTS: &[u8] = b"\0\0\0\x20stts\0\0\0\0\0\0\0\x02\0\0\0\x0e\0\0\0\x0a\0\0\0\x01\0\0\0\0";

    fn time_to_sample() -> TimeToSampleBox {
        TimeToSampleBox {
            entries: vec![
                TimeToSampleEntry {
                    sample_count: 14,
                    sample_delta: 10,
                },
                TimeToSampleEntry {
                    sample_count: 1,
                    sample_delta: 0,
                },
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            TimeToSampleBox::decode(STTS),
            Ok((time_to_sample(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&time_to_sample()), STTS);
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            TimeToSampleBox::decode_payload(b"\0\0\0\0\0\0\0\x02\0\0\0\x0e\0\0\0\x0a"),
            Err(Error::truncated_payload(24, 16))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            TimeToSampleBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
