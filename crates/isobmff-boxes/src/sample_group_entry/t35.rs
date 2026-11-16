//! [`T35SampleGroupEntry`], an ITU-T T.35 message the samples of a sample group carry

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter};

/// [`T35SampleGroupEntry`] (`it35`), ISO/IEC 14496-12 §10.13
///
/// An ITU-T T.35 message, whole or its part that identifies its type. The
/// data runs to the end of the entry, so it takes the length a
/// `SampleGroupDescriptionBox` of version 1 or more states.
#[doc(alias = "it35")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct T35SampleGroupEntry {
    /// Whether `itu_t_t35_data` holds the whole message, which the samples then do not
    pub complete_message_flag: bool,
    /// The message from its first byte, without emulation prevention bits
    pub itu_t_t35_data: Vec<u8>,
}

impl T35SampleGroupEntry {
    pub(super) fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let &[byte] = reader.read_bytes::<1>()?;

        Ok(Self {
            complete_message_flag: byte & 0x80 != 0,
            itu_t_t35_data: reader.take_remainder().to_vec(),
        })
    }

    pub(super) fn encoded_len(&self) -> u64 {
        u64::try_from(self.itu_t_t35_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1)
    }

    pub(super) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[if self.complete_message_flag { 0x80 } else { 0 }])?;
        writer.write_slice(&self.itu_t_t35_data)
    }
}
