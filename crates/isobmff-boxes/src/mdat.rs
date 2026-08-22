//! [`MediaDataBox`] (`mdat`), ISO/IEC 14496-12 §8.1.1

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// Box that carries media data
///
/// [`MediaDataBox`] (`mdat`), ISO/IEC 14496-12 §8.1.1. The `data` has no
/// structure of its own: it is described by the metadata — see particularly
/// the sample table (§8.5) and the item location box (§8.11.3) — which locates
/// each piece of it by offsets into the file. A presentation may carry any
/// number of these boxes, including none.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::MediaDataBox;
/// use isobmff_core::{BoxDecode, BoxEncode};
///
/// // Media data owns the bytes the metadata describes
/// let media_data = MediaDataBox::new(vec![0xDE, 0xAD, 0xBE, 0xEF]);
///
/// // The box writes as its header followed by the data as it stands
/// let mut buffer = vec![0; usize::try_from(media_data.encoded_len()).unwrap()];
/// media_data.encode(&mut buffer).unwrap();
/// assert_eq!(buffer, b"\0\0\0\x0cmdat\xDE\xAD\xBE\xEF");
///
/// // And the whole box reads back from them, leaving nothing over
/// assert_eq!(
///     MediaDataBox::decode(&buffer).unwrap(),
///     (media_data, b"".as_slice())
/// );
/// ```
#[doc(alias = "mdat")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MediaDataBox {
    data: Vec<u8>,
}

impl MediaDataBox {
    /// Creates the box from the media data it carries
    #[must_use]
    pub const fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Returns the contained media data
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

impl BoxDefinition for MediaDataBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdat");
}

impl BoxDecode for MediaDataBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self::new(reader.take_remainder().to_vec()))
    }
}

impl BoxEncode for MediaDataBox {
    fn payload_len(&self) -> u64 {
        self.data.len() as u64
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_slice(&self.data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode as _, BoxEncode as _};

    use super::MediaDataBox;

    /// Writes the whole box and returns the bytes it occupies
    fn encoded(media_data: &MediaDataBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(media_data.encoded_len()).unwrap()];
        media_data.encode(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_carrying_no_data_writes_as_its_header_alone() {
        let media_data = MediaDataBox::new(Vec::new());

        assert_eq!(encoded(&media_data), b"\0\0\0\x08mdat");
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let media_data = MediaDataBox::new(vec![1, 2, 3, 4, 5]);

        let whole = encoded(&media_data);

        assert_eq!(
            MediaDataBox::decode(&whole).unwrap(),
            (media_data, b"".as_slice())
        );
    }

    #[test]
    fn a_payload_reads_whole_into_the_data() {
        assert_eq!(
            MediaDataBox::decode_payload(b"\xDE\xAD\xBE\xEF").unwrap(),
            MediaDataBox::new(vec![0xDE, 0xAD, 0xBE, 0xEF])
        );
    }

    #[test]
    fn every_data_byte_adds_to_the_payload_length() {
        assert_eq!(MediaDataBox::new(Vec::new()).payload_len(), 0);
        assert_eq!(MediaDataBox::new(vec![0; 7]).payload_len(), 7);
    }
}
