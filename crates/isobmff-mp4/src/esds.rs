//! [`ESDBox`] (`esds`), ISO/IEC 14496-14 §6.7

use isobmff_core::{
    BoxDefinition, BoxEncode, BoxType, FieldReader, FieldWriter, FullBoxFields, FullBoxFlags,
    RawBox,
};

use crate::error::Error;
use crate::es_descriptor::ESDescriptor;

/// Box an MPEG-4 sample entry holds to carry the descriptor of its stream
///
/// [`ESDBox`] (`esds`), ISO/IEC 14496-14 §6.7. The [`ESDescriptor`] is the
/// whole of the payload after the version and flags.
///
/// The payload is read by [`decode_payload`](Self::decode_payload) rather than
/// [`BoxDecode`](isobmff_core::BoxDecode): what goes wrong inside a descriptor
/// is this crate's [`Error`], which that trait has no room for. Writing is
/// [`BoxEncode`] as for any box.
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

    /// Reads the box from its payload, header excluded
    ///
    /// # Errors
    ///
    /// * [`Box`](crate::ErrorKind::Box) of
    ///   [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
    ///   box declares a version other than 0.
    /// * [`Box`](crate::ErrorKind::Box) of
    ///   [`TrailingPayload`](isobmff_core::ErrorKind::TrailingPayload): bytes
    ///   follow the descriptor.
    /// * What [`ESDescriptor::decode`] reports.
    pub fn decode_payload(payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(isobmff_core::Error::unsupported_version(version).into());
        }

        let es = ESDescriptor::decode(&mut reader)?;
        reader.finish()?;

        Ok(Self { es })
    }
}

/// Reads the `esds` child of a sample entry into the slot it must fill once
///
/// # Errors
///
/// * [`Box`](crate::ErrorKind::Box) of
///   [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): `slot` was
///   filled already.
/// * What [`ESDBox::decode_payload`] reports, with `esds` on the
///   [`containers`](isobmff_core::Error::containers) path of a box failure.
pub(crate) fn decode_child(slot: &mut Option<ESDBox>, child: RawBox<'_>) -> Result<(), Error> {
    if slot.is_some() {
        return Err(isobmff_core::Error::duplicate_box(ESDBox::BOX_TYPE).into());
    }
    let es = ESDBox::decode_payload(child.payload())
        .map_err(|error| error.in_container(ESDBox::BOX_TYPE))?;
    *slot = Some(es);

    Ok(())
}

impl BoxDefinition for ESDBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"esds");
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

    use isobmff_core::BoxEncode;

    use super::ESDBox;
    use crate::error::{Error, ErrorKind};
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
            ESDBox::decode_payload(&payload).unwrap_err().kind(),
            ErrorKind::Box(isobmff_core::ErrorKind::TrailingPayload)
        );
    }
}
