//! [`DescriptorTag`] and [`RawDescriptor`], the `BaseDescriptor` of ISO/IEC 14496-1 §7.2.2.2

use alloc::vec::Vec;
use core::fmt;

use isobmff_core::{FieldReader, FieldWidth, FieldWriter};

use crate::error::Error;

/// Most bytes an expandable size may take
const MAX_SIZE_LEN: usize = 4;

/// Largest size an expandable size of four bytes states, 28 bits of it
const MAX_BODY_LEN: u64 = (1 << 28) - 1;

/// Tag that names the class of a descriptor
///
/// ISO/IEC 14496-1 §7.2.2.2 opens every descriptor with one byte naming its
/// class; the constants here are the tags this crate reads a class for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorTag(u8);

impl DescriptorTag {
    /// `ES_DescrTag`, an [`ESDescriptor`](crate::ESDescriptor)
    pub const ES: Self = Self(0x03);

    /// `DecoderConfigDescrTag`, a [`DecoderConfigDescriptor`](crate::DecoderConfigDescriptor)
    pub const DECODER_CONFIG: Self = Self(0x04);

    /// `DecSpecificInfoTag`, a [`DecoderSpecificInfo`](crate::DecoderSpecificInfo)
    pub const DECODER_SPECIFIC_INFO: Self = Self(0x05);

    /// `SLConfigDescrTag`, an [`SLConfigDescriptor`](crate::SLConfigDescriptor)
    pub const SL_CONFIG: Self = Self(0x06);

    /// Creates a tag from the byte that carries it
    #[must_use]
    pub const fn new(tag: u8) -> Self {
        Self(tag)
    }

    /// Returns the byte that carries the tag
    #[must_use]
    pub const fn byte(self) -> u8 {
        self.0
    }
}

impl fmt::Display for DescriptorTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{:02x}", self.0)
    }
}

impl fmt::Debug for DescriptorTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DescriptorTag({self})")
    }
}

/// Descriptor this crate has no type for, kept as its tag and the bytes of its body
///
/// A descriptor tree holds classes beyond the ones this crate reads —
/// `IPI_DescrPointer`, `LanguageDescriptor`, the extension descriptors — and
/// each is kept as one of these and written back where it stood.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RawDescriptor {
    tag: DescriptorTag,
    body: Vec<u8>,
}

impl RawDescriptor {
    /// Creates a descriptor from its tag and the bytes of its body
    ///
    /// Returns `None` when `body` is longer than the 28 bits an expandable
    /// size can state.
    #[must_use]
    pub fn new(tag: DescriptorTag, body: Vec<u8>) -> Option<Self> {
        if body.len() as u64 > MAX_BODY_LEN {
            return None;
        }

        Some(Self { tag, body })
    }

    /// Returns the tag naming the class of the descriptor
    #[must_use]
    pub const fn tag(&self) -> DescriptorTag {
        self.tag
    }

    /// Returns the bytes of the body, tag and size excluded
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Returns the length the descriptor occupies, tag and size included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        encoded_len(self.body.len() as u64)
    }

    /// Reads the descriptor that opens `reader`, whatever its tag
    ///
    /// # Errors
    ///
    /// * [`ExpandableSizeTooLong`](crate::ErrorKind::ExpandableSizeTooLong): the size
    ///   runs past four bytes.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the descriptor.
    pub fn decode(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (tag, body) = decode_header(reader)?;

        Ok(Self {
            tag,
            body: body.to_vec(),
        })
    }

    /// Writes the descriptor into the front of `writer`
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): `writer`
    ///   has less than [`encoded_len`](Self::encoded_len) bytes left.
    pub fn encode(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        encode_header(writer, self.tag, self.body.len() as u64)?;
        writer.write_slice(&self.body)?;

        Ok(())
    }
}

/// Returns the length a descriptor of `body_len` occupies, tag and size included
pub(crate) fn encoded_len(body_len: u64) -> u64 {
    1_u64
        .saturating_add(size_len(body_len) as u64)
        .saturating_add(body_len)
}

/// Returns how many bytes the expandable size of `body_len` takes, the fewest
/// that state it
fn size_len(body_len: u64) -> usize {
    let mut length: usize = 1;
    let mut rest = body_len >> 7;
    while rest != 0 {
        length = length.saturating_add(1);
        rest >>= 7;
    }

    length
}

/// Reads the tag and size that open a descriptor and hands back its body
pub(crate) fn decode_header<'payload>(
    reader: &mut FieldReader<'payload>,
) -> Result<(DescriptorTag, &'payload [u8]), Error> {
    let tag = DescriptorTag(reader.read_bytes::<1>()?[0]);

    let mut size: u64 = 0;
    for _ in 0..MAX_SIZE_LEN {
        let byte = reader.read_bytes::<1>()?[0];
        size = (size << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            let body = reader.read_slice(usize::try_from(size).unwrap_or(usize::MAX))?;

            return Ok((tag, body));
        }
    }

    Err(Error::expandable_size_too_long(tag))
}

/// Writes the tag and the size of a body `body_len` long, in the fewest bytes
pub(crate) fn encode_header(
    writer: &mut FieldWriter<'_>,
    tag: DescriptorTag,
    body_len: u64,
) -> Result<(), isobmff_core::Error> {
    if body_len > MAX_BODY_LEN {
        // Why not a kind of this crate: `BoxEncode` reports a box failure, and
        // the constructors bound every body, so this stands for a value no
        // caller can build rather than a situation of its own.
        return Err(isobmff_core::Error::out_of_range(
            body_len,
            FieldWidth::Compact,
        ));
    }

    writer.write_bytes(&[tag.0])?;
    for index in (0..size_len(body_len)).rev() {
        let shift = index.saturating_mul(7);
        let more = if index == 0 { 0 } else { 0x80 };
        // Why not saturate: seven bits are masked out, so the cast is exact.
        writer.write_bytes(&[more | ((body_len >> shift) & 0x7f) as u8])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{FieldReader, FieldWriter};

    use super::{DescriptorTag, RawDescriptor};
    use crate::error::Error;

    fn encoded(descriptor: &RawDescriptor) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(descriptor.encoded_len()).unwrap()];
        let mut writer = FieldWriter::new(&mut buffer);
        descriptor.encode(&mut writer).unwrap();
        writer.finish().unwrap();

        buffer
    }

    #[test]
    fn a_descriptor_reads_back_as_the_value_that_wrote_it() {
        let descriptor = RawDescriptor::new(DescriptorTag::new(0x09), vec![1, 2, 3]).unwrap();

        let bytes = encoded(&descriptor);

        assert_eq!(bytes, [0x09, 0x03, 1, 2, 3]);
        assert_eq!(
            RawDescriptor::decode(&mut FieldReader::new(&bytes)).unwrap(),
            descriptor
        );
    }

    #[test]
    fn a_body_past_127_bytes_takes_a_size_of_two_bytes() {
        let descriptor = RawDescriptor::new(DescriptorTag::new(0x09), vec![0xab; 128]).unwrap();

        let bytes = encoded(&descriptor);

        assert_eq!(bytes.get(..3), Some([0x09, 0x81, 0x00].as_slice()));
        assert_eq!(
            RawDescriptor::decode(&mut FieldReader::new(&bytes)).unwrap(),
            descriptor
        );
    }

    #[test]
    fn a_size_written_in_more_bytes_than_it_needs_still_reads() {
        let bytes = [0x09, 0x80, 0x80, 0x80, 0x03, 1, 2, 3];

        assert_eq!(
            RawDescriptor::decode(&mut FieldReader::new(&bytes)).unwrap(),
            RawDescriptor::new(DescriptorTag::new(0x09), vec![1, 2, 3]).unwrap()
        );
    }

    #[test]
    fn a_size_running_past_four_bytes_is_rejected() {
        let bytes = [0x09, 0x80, 0x80, 0x80, 0x83, 1, 2, 3];

        assert_eq!(
            RawDescriptor::decode(&mut FieldReader::new(&bytes)),
            Err(Error::expandable_size_too_long(DescriptorTag::new(0x09)))
        );
    }

    #[test]
    fn a_body_the_payload_cuts_short_is_rejected_as_truncated() {
        assert_eq!(
            RawDescriptor::decode(&mut FieldReader::new(&[0x09, 0x03, 1])),
            Err(Error::from(isobmff_core::Error::truncated_payload(5, 3)))
        );
    }
}
