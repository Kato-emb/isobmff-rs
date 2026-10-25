//! [`IdentifiedMediaDataBox`], media data a data reference can name

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::field::FieldCodec;

/// [`IdentifiedMediaDataBox`] (`imda`), ISO/IEC 14496-12 §8.1.4
///
/// Media data as a [`MediaDataBox`](crate::MediaDataBox) holds it, with an
/// identifier that data references set up to it use. The payload is held
/// whole, so a box read holds a copy of all the media data it frames.
#[doc(alias = "imda")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct IdentifiedMediaDataBox {
    /// Identifier, which differs from that of every other `imda` of the file
    pub imda_identifier: u32,
    /// Media data the box contains
    pub data: Vec<u8>,
}

impl BoxDefinition for IdentifiedMediaDataBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"imda");
}

impl BoxDecode for IdentifiedMediaDataBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            imda_identifier: u32::read(reader)?,
            data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for IdentifiedMediaDataBox {
    fn payload_len(&self) -> u64 {
        self.imda_identifier
            .encoded_len()
            .saturating_add(u64::try_from(self.data.len()).unwrap_or(u64::MAX))
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        self.imda_identifier.write(writer)?;
        writer.write_slice(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::IdentifiedMediaDataBox;
    use crate::encoded;

    /// `imda` of identifier 7 and three bytes of media data
    const IMDA: &[u8] = b"\0\0\0\x0fimda\0\0\0\x07\xaa\xbb\xcc";

    fn identified_media_data() -> IdentifiedMediaDataBox {
        IdentifiedMediaDataBox {
            imda_identifier: 7,
            data: vec![0xaa, 0xbb, 0xcc],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            IdentifiedMediaDataBox::decode(IMDA),
            Ok((identified_media_data(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&identified_media_data()), IMDA);
    }

    #[test]
    fn a_payload_ending_inside_the_identifier_is_refused() {
        assert_eq!(
            IdentifiedMediaDataBox::decode_payload(b"\0\0"),
            Err(Error::truncated_payload(4, 2))
        );
    }
}
