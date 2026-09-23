//! Framing of the NAL units of an AVC sample, ISO/IEC 14496-15 §4.3.3

use alloc::vec::Vec;
use core::mem;

use crate::{Error, LengthSizeMinusOne};

impl LengthSizeMinusOne {
    /// Returns the sample that carries `nal_units`, each behind a
    /// `NALUnitLength` field of the length this value states
    ///
    /// ISO/IEC 14496-15 §4.3.3 lays a sample out as its NAL units one after
    /// another, each preceded by its length in `length_size_minus_one() + 1`
    /// bytes, big-endian. The NAL units are copied as they are; nothing of
    /// ISO/IEC 14496-10 is read.
    ///
    /// # Errors
    ///
    /// * [`NALUnitLengthOutOfRange`](crate::ErrorKind::NALUnitLengthOutOfRange):
    ///   a NAL unit is longer than the `NALUnitLength` field can state.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_avc::LengthSizeMinusOne;
    ///
    /// // An SPS and a PPS as an encoder emits them, header byte first
    /// let sequence_parameter_set = [0x67, 0x42, 0xc0, 0x1e, 0xd9];
    /// let picture_parameter_set = [0x68, 0xce, 0x3c, 0x80];
    ///
    /// // Each goes behind its length in four bytes
    /// let sample = LengthSizeMinusOne::FOUR_BYTES
    ///     .frame([sequence_parameter_set.as_slice(), &picture_parameter_set])
    ///     .unwrap();
    ///
    /// assert_eq!(
    ///     sample,
    ///     [0, 0, 0, 5, 0x67, 0x42, 0xc0, 0x1e, 0xd9, 0, 0, 0, 4, 0x68, 0xce, 0x3c, 0x80]
    /// );
    /// ```
    pub fn frame(self, nal_units: impl IntoIterator<Item: AsRef<[u8]>>) -> Result<Vec<u8>, Error> {
        let length_size = self.length_size();
        let mut sample = Vec::new();
        for nal_unit in nal_units {
            let nal_unit = nal_unit.as_ref();
            let length = nal_unit.len() as u64;
            let length_bytes = length.to_be_bytes();
            let (overflow, field) =
                length_bytes.split_at(length_bytes.len().saturating_sub(length_size));
            if overflow.iter().any(|&byte| byte != 0) {
                return Err(Error::nal_unit_length_out_of_range(
                    length,
                    length_size as u64,
                ));
            }
            sample.extend_from_slice(field);
            sample.extend_from_slice(nal_unit);
        }

        Ok(sample)
    }

    /// Returns the NAL units of `sample` in turn, each read behind its
    /// `NALUnitLength` field of the length this value states
    ///
    /// The layout is the one [`frame`](Self::frame) writes. A NAL unit comes
    /// out as a slice of `sample`, as it is; nothing of ISO/IEC 14496-10 is
    /// read. After an `Err` the iterator ends.
    ///
    /// # Errors
    ///
    /// * [`TruncatedSample`](crate::ErrorKind::TruncatedSample): `sample` ends
    ///   inside a `NALUnitLength` field or inside the NAL unit it measures.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_avc::LengthSizeMinusOne;
    ///
    /// // A sample of two NAL units behind two-byte lengths
    /// let length_size = LengthSizeMinusOne::new(1).unwrap();
    /// let sample = length_size.frame([[0x65, 0x88].as_slice(), &[0x06]]).unwrap();
    ///
    /// // Reading it gives the NAL units back
    /// let nal_units = length_size
    ///     .nal_units(&sample)
    ///     .collect::<Result<Vec<_>, _>>()
    ///     .unwrap();
    ///
    /// assert_eq!(nal_units, [[0x65, 0x88].as_slice(), &[0x06]]);
    /// ```
    pub fn nal_units(self, sample: &[u8]) -> impl Iterator<Item = Result<&[u8], Error>> {
        NALUnits {
            remaining: sample,
            length_size: self.length_size(),
        }
    }

    /// Returns the length in bytes of the `NALUnitLength` field
    fn length_size(self) -> usize {
        usize::from(self.length_size_minus_one()).saturating_add(1)
    }
}

/// The NAL units of a sample read one after another
struct NALUnits<'sample> {
    /// Bytes of the sample still to be read, empty once an `Err` came out
    remaining: &'sample [u8],
    /// Length in bytes of the `NALUnitLength` field
    length_size: usize,
}

impl<'sample> Iterator for NALUnits<'sample> {
    type Item = Result<&'sample [u8], Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let remaining = mem::take(&mut self.remaining);
        if remaining.is_empty() {
            return None;
        }

        let Some((field, rest)) = remaining.split_at_checked(self.length_size) else {
            return Some(Err(Error::truncated_sample(
                self.length_size as u64,
                remaining.len() as u64,
            )));
        };
        let length = field.iter().fold(0_u64, |length, &byte| {
            length.wrapping_shl(8) | u64::from(byte)
        });
        let Some((nal_unit, rest)) = usize::try_from(length)
            .ok()
            .and_then(|length| rest.split_at_checked(length))
        else {
            return Some(Err(Error::truncated_sample(length, rest.len() as u64)));
        };
        self.remaining = rest;

        Some(Ok(nal_unit))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::FieldReader;

    use crate::{AVCDecoderConfigurationRecord, Error, LengthSizeMinusOne};

    #[test]
    fn a_nal_unit_longer_than_the_length_field_can_state_is_refused() {
        let one_byte = LengthSizeMinusOne::new(0).unwrap();

        assert_eq!(
            one_byte.frame([vec![0; 256]]),
            Err(Error::nal_unit_length_out_of_range(256, 1))
        );
    }

    #[test]
    fn a_length_size_the_spec_forbids_frames_behind_three_bytes() {
        let record = AVCDecoderConfigurationRecord::decode_fields(&mut FieldReader::new(
            b"\x01\x42\xc0\x1e\xfe\xe0\0",
        ))
        .unwrap();

        assert_eq!(
            record.length_size_minus_one().frame([[0x65, 0x88]]),
            Ok(vec![0, 0, 2, 0x65, 0x88])
        );
    }

    #[test]
    fn a_framed_sample_reads_back_as_the_nal_units_it_was_framed_from() {
        let nal_units = [
            vec![0x67, 0x42, 0xc0, 0x1e],
            vec![0x68, 0xce],
            vec![0x65; 300],
        ];
        let sample = LengthSizeMinusOne::FOUR_BYTES.frame(&nal_units).unwrap();

        assert_eq!(
            LengthSizeMinusOne::FOUR_BYTES
                .nal_units(&sample)
                .collect::<Result<Vec<_>, _>>(),
            Ok(nal_units.iter().map(Vec::as_slice).collect())
        );
    }

    #[test]
    fn a_nal_unit_cut_short_ends_the_sample_with_an_error() {
        let sample = b"\0\x01\x65\0\x0a\x41\x9a\x02";

        assert_eq!(
            LengthSizeMinusOne::new(1)
                .unwrap()
                .nal_units(sample)
                .collect::<Vec<_>>(),
            [Ok([0x65].as_slice()), Err(Error::truncated_sample(10, 3))]
        );
    }

    #[test]
    fn a_length_field_cut_short_ends_the_sample_with_an_error() {
        let sample = b"\0\0\0\x01\x65\0\0";

        assert_eq!(
            LengthSizeMinusOne::FOUR_BYTES
                .nal_units(sample)
                .collect::<Vec<_>>(),
            [Ok([0x65].as_slice()), Err(Error::truncated_sample(4, 2))]
        );
    }

    #[test]
    fn an_empty_nal_unit_frames_and_reads_back() {
        let sample = LengthSizeMinusOne::FOUR_BYTES.frame([[0_u8; 0]]).unwrap();

        assert_eq!(sample, [0, 0, 0, 0]);
        assert_eq!(
            LengthSizeMinusOne::FOUR_BYTES
                .nal_units(&sample)
                .collect::<Vec<_>>(),
            [Ok([].as_slice())]
        );
    }
}
