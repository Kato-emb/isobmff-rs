//! [`ESDBox`] (`esds`), ISO/IEC 14496-14 §6.7

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, FieldReader, FieldWriter, FullBoxFields,
    FullBoxFlags,
};

use crate::error::Error;
use crate::es_descriptor::ESDescriptor;

/// Box an MPEG-4 sample entry holds to carry the descriptor of its stream
///
/// [`ESDBox`] (`esds`), ISO/IEC 14496-14 §6.7. The [`ESDescriptor`] is the
/// whole of the payload after the version and flags.
///
/// What goes wrong inside a descriptor is this crate's [`Error`], which
/// [`BoxDecode`] reports for the box.
#[doc(alias = "esds")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ESDBox {
    es: ESDescriptor,
}

impl ESDBox {
    /// Creates the box around the descriptor it carries
    #[must_use]
    pub const fn new(es: ESDescriptor) -> Self {
        Self { es }
    }

    /// Returns the descriptor of the stream
    #[must_use]
    pub const fn es(&self) -> &ESDescriptor {
        &self.es
    }
}

impl BoxDefinition for ESDBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"esds");
}

impl BoxDecode for ESDBox {
    type Error = Error;

    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box) of
    ///   [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the version and flags.
    /// * [`Box`](crate::ErrorKind::Box) of
    ///   [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
    ///   box declares a version other than 0.
    /// * What [`ESDescriptor::decode`] reports.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(isobmff_core::Error::unsupported_version(version).into());
        }

        Ok(Self {
            es: ESDescriptor::decode(reader)?,
        })
    }
}

impl BoxEncode for ESDBox {
    fn payload_len(&self) -> u64 {
        4_u64.saturating_add(self.es.encoded_len())
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), isobmff_core::Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        self.es.encode(writer)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode};

    use super::ESDBox;
    use crate::error::Error;
    use crate::es_descriptor::tests::{aac_descriptor, aac_descriptor_bytes};

    fn encoded_payload(esds: &ESDBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(esds.payload_len()).unwrap()];
        esds.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let esds = ESDBox::new(aac_descriptor());

        let payload = encoded_payload(&esds);

        assert_eq!(payload, [vec![0; 4], aac_descriptor_bytes()].concat());
        assert_eq!(ESDBox::decode_payload(&payload).unwrap(), esds);
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        assert_eq!(
            ESDBox::decode_payload(b"\x01\0\0\0"),
            Err(Error::from(isobmff_core::Error::unsupported_version(1)))
        );
    }

    #[test]
    fn bytes_after_the_descriptor_are_rejected() {
        let payload = [vec![0; 4], aac_descriptor_bytes(), vec![0]].concat();

        assert_eq!(
            ESDBox::decode_payload(&payload),
            Err(Error::from(isobmff_core::Error::trailing_payload(31, 32)))
        );
    }
}
