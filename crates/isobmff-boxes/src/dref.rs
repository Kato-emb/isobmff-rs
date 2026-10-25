//! [`DataReferenceBox`], the table of the sources of the media data of a track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth,
    FieldWriter, RawBox,
};

use crate::data_entry::{
    DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox,
};
use crate::full_box::{read_version_zero, write_version_zero};

/// [`DataReferenceBox`] (`dref`), ISO/IEC 14496-12 §8.7.2
///
/// Table of the sources of the media data of a track, which the
/// `data_reference_index` of a sample entry indexes from 1.
///
/// An entry of a type this crate does not read is kept as its bytes, in the
/// place it came, and so is an entry of a type it reads that does not decode,
/// such as one of a version it does not read.
///
/// The box holds one or more entries. A box of none reads, but writing it is
/// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `entry_count`, so a box built from [`Default`] is given an entry before it
/// is written.
///
/// # Untrusted input
///
/// The box reads the `entry_count` entries it states, after holding the count
/// against the payload, and allocates room for an entry only once it has read
/// it, so what it allocates is bounded by the length of the payload, which the
/// caller sets.
#[doc(alias = "dref")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct DataReferenceBox {
    /// Entries, in the order the `data_reference_index` of a sample entry counts them
    pub data_entries: Vec<DataEntry>,
}

impl BoxDefinition for DataReferenceBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"dref");
}

impl BoxDecode for DataReferenceBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;
        let entry_count = reader.read_u32()?;
        // Why not the twelve bytes of a full box: an entry of a type this crate
        // does not read may be a box of its header alone, eight bytes.
        reader.require(u64::from(entry_count).saturating_mul(8))?;
        let mut rest = reader.take_remainder();
        // Why not room for `entry_count` entries up front: an entry may take only
        // the eight bytes of its header, while a `DataEntry` takes several times
        // that, so the room would outgrow the payload it was read from.
        let mut data_entries = Vec::new();
        for _ in 0..entry_count {
            let (entry, after) = RawBox::split_first(rest)?;
            rest = after;
            let box_type = entry.header().box_type();
            let payload = entry.payload();
            let read = if box_type == DataEntryUrlBox::BOX_TYPE {
                DataEntryUrlBox::decode_payload(payload).map(DataEntry::Url)
            } else if box_type == DataEntryUrnBox::BOX_TYPE {
                DataEntryUrnBox::decode_payload(payload).map(DataEntry::Urn)
            } else if box_type == DataEntryImdaBox::BOX_TYPE {
                DataEntryImdaBox::decode_payload(payload).map(DataEntry::Imda)
            } else if box_type == DataEntrySeqNumImdaBox::BOX_TYPE {
                DataEntrySeqNumImdaBox::decode_payload(payload).map(DataEntry::SeqNumImda)
            } else {
                Err(Error::unsupported_box(box_type))
            };
            data_entries.push(read.unwrap_or_else(|_| {
                DataEntry::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self { data_entries })
    }
}

impl BoxEncode for DataReferenceBox {
    fn payload_len(&self) -> u64 {
        self.data_entries
            .iter()
            .map(|entry| match entry {
                DataEntry::Url(url) => url.encoded_len(),
                DataEntry::Urn(urn) => urn.encoded_len(),
                DataEntry::Imda(imda) => imda.encoded_len(),
                DataEntry::SeqNumImda(seq_num_imda) => seq_num_imda.encoded_len(),
                DataEntry::Other(other) => other.encoded_len(),
            })
            .fold(4 + 4, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.data_entries.is_empty() {
            return Err(Error::invalid_value("entry_count"));
        }
        write_version_zero(writer)?;
        writer.write_unsigned(
            FieldWidth::Compact,
            u64::try_from(self.data_entries.len()).unwrap_or(u64::MAX),
        )?;
        let mut rest = writer.take_remainder();
        for entry in &self.data_entries {
            rest = match entry {
                DataEntry::Url(url) => url.encode(rest)?,
                DataEntry::Urn(urn) => urn.encode(rest)?,
                DataEntry::Imda(imda) => imda.encode(rest)?,
                DataEntry::SeqNumImda(seq_num_imda) => seq_num_imda.encode(rest)?,
                DataEntry::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error};

    use super::DataReferenceBox;
    use crate::data_entry::{DataEntry, DataEntryImdaBox, DataEntryUrlBox};
    use crate::encoded;

    /// `url ` of media data in the same file
    const URL: &[u8] = b"\0\0\0\x0curl \0\0\0\x01";

    /// `imdt` of the identifier 7
    const IMDT: &[u8] = b"\0\0\0\x10imdt\0\0\0\0\0\0\0\x07";

    /// Entry of a type this crate does not read
    const ALIS: &[u8] = b"\0\0\0\x0ealis\0\0\0\x01\x12\x34";

    /// `dref` of an `alis`, a `url ` and an `imdt`, in that order
    fn data_reference_bytes() -> Vec<u8> {
        [
            b"\0\0\0\x3adref\0\0\0\0\0\0\0\x03".as_slice(),
            ALIS,
            URL,
            IMDT,
        ]
        .concat()
    }

    fn data_reference() -> DataReferenceBox {
        DataReferenceBox {
            data_entries: vec![
                DataEntry::Other(AnyBox::from_raw_bytes(
                    BoxType::compact(*b"alis"),
                    b"\0\0\0\x01\x12\x34".to_vec(),
                )),
                DataEntry::Url(DataEntryUrlBox::default()),
                DataEntry::Imda(DataEntryImdaBox {
                    imda_ref_identifier: 7,
                }),
            ],
        }
    }

    #[test]
    fn reads_its_entries_in_order_keeping_one_of_a_type_it_does_not_read_in_its_place() {
        assert_eq!(
            DataReferenceBox::decode(&data_reference_bytes()),
            Ok((data_reference(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_its_entries_in_the_order_they_are_held() {
        assert_eq!(encoded(&data_reference()), data_reference_bytes());
    }

    #[test]
    fn an_entry_of_a_version_it_does_not_read_is_kept_as_its_bytes() {
        let version_1 = b"\0\0\0\x0curl \x01\0\0\x01";
        let bytes = [b"\0\0\0\x1cdref\0\0\0\0\0\0\0\x01".as_slice(), version_1].concat();

        assert_eq!(
            DataReferenceBox::decode(&bytes),
            Ok((
                DataReferenceBox {
                    data_entries: vec![DataEntry::Other(AnyBox::from_raw_bytes(
                        BoxType::compact(*b"url "),
                        b"\x01\0\0\x01".to_vec(),
                    ))],
                },
                b"".as_slice()
            ))
        );
    }

    #[test]
    fn a_box_of_no_entry_reads_but_is_refused_when_written() {
        let (read, _) = DataReferenceBox::decode(b"\0\0\0\x10dref\0\0\0\0\0\0\0\0").unwrap();

        assert_eq!(
            (read.clone(), read.encode(&mut [0; 16])),
            (
                DataReferenceBox::default(),
                Err(Error::invalid_value("entry_count"))
            )
        );
    }

    #[test]
    fn a_count_the_payload_cannot_hold_is_refused_before_an_entry_is_read() {
        assert_eq!(
            DataReferenceBox::decode_payload(b"\0\0\0\0\xff\xff\xff\xff\0\0\0\x0curl \0\0\0\x01"),
            Err(Error::truncated_payload(8 + u64::from(u32::MAX) * 8, 20))
        );
    }

    #[test]
    fn a_count_beyond_the_entries_framed_is_refused() {
        let bytes = [b"\0\0\0\x0c\0\0\0\x02".as_slice(), URL, b"\0\0\0\x00"].concat();

        assert_eq!(
            DataReferenceBox::decode_payload(&bytes),
            Err(Error::truncated_header(8, 4))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            DataReferenceBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
