//! [`ItemInfoEntry`], the information about one item, and what its `item_type` adds to it

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    FullBoxFlags, FullBoxHeader, Utf8String,
};

use crate::full_box::{read_u16_or_u32, write_u16_or_u32};
use crate::table::{entries_len, read_entries, write_entries};

/// [`FdItemInfoExtension`], ISO/IEC 14496-12 §8.11.6
///
/// The file delivery information of an item, the extension of `extension_type`
/// `fdel` of an [`ItemInfoEntry`] of version 1.
#[doc(alias = "fdel", alias = "FDItemInfoExtension")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct FdItemInfoExtension {
    /// URI of the file, as HTTP/1.1 defines it
    pub content_location: Utf8String,
    /// MD5 digest of the file
    pub content_md5: Utf8String,
    /// Total length in bytes of the file before content encoding
    pub content_length: u64,
    /// Total length in bytes of the file after content encoding
    pub transfer_length: u64,
    /// File groups the file belongs to
    ///
    /// More than 255 are refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
    /// `entry_count` when the entry is written.
    pub group_ids: Vec<u32>,
}

/// Extension of an [`ItemInfoEntry`] of version 1, as its `extension_type` lays it out
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ItemInfoExtension {
    /// Extension of `extension_type` `fdel`
    FileDelivery(FdItemInfoExtension),
    /// Extension of any other `extension_type`, kept as the bytes that follow it
    Other {
        /// Type of the extension
        extension_type: FourCc,
        /// Bytes of the extension
        data: Vec<u8>,
    },
}

/// What an [`ItemInfoEntry`] says the item is: its `item_type` and the fields that type adds, or the content type of versions 0 and 1
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ItemType {
    /// Fields of versions 0 and 1, which carry no `item_type`
    ContentType {
        /// MIME type of the item, after any content decoding
        content_type: Utf8String,
        /// Content encoding of the item as HTTP/1.1 defines it, the empty string for none, where the entry carries the field
        content_encoding: Option<Utf8String>,
        /// Extension of version 1, where the entry carries one
        extension: Option<ItemInfoExtension>,
    },
    /// `item_type` `mime`, an item of a MIME type
    Mime {
        /// MIME type of the item, after any content decoding
        content_type: Utf8String,
        /// Content encoding of the item as HTTP/1.1 defines it, the empty string for none, where the entry carries the field
        content_encoding: Option<Utf8String>,
    },
    /// `item_type` `uri `, metadata typed by a URI
    Uri {
        /// Absolute URI that indicates the type
        item_uri_type: Utf8String,
    },
    /// Any other `item_type`, with the bytes that follow `item_name`
    Other {
        /// Type of the item, typically four printable characters
        item_type: FourCc,
        /// Bytes that follow `item_name`, which most types leave empty
        data: Vec<u8>,
    },
}

impl ItemType {
    /// `item_type` of an item of a MIME type
    const MIME: FourCc = FourCc::new(*b"mime");
    /// `item_type` of metadata typed by a URI
    const URI: FourCc = FourCc::new(*b"uri ");
}

/// [`ItemInfoEntry`] (`infe`), ISO/IEC 14496-12 §8.11.6
///
/// The information about one item: its `item_ID`, its protection, its name,
/// and what kind of item it is.
///
/// The entry is written in the `version` it holds, 0 to 3. Versions 0 and 1
/// hold an [`ItemType::ContentType`], and version 1 alone an extension;
/// versions 2 and 3 hold an `item_type`, and version 3 widens `item_ID` to 32
/// bits. An [`ItemType`] the version does not carry is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `item_type`, or
/// for `extension_type`, when the entry is written, and so is an `item_ID`
/// that does not fit the version. A field the entry may leave off, such as
/// `content_encoding`, is [`None`] where the entry ends before it, and an
/// extension of no `content_encoding` before it is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `content_encoding` when it is written.
///
/// The strings are read whether or not their null terminator is there, and
/// written with it.
#[doc(alias = "infe")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ItemInfoEntry {
    /// Version of the entry, 0 to 3
    pub version: u8,
    /// Flags of the entry
    ///
    /// In versions 2 and 3, bit 0 set says the item is not intended to be a
    /// part of the presentation.
    pub flags: FullBoxFlags,
    /// `item_ID` of the item, or 0 for the primary resource, such as the XML of an `XMLBox`
    pub item_id: u32,
    /// 0 for an unprotected item, or the index, from 1, of its protection in the `ItemProtectionBox`
    pub item_protection_index: u16,
    /// Symbolic name of the item, a relative URL
    pub item_name: Utf8String,
    /// What the item is, and the fields that says it carries
    pub item_type: ItemType,
}

impl Default for ItemInfoEntry {
    fn default() -> Self {
        Self {
            version: 2,
            flags: FullBoxFlags::ZERO,
            item_id: 0,
            item_protection_index: 0,
            item_name: Utf8String::default(),
            item_type: ItemType::Other {
                item_type: FourCc::new([0; 4]),
                data: Vec::new(),
            },
        }
    }
}

/// Reads a `utf8string` that runs to its null terminator, or to the end of the payload where it has none
pub(crate) fn read_string(reader: &mut FieldReader<'_>) -> Result<Utf8String, Error> {
    let rest = reader.remainder();
    let len = rest
        .iter()
        .position(|byte| *byte == 0)
        .map_or(rest.len(), |end| end.saturating_add(1));

    Ok(Utf8String::from_slice(reader.read_slice(len)?))
}

/// Reads a `utf8string` the entry may leave off, [`None`] where the payload has ended
fn read_optional_string(reader: &mut FieldReader<'_>) -> Result<Option<Utf8String>, Error> {
    if reader.remainder().is_empty() {
        return Ok(None);
    }

    read_string(reader).map(Some)
}

/// Writes `string` and its null terminator
///
/// # Errors
///
/// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String):
///   `string` is not UTF-8.
pub(crate) fn write_string(writer: &mut FieldWriter<'_>, string: &Utf8String) -> Result<(), Error> {
    if string.as_str().is_none() {
        return Err(Error::invalid_utf8_string());
    }
    writer.write_slice(string.as_bytes())?;
    writer.write_bytes(&[0])
}

impl ItemInfoExtension {
    /// Reads the extension of `extension_type` that runs to the end of the payload
    fn read(extension_type: FourCc, reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        if extension_type != FourCc::new(*b"fdel") {
            return Ok(Self::Other {
                extension_type,
                data: reader.take_remainder().to_vec(),
            });
        }
        let content_location = read_string(reader)?;
        let content_md5 = read_string(reader)?;
        let content_length = reader.read_u64()?;
        let transfer_length = reader.read_u64()?;
        let [entry_count] = *reader.read_bytes::<1>()?;
        let group_ids = read_entries(reader, u32::from(entry_count), 4, |reader| {
            reader.read_u32()
        })?;

        Ok(Self::FileDelivery(FdItemInfoExtension {
            content_location,
            content_md5,
            content_length,
            transfer_length,
            group_ids,
        }))
    }

    /// Returns the bytes the extension and its `extension_type` take
    fn encoded_len(&self) -> u64 {
        let extension_len = match self {
            Self::FileDelivery(file_delivery) => file_delivery
                .content_location
                .encoded_len()
                .saturating_add(file_delivery.content_md5.encoded_len())
                .saturating_add(entries_len(&file_delivery.group_ids))
                .saturating_add(8 + 8 + 1),
            Self::Other { data, .. } => u64::try_from(data.len()).unwrap_or(u64::MAX),
        };

        extension_len.saturating_add(4)
    }

    /// Writes the `extension_type` and the extension
    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        match self {
            Self::FileDelivery(file_delivery) => {
                let entry_count = u8::try_from(file_delivery.group_ids.len())
                    .map_err(|_| Error::invalid_value("entry_count"))?;
                writer.write_bytes(b"fdel")?;
                write_string(writer, &file_delivery.content_location)?;
                write_string(writer, &file_delivery.content_md5)?;
                writer.write_u64(file_delivery.content_length)?;
                writer.write_u64(file_delivery.transfer_length)?;
                writer.write_bytes(&[entry_count])?;
                write_entries(writer, &file_delivery.group_ids)
            }
            Self::Other {
                extension_type,
                data,
            } => {
                writer.write_bytes(extension_type.as_bytes())?;
                writer.write_slice(data)
            }
        }
    }
}

impl BoxDefinition for ItemInfoEntry {
    const BOX_TYPE: BoxType = BoxType::compact(*b"infe");
}

impl BoxDecode for ItemInfoEntry {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let version = header.version();
        if version > 3 {
            return Err(Error::unsupported_version(version));
        }
        let item_id = read_u16_or_u32(reader, version == 3)?;
        let item_protection_index = reader.read_u16()?;
        let item_type = (version >= 2)
            .then(|| reader.read_bytes::<4>().map(|bytes| FourCc::new(*bytes)))
            .transpose()?;
        let item_name = read_string(reader)?;
        let item_type = match item_type {
            None => {
                let content_type = read_string(reader)?;
                let content_encoding = read_optional_string(reader)?;
                let extension_type = (version == 1 && reader.remainder().len() >= 4)
                    .then(|| reader.read_bytes::<4>().map(|bytes| FourCc::new(*bytes)))
                    .transpose()?;
                ItemType::ContentType {
                    content_type,
                    content_encoding,
                    extension: extension_type
                        .map(|extension_type| ItemInfoExtension::read(extension_type, reader))
                        .transpose()?,
                }
            }
            Some(ItemType::MIME) => ItemType::Mime {
                content_type: read_string(reader)?,
                content_encoding: read_optional_string(reader)?,
            },
            Some(ItemType::URI) => ItemType::Uri {
                item_uri_type: read_string(reader)?,
            },
            Some(item_type) => ItemType::Other {
                item_type,
                data: reader.take_remainder().to_vec(),
            },
        };

        Ok(Self {
            version,
            flags: header.flags(),
            item_id,
            item_protection_index,
            item_name,
            item_type,
        })
    }
}

impl BoxEncode for ItemInfoEntry {
    fn payload_len(&self) -> u64 {
        let fields_len = match self.version {
            0 | 1 => 4 + 2 + 2,
            2 => 4 + 2 + 2 + 4,
            _ => 4 + 4 + 2 + 4,
        };
        let item_type_len = match &self.item_type {
            ItemType::ContentType {
                content_type,
                content_encoding,
                ..
            }
            | ItemType::Mime {
                content_type,
                content_encoding,
            } => content_type
                .encoded_len()
                .saturating_add(content_encoding.as_ref().map_or(0, Utf8String::encoded_len)),
            ItemType::Uri { item_uri_type } => item_uri_type.encoded_len(),
            ItemType::Other { data, .. } => u64::try_from(data.len()).unwrap_or(u64::MAX),
        };
        let extension_len = match &self.item_type {
            ItemType::ContentType { extension, .. } => {
                extension.as_ref().map_or(0, ItemInfoExtension::encoded_len)
            }
            ItemType::Mime { .. } | ItemType::Uri { .. } | ItemType::Other { .. } => 0,
        };

        self.item_name
            .encoded_len()
            .saturating_add(item_type_len)
            .saturating_add(extension_len)
            .saturating_add(fields_len)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if self.version > 3 {
            return Err(Error::unsupported_version(self.version));
        }
        let item_type = match (self.version, &self.item_type) {
            (0 | 1, ItemType::ContentType { .. }) => None,
            (2 | 3, ItemType::Mime { .. }) => Some(ItemType::MIME),
            (2 | 3, ItemType::Uri { .. }) => Some(ItemType::URI),
            (2 | 3, ItemType::Other { item_type, .. }) => Some(*item_type),
            _ => return Err(Error::invalid_value("item_type")),
        };

        writer.write_bytes(&FullBoxHeader::new(self.version, self.flags).to_bytes())?;
        write_u16_or_u32(
            writer,
            self.version == 3,
            u64::from(self.item_id),
            "item_ID",
        )?;
        writer.write_u16(self.item_protection_index)?;
        if let Some(item_type) = item_type {
            writer.write_bytes(item_type.as_bytes())?;
        }
        write_string(writer, &self.item_name)?;
        match &self.item_type {
            ItemType::ContentType {
                content_type,
                content_encoding,
                ..
            }
            | ItemType::Mime {
                content_type,
                content_encoding,
            } => {
                write_string(writer, content_type)?;
                if let Some(content_encoding) = content_encoding {
                    write_string(writer, content_encoding)?;
                }
            }
            ItemType::Uri { item_uri_type } => write_string(writer, item_uri_type)?,
            ItemType::Other { data, .. } => writer.write_slice(data)?,
        }
        if let ItemType::ContentType {
            content_encoding,
            extension: Some(extension),
            ..
        } = &self.item_type
        {
            match (self.version, content_encoding) {
                (1, Some(_)) => extension.write(writer)?,
                (1, None) => return Err(Error::invalid_value("content_encoding")),
                _ => return Err(Error::invalid_value("extension_type")),
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FourCc, FullBoxFlags, Utf8String};

    use super::{FdItemInfoExtension, ItemInfoEntry, ItemInfoExtension, ItemType};
    use crate::encoded;

    /// `infe` of version 0: item 1, unprotected, `drm.xml` of `text/xml` and no content encoding
    const INFE_V0: &[u8] = b"\0\0\0\x21infe\0\0\0\0\0\x01\0\0drm.xml\0text/xml\0";

    /// `infe` of version 1: item 2, `a.bin` of `application/octet-stream`, `gzip`, with an `fdel` extension
    const INFE_V1: &[u8] =
        b"\0\0\0\x5dinfe\x01\0\0\0\0\x02\0\0a.bin\0application/octet-stream\0gzip\0\
        fdelhttp://a/a.bin\0\0\0\0\0\0\0\0\0\x10\0\0\0\0\0\0\0\x08\x01\0\0\0\x07";

    /// `infe` of version 2 and `item_type` `mime`: item 3, protected by the first scheme, `doc` of `text/plain`
    const INFE_MIME: &[u8] = b"\0\0\0\x23infe\x02\0\0\0\0\x03\0\x01mimedoc\0text/plain\0";

    /// `infe` of version 2 and `item_type` `uri `: item 4, of no name, typed `urn:x`
    const INFE_URI: &[u8] = b"\0\0\0\x1binfe\x02\0\0\0\0\x04\0\0uri \0urn:x\0";

    /// `infe` of version 3, hidden, and `item_type` `hvc1`: item 2^16, named `image`
    const INFE_V3: &[u8] = b"\0\0\0\x1cinfe\x03\0\0\x01\0\x01\0\0\0\0hvc1image\0";

    fn string(text: &str) -> Utf8String {
        Utf8String::new(String::from(text)).unwrap()
    }

    fn entry(version: u8, item_id: u32, item_name: &str, item_type: ItemType) -> ItemInfoEntry {
        ItemInfoEntry {
            version,
            flags: FullBoxFlags::ZERO,
            item_id,
            item_protection_index: 0,
            item_name: string(item_name),
            item_type,
        }
    }

    fn entries() -> Vec<ItemInfoEntry> {
        let mut mime = entry(
            2,
            3,
            "doc",
            ItemType::Mime {
                content_type: string("text/plain"),
                content_encoding: None,
            },
        );
        mime.item_protection_index = 1;
        let mut hidden = entry(
            3,
            1 << 16,
            "image",
            ItemType::Other {
                item_type: FourCc::new(*b"hvc1"),
                data: vec![],
            },
        );
        hidden.flags = FullBoxFlags::new(1).unwrap();

        vec![
            entry(
                0,
                1,
                "drm.xml",
                ItemType::ContentType {
                    content_type: string("text/xml"),
                    content_encoding: None,
                    extension: None,
                },
            ),
            entry(
                1,
                2,
                "a.bin",
                ItemType::ContentType {
                    content_type: string("application/octet-stream"),
                    content_encoding: Some(string("gzip")),
                    extension: Some(ItemInfoExtension::FileDelivery(FdItemInfoExtension {
                        content_location: string("http://a/a.bin"),
                        content_md5: string(""),
                        content_length: 16,
                        transfer_length: 8,
                        group_ids: vec![7],
                    })),
                },
            ),
            mime,
            entry(
                2,
                4,
                "",
                ItemType::Uri {
                    item_uri_type: string("urn:x"),
                },
            ),
            hidden,
        ]
    }

    const ENCODED: [&[u8]; 5] = [INFE_V0, INFE_V1, INFE_MIME, INFE_URI, INFE_V3];

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_each_version_and_item_type() {
        let read: Vec<_> = ENCODED
            .iter()
            .map(|bytes| ItemInfoEntry::decode(bytes).map(|(entry, _)| entry))
            .collect();

        assert_eq!(read, entries().into_iter().map(Ok).collect::<Vec<_>>());
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_each_version_and_item_type() {
        let written: Vec<_> = entries().iter().map(encoded).collect();

        assert_eq!(written, ENCODED.map(<[u8]>::to_vec));
    }

    #[test]
    fn the_bytes_after_the_name_of_an_unknown_item_type_are_kept() {
        let bytes = b"\0\0\0\x18infe\x02\0\0\0\0\x05\0\0xyzwa\0\x01\x02";
        let expected = entry(
            2,
            5,
            "a",
            ItemType::Other {
                item_type: FourCc::new(*b"xyzw"),
                data: vec![1, 2],
            },
        );

        assert_eq!(
            ItemInfoEntry::decode(bytes).map(|(read, _)| (encoded(&read), read)),
            Ok((bytes.to_vec(), expected))
        );
    }

    #[test]
    fn strings_without_their_null_terminator_read_and_are_written_with_it() {
        let (read, _) =
            ItemInfoEntry::decode(b"\0\0\0\x18infe\x02\0\0\0\0\x01\0\0uri \0urn").unwrap();

        assert_eq!(
            encoded(&read),
            b"\0\0\0\x19infe\x02\0\0\0\0\x01\0\0uri \0urn\0"
        );
    }

    #[test]
    fn an_item_type_the_version_does_not_carry_is_refused_when_written() {
        let mut mime_in_version_0 = entries().remove(2);
        mime_in_version_0.version = 0;
        let mut extension_in_version_0 = entries().remove(1);
        extension_in_version_0.version = 0;

        assert_eq!(
            (
                mime_in_version_0.encode(&mut [0; 64]),
                extension_in_version_0.encode(&mut [0; 128])
            ),
            (
                Err(Error::invalid_value("item_type")),
                Err(Error::invalid_value("extension_type"))
            )
        );
    }

    #[test]
    fn an_extension_without_a_content_encoding_before_it_is_refused_when_written() {
        let mut entry = entries().remove(1);
        if let ItemType::ContentType {
            content_encoding, ..
        } = &mut entry.item_type
        {
            *content_encoding = None;
        }

        assert_eq!(
            entry.encode(&mut [0; 128]),
            Err(Error::invalid_value("content_encoding"))
        );
    }

    #[test]
    fn version_2_refuses_an_item_id_wider_than_16_bits() {
        let mut wide = entries().remove(4);
        wide.version = 2;

        assert_eq!(
            wide.encode(&mut [0; 64]),
            Err(Error::invalid_value("item_ID"))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            (
                ItemInfoEntry::decode_payload(b"\x02\0\0\0\0\x01\0\0mi"),
                ItemInfoEntry::decode_payload(b"\x01\0\0\0\0\x01\0\0a\0b\0c\0fdel\0\0\0\0")
            ),
            (
                Err(Error::truncated_payload(12, 10)),
                Err(Error::truncated_payload(28, 22))
            )
        );
    }

    #[test]
    fn a_version_the_entry_does_not_read_is_refused_when_read_and_when_written() {
        let mut version_4 = entries().remove(4);
        version_4.version = 4;

        assert_eq!(
            (
                ItemInfoEntry::decode_payload(b"\x04\0\0\0"),
                version_4.encode(&mut [0; 64])
            ),
            (
                Err(Error::unsupported_version(4)),
                Err(Error::unsupported_version(4))
            )
        );
    }
}
