//! [`DegradationPriorityBox`]

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{entries_len, read_to_end, write_entries};

/// [`DegradationPriorityBox`] (`stdp`), ISO/IEC 14496-12 §8.5.3
///
/// The degradation priority of each sample, whose meaning and range the
/// specifications derived from ISO/IEC 14496-12 define.
///
/// The box states no count: its table runs to the end of the payload, as GPAC
/// reads it when it has not yet read the sample size box. The specification
/// takes the number of entries from the `sample_count` of the sample size box.
#[doc(alias = "stdp")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct DegradationPriorityBox {
    /// `priority` of every sample, in sample order
    pub priorities: Vec<u16>,
}

impl BoxDefinition for DegradationPriorityBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stdp");
}

impl BoxDecode for DegradationPriorityBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            priorities: read_to_end(reader)?,
        })
    }
}

impl BoxEncode for DegradationPriorityBox {
    fn payload_len(&self) -> u64 {
        entries_len(&self.priorities).saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        write_entries(writer, &self.priorities)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::DegradationPriorityBox;
    use crate::encoded;

    /// `stdp` of three samples, of priorities 0, 7 and 0x100
    const STDP: &[u8] = b"\0\0\0\x12stdp\0\0\0\0\0\0\0\x07\x01\0";

    fn degradation_priority() -> DegradationPriorityBox {
        DegradationPriorityBox {
            priorities: vec![0, 7, 0x100],
        }
    }

    #[test]
    fn reads_a_priority_from_every_two_bytes_to_the_end_of_the_payload() {
        assert_eq!(
            DegradationPriorityBox::decode(STDP),
            Ok((degradation_priority(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&degradation_priority()), STDP);
    }

    #[test]
    fn a_payload_ending_inside_a_priority_is_refused() {
        assert_eq!(
            DegradationPriorityBox::decode_payload(b"\0\0\0\0\0\x07\x01"),
            Err(Error::truncated_payload(8, 7))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            DegradationPriorityBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
