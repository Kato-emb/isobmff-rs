//! [`SampleEntry`], the fields every sample entry opens with, and the bytes the class derived for its format lays out

use alloc::vec::Vec;

use isobmff_core::{BoxEncode, Error, FieldReader, FieldWriter, FourCc};

/// [`SampleEntry`], ISO/IEC 14496-12 §8.5.2
///
/// One entry of a [`SampleDescriptionBox`](crate::SampleDescriptionBox): the
/// fields of the abstract class every sample entry derives from, and the
/// bytes after them as found. The class that lays those bytes out depends on
/// both the [`format`](Self::format) and the handler type of the track, so
/// the entry is read into that class by a caller who knows the handler type,
/// and written back from it.
///
/// The format is the box type of the entry, which a
/// [`SampleDescriptionBox`](crate::SampleDescriptionBox) reads into
/// [`format`](Self::format) and writes from it, so an entry of any format,
/// registered or not, reads. The six `reserved` bytes are read past and
/// written as zero.
///
/// The entry has no box type of its own, so it implements neither
/// [`BoxDecode`](isobmff_core::BoxDecode) nor
/// [`BoxDefinition`](isobmff_core::BoxDefinition): its [`BoxEncode`] writes
/// its payload, and a [`SampleDescriptionBox`](crate::SampleDescriptionBox)
/// writes the header.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleEntry {
    /// Format of the samples the entry describes, such as `avc1` or `mp4a`
    ///
    /// It is written as the box type, so a `format` of `uuid` is refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `format`
    /// when the entry is written.
    pub format: FourCc,
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) in the `dref` of the track from which the samples the entry describes are retrieved
    pub data_reference_index: u16,
    /// Bytes after `data_reference_index`, which the class derived for the format and the handler type lays out
    pub remaining_data: Vec<u8>,
}

impl Default for SampleEntry {
    fn default() -> Self {
        Self {
            format: FourCc::new([0; 4]),
            data_reference_index: 0,
            remaining_data: Vec::new(),
        }
    }
}

impl SampleEntry {
    /// Reads the payload of a box of the box type `format`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside `data_reference_index`.
    pub(crate) fn decode_payload(format: FourCc, payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        reader.read_bytes::<6>()?;

        Ok(Self {
            format,
            data_reference_index: reader.read_u16()?,
            remaining_data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for SampleEntry {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.remaining_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(6 + 2)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[0; 6])?;
        writer.write_u16(self.data_reference_index)?;
        writer.write_slice(&self.remaining_data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxEncode, Error, FourCc};

    use super::SampleEntry;

    /// Payload of an entry of data reference 1, with two bytes its class lays out
    const PAYLOAD: &[u8] = b"\0\0\0\0\0\0\0\x01\xab\xcd";

    fn entry() -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"xvid"),
            data_reference_index: 1,
            remaining_data: vec![0xab, 0xcd],
        }
    }

    #[test]
    fn reads_the_data_reference_index_past_the_reserved_bytes_and_keeps_the_rest() {
        assert_eq!(
            SampleEntry::decode_payload(
                FourCc::new(*b"xvid"),
                b"\xff\xff\xff\xff\xff\xff\0\x01\xab\xcd"
            ),
            Ok(entry())
        );
    }

    #[test]
    fn writes_the_reserved_bytes_as_zero_then_the_data_reference_index_and_the_rest() {
        let mut payload = [0xff; 10];
        entry().encode_payload(&mut payload).unwrap();

        assert_eq!(payload, PAYLOAD);
    }

    #[test]
    fn a_payload_ending_inside_the_data_reference_index_is_refused() {
        assert_eq!(
            SampleEntry::decode_payload(FourCc::new(*b"xvid"), b"\0\0\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 7))
        );
    }
}
