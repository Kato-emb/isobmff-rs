//! [`T35SampleGroupEntry`], an ITU-T T.35 message the samples of a sample group carry

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

use super::entry::{DerivedEntry, SampleGroupDescriptionEntry, read_class, write_class};

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

impl DerivedEntry for T35SampleGroupEntry {
    const GROUPING_TYPE: FourCc = FourCc::new(*b"it35");

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let &[byte] = reader.read_bytes::<1>()?;

        Ok(Self {
            complete_message_flag: byte & 0x80 != 0,
            itu_t_t35_data: reader.take_remainder().to_vec(),
        })
    }

    fn encoded_len(&self) -> u64 {
        u64::try_from(self.itu_t_t35_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1)
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[if self.complete_message_flag { 0x80 } else { 0 }])?;
        writer.write_slice(&self.itu_t_t35_data)
    }
}

impl TryFrom<&SampleGroupDescriptionEntry> for T35SampleGroupEntry {
    type Error = Error;

    fn try_from(entry: &SampleGroupDescriptionEntry) -> Result<Self, Error> {
        read_class(entry)
    }
}

impl TryFrom<&T35SampleGroupEntry> for SampleGroupDescriptionEntry {
    type Error = Error;

    fn try_from(entry: &T35SampleGroupEntry) -> Result<Self, Error> {
        write_class(entry)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::Error;

    use super::super::entry::{assert_round_trips, entry_of};
    use super::T35SampleGroupEntry;

    #[test]
    fn reads_and_writes_the_bytes_its_syntax_lays_out() {
        assert_round_trips(&[(
            b"it35",
            b"\x80\xb5\0\x31".as_slice(),
            T35SampleGroupEntry {
                complete_message_flag: true,
                itu_t_t35_data: vec![0xb5, 0x00, 0x31],
            },
        )]);
    }

    #[test]
    fn an_entry_of_another_grouping_type_is_refused() {
        assert_eq!(
            [T35SampleGroupEntry::try_from(&entry_of(b"seig", b"\x80\xb5\0\x31")).map(|_| ()),],
            [Err(Error::invalid_value("grouping_type")); 1]
        );
    }
}
