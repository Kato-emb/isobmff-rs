//! [`SubSampleReferenceTableBox`] and the [`SubSampleDescription`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader,
};

use crate::full_box::read_version_zero;
use crate::table::read_entries;

/// Properties of a sub-sample, stated by an entry of a [`SubSampleReferenceTableBox`] or by a sub-sample of version 2 of a [`SubSampleInformationBox`](crate::SubSampleInformationBox)
///
/// `csp_length_minus_1` is kept whether `codec_specific_parameters` is
/// there or not, as it was read. A `csp_length_minus_1` above 3, or a
/// `codec_specific_parameters` wider than the bytes it states, is refused when
/// the description is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field.
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SubSampleDescription {
    /// Whether the sub-sample is not required to decode the sample, but may be
    /// used for enhancements
    pub discardable: bool,
    /// Degradation priority of the sub-sample, higher for one of greater
    /// impact on the decoded quality; `None` when it is not present, and so 0
    pub subsample_priority: Option<u8>,
    /// Number of bytes, minus 1, of `codec_specific_parameters`, from 0 to 3
    pub csp_length_minus_1: u8,
    /// Parameters the codec in use defines; `None` when they are not present,
    /// and so 0
    pub codec_specific_parameters: Option<u32>,
}

impl SubSampleDescription {
    /// Reads the fields after the bits of `flags` that open the description
    ///
    /// `flags` is the byte that opens an entry of a
    /// [`SubSampleReferenceTableBox`]: `discardable`,
    /// `has_subsample_priority`, `is_csp_present` and `csp_length_minus_1`, in
    /// its five high bits. A sub-sample of version 2 of a
    /// [`SubSampleInformationBox`](crate::SubSampleInformationBox) holds them
    /// one bit lower, after `has_reference`, and is shifted into this layout.
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a field.
    pub(crate) fn read(flags: u8, reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let csp_length_minus_1 = flags >> 3 & 0b11;
        let subsample_priority = if flags & 0x40 == 0 {
            None
        } else {
            Some(u8::from_be_bytes(*reader.read_bytes::<1>()?))
        };
        let codec_specific_parameters = if flags & 0x20 == 0 {
            None
        } else {
            let bytes = reader.read_slice(usize::from(csp_length_minus_1).saturating_add(1))?;
            Some(
                bytes
                    .iter()
                    .fold(0, |value, byte| value << 8 | u32::from(*byte)),
            )
        };

        Ok(Self {
            discardable: flags & 0x80 != 0,
            subsample_priority,
            csp_length_minus_1,
            codec_specific_parameters,
        })
    }

    /// Returns the byte that opens the description in a [`SubSampleReferenceTableBox`], its reserved bits zero
    ///
    /// # Errors
    ///
    /// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue): a field
    ///   holds a value the description cannot be written with.
    pub(crate) fn flags(&self) -> Result<u8, Error> {
        if self.csp_length_minus_1 > 0b11 {
            return Err(Error::invalid_value("csp_length_minus_1"));
        }
        let width = u32::from(self.csp_length_minus_1)
            .saturating_add(1)
            .saturating_mul(8);
        if self
            .codec_specific_parameters
            .is_some_and(|parameters| parameters.checked_shr(width).is_some_and(|rest| rest != 0))
        {
            return Err(Error::invalid_value("codec_specific_parameters"));
        }

        Ok(u8::from(self.discardable) << 7
            | u8::from(self.subsample_priority.is_some()) << 6
            | u8::from(self.codec_specific_parameters.is_some()) << 5
            | self.csp_length_minus_1 << 3)
    }

    /// Returns how many bytes the description takes, its opening byte included
    pub(crate) fn encoded_len(&self) -> u64 {
        let priority_len = u64::from(self.subsample_priority.is_some());
        let parameters_len = self
            .codec_specific_parameters
            .map_or(0, |_| u64::from(self.csp_length_minus_1).saturating_add(1));

        priority_len
            .saturating_add(parameters_len)
            .saturating_add(1)
    }

    /// Writes the fields after the byte that opens the description
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside a field.
    pub(crate) fn write_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if let Some(priority) = self.subsample_priority {
            writer.write_bytes(&[priority])?;
        }
        if let Some(parameters) = self.codec_specific_parameters {
            let bytes = parameters.to_be_bytes();
            let start = usize::from(3_u8.saturating_sub(self.csp_length_minus_1));
            writer.write_slice(bytes.get(start..).unwrap_or_default())?;
        }

        Ok(())
    }
}

/// [`SubSampleReferenceTableBox`] (`ssrt`), ISO/IEC 14496-12 §8.7.10
///
/// A table of the properties of sub-samples that are likely to occur several
/// times, which version 2 of a
/// [`SubSampleInformationBox`](crate::SubSampleInformationBox) of the same
/// `flags` refers to by index.
///
/// More entries than `is_short_ref` lets the box count, 128 or 32 768, are
/// refused when the box is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `entry_count`.
#[doc(alias = "ssrt")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SubSampleReferenceTableBox {
    /// Flags of the box, which tell the tables of one container apart and
    /// match a table to the boxes that refer to it
    pub flags: FullBoxFlags,
    /// Whether `entry_count` is 7 bits wide rather than 15
    pub is_short_ref: bool,
    /// Properties of sub-samples, in the order their indices count from 0
    pub entries: Vec<SubSampleDescription>,
}

impl Default for SubSampleReferenceTableBox {
    fn default() -> Self {
        Self {
            flags: FullBoxFlags::ZERO,
            is_short_ref: false,
            entries: Vec::new(),
        }
    }
}

impl BoxDefinition for SubSampleReferenceTableBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ssrt");
}

impl BoxDecode for SubSampleReferenceTableBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let flags = read_version_zero(reader)?;
        let [first] = *reader.read_bytes::<1>()?;
        let is_short_ref = first & 0x80 != 0;
        let entry_count = if is_short_ref {
            u16::from(first & 0x7f)
        } else {
            let [second] = *reader.read_bytes::<1>()?;
            u16::from_be_bytes([first & 0x7f, second])
        };
        let entries = read_entries(reader, u32::from(entry_count), 1, |reader| {
            let [flags] = *reader.read_bytes::<1>()?;
            SubSampleDescription::read(flags, reader)
        })?;

        Ok(Self {
            flags,
            is_short_ref,
            entries,
        })
    }
}

impl BoxEncode for SubSampleReferenceTableBox {
    fn payload_len(&self) -> u64 {
        let fields_len = if self.is_short_ref { 4 + 1 } else { 4 + 2 };

        self.entries
            .iter()
            .map(SubSampleDescription::encoded_len)
            .fold(fields_len, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let maximum = if self.is_short_ref { 0x7f } else { 0x7fff };
        let entry_count = u16::try_from(self.entries.len())
            .ok()
            .filter(|entry_count| *entry_count <= maximum)
            .ok_or(Error::invalid_value("entry_count"))?;

        writer.write_bytes(&FullBoxHeader::new(0, self.flags).to_bytes())?;
        if self.is_short_ref {
            let [_, low] = entry_count.to_be_bytes();
            writer.write_bytes(&[0x80 | low])?;
        } else {
            writer.write_u16(entry_count)?;
        }
        for entry in &self.entries {
            writer.write_bytes(&[entry.flags()?])?;
            entry.write_fields(writer)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FullBoxFlags};

    use super::{SubSampleDescription, SubSampleReferenceTableBox};
    use crate::encoded;

    /// `ssrt` of flags 1 and a 7-bit count: a discardable sub-sample of no
    /// priority, then one of priority 9 and 3 bytes of parameters `0x01_0203`
    const SSRT_SHORT: &[u8] = b"\0\0\0\x13ssrt\0\0\0\x01\x82\x80\x70\x09\x01\x02\x03";

    /// The same entries under a 15-bit count
    const SSRT_LONG: &[u8] = b"\0\0\0\x14ssrt\0\0\0\x01\0\x02\x80\x70\x09\x01\x02\x03";

    fn sub_sample_reference_table(is_short_ref: bool) -> SubSampleReferenceTableBox {
        SubSampleReferenceTableBox {
            flags: FullBoxFlags::new(1).unwrap(),
            is_short_ref,
            entries: vec![
                SubSampleDescription {
                    discardable: true,
                    subsample_priority: None,
                    csp_length_minus_1: 0,
                    codec_specific_parameters: None,
                },
                SubSampleDescription {
                    discardable: false,
                    subsample_priority: Some(9),
                    csp_length_minus_1: 2,
                    codec_specific_parameters: Some(0x01_0203),
                },
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_under_either_count() {
        assert_eq!(
            (
                SubSampleReferenceTableBox::decode(SSRT_SHORT),
                SubSampleReferenceTableBox::decode(SSRT_LONG)
            ),
            (
                Ok((sub_sample_reference_table(true), b"".as_slice())),
                Ok((sub_sample_reference_table(false), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_under_either_count() {
        assert_eq!(
            (
                encoded(&sub_sample_reference_table(true)),
                encoded(&sub_sample_reference_table(false))
            ),
            (SSRT_SHORT.to_vec(), SSRT_LONG.to_vec())
        );
    }

    #[test]
    fn more_entries_than_a_7_bit_count_holds_are_refused_when_written() {
        let mut table = sub_sample_reference_table(true);
        table.entries = vec![SubSampleDescription::default(); 128];

        assert_eq!(
            table.encode(&mut [0; 141]),
            Err(Error::invalid_value("entry_count"))
        );
    }

    #[test]
    fn parameters_wider_than_their_length_are_refused_when_written() {
        let mut table = sub_sample_reference_table(true);
        table.entries.last_mut().unwrap().codec_specific_parameters = Some(0x0100_0000);

        assert_eq!(
            table.encode(&mut [0; 19]),
            Err(Error::invalid_value("codec_specific_parameters"))
        );
    }

    #[test]
    fn a_count_of_more_entries_than_the_payload_holds_is_refused() {
        assert_eq!(
            SubSampleReferenceTableBox::decode_payload(b"\0\0\0\0\x83\x80\x80"),
            Err(Error::truncated_payload(8, 7))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SubSampleReferenceTableBox::decode_payload(b"\x01\0\0\0\x80"),
            Err(Error::unsupported_version(1))
        );
    }
}
