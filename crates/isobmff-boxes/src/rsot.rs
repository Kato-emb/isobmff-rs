//! [`RedundantSampleOriginalTimingBox`], the original timing of the first and last samples of a track fragment

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::read_version_zero;

/// Flag mask of `rsot_original_duration`
const RSOT_ORIGINAL_DURATION: u8 = 0x01;

/// Flag mask of `rsot_elapsed_duration`
const RSOT_ELAPSED_DURATION: u8 = 0x02;

/// [`RedundantSampleOriginalTimingBox`] (`rsot`), ISO/IEC 14496-12 §8.8.18
///
/// Documents that the last sample of a track fragment was cut short of the
/// duration it was authored with, or that its first sample is a copy of the
/// previous sample whose original start time was before its decoding time.
///
/// A flag that says a field is there is whether the field is [`Some`]; the
/// other flags are read past and written as zero. An `original_duration` of
/// 0, which the specification does not allow, reads, but writing it is
/// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `original_duration`.
#[doc(alias = "rsot")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct RedundantSampleOriginalTimingBox {
    /// Original duration of the last sample of the track fragment, in the
    /// timescale of its media, the `rsot_original_duration` flag
    ///
    /// It is no less than the duration of that sample in the track fragment.
    pub original_duration: Option<u32>,
    /// Duration of the first sample of the track fragment already elapsed,
    /// for a reader that tunes in at it, the `rsot_elapsed_duration` flag
    pub elapsed_duration: Option<u32>,
}

impl BoxDefinition for RedundantSampleOriginalTimingBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"rsot");
}

impl BoxDecode for RedundantSampleOriginalTimingBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let [.., flags] = read_version_zero(reader)?.bits().to_be_bytes();
        let original_duration = if flags & RSOT_ORIGINAL_DURATION == 0 {
            None
        } else {
            Some(reader.read_u32()?)
        };
        let elapsed_duration = if flags & RSOT_ELAPSED_DURATION == 0 {
            None
        } else {
            Some(reader.read_u32()?)
        };

        Ok(Self {
            original_duration,
            elapsed_duration,
        })
    }
}

impl BoxEncode for RedundantSampleOriginalTimingBox {
    fn payload_len(&self) -> u64 {
        [self.original_duration, self.elapsed_duration]
            .into_iter()
            .flatten()
            .fold(4, |len, _| len.saturating_add(4))
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.original_duration == Some(0) {
            return Err(Error::invalid_value("original_duration"));
        }

        let flags = [
            (self.original_duration.is_some(), RSOT_ORIGINAL_DURATION),
            (self.elapsed_duration.is_some(), RSOT_ELAPSED_DURATION),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |flags, (_, mask)| flags | mask);
        writer.write_bytes(&[0, 0, 0, flags])?;
        if let Some(original_duration) = self.original_duration {
            writer.write_u32(original_duration)?;
        }
        if let Some(elapsed_duration) = self.elapsed_duration {
            writer.write_u32(elapsed_duration)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::RedundantSampleOriginalTimingBox;
    use crate::encoded;

    /// `rsot` of both flags: an original duration of 3000 and an elapsed duration of 1000
    const RSOT_BOTH: &[u8] = b"\0\0\0\x14rsot\0\0\0\x03\0\0\x0b\xb8\0\0\x03\xe8";

    /// `rsot` of the elapsed duration alone, 1000
    const RSOT_ELAPSED: &[u8] = b"\0\0\0\x10rsot\0\0\0\x02\0\0\x03\xe8";

    fn timing(
        original_duration: Option<u32>,
        elapsed_duration: Option<u32>,
    ) -> RedundantSampleOriginalTimingBox {
        RedundantSampleOriginalTimingBox {
            original_duration,
            elapsed_duration,
        }
    }

    #[test]
    fn reads_the_fields_its_flags_say_are_there() {
        assert_eq!(
            (
                RedundantSampleOriginalTimingBox::decode(RSOT_BOTH),
                RedundantSampleOriginalTimingBox::decode(RSOT_ELAPSED)
            ),
            (
                Ok((timing(Some(3000), Some(1000)), b"".as_slice())),
                Ok((timing(None, Some(1000)), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_flags_of_the_fields_that_are_there() {
        assert_eq!(
            (
                encoded(&timing(Some(3000), Some(1000))),
                encoded(&timing(None, Some(1000)))
            ),
            (RSOT_BOTH.to_vec(), RSOT_ELAPSED.to_vec())
        );
    }

    #[test]
    fn an_original_duration_of_0_is_refused_when_written() {
        assert_eq!(
            timing(Some(0), None).encode(&mut [0; 16]),
            Err(Error::invalid_value("original_duration"))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_its_flags_say_is_there_is_refused() {
        assert_eq!(
            RedundantSampleOriginalTimingBox::decode_payload(b"\0\0\0\x03\0\0\x0b\xb8\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            RedundantSampleOriginalTimingBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
