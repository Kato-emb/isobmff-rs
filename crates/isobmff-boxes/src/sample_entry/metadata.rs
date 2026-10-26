//! The sample entries of a timed metadata track: [`XmlMetaDataSampleEntry`], [`TextMetaDataSampleEntry`], [`UriMetaSampleEntry`] and [`BoxedMetadataSampleEntry`]

use isobmff_core::{
    BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FourCc, OtherBoxes, Utf8List, Utf8String,
};

use crate::btrt::BitRateBox;
use crate::keys::MetadataKeyTableBox;
use crate::sample_entry::text::{MimeFormat, encode_remaining, remaining_len};
use crate::sample_entry::{
    BitRateChildren, SampleEntry, children_len, encode_children, encode_optional, from_class,
    of_format, read_string, split_children,
};
use crate::txtc::TextConfigBox;
use crate::uri::UriBox;
use crate::urii::UriInitBox;

/// Box types of the boxes ISO/IEC 14496-12 defines for [`UriMetaSampleEntry`]
const URI_DEFINED: [BoxType; 3] = [UriBox::BOX_TYPE, UriInitBox::BOX_TYPE, BitRateBox::BOX_TYPE];

/// Box types of the boxes ISO/IEC 14496-12 defines for [`BoxedMetadataSampleEntry`]
const BOXED_DEFINED: [BoxType; 2] = [MetadataKeyTableBox::BOX_TYPE, BitRateBox::BOX_TYPE];

/// [`XmlMetaDataSampleEntry`] (`metx`), ISO/IEC 14496-12 §12.3.3
///
/// The sample entry of a timed metadata track whose samples are XML
/// documents, a `MetaDataSampleEntry`. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// A string or list field ends at its NUL, or at the end of the entry where it
/// has none, and is written with its NUL.
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// `btrt`, which can choose the memory representation of the documents; and
/// the boxes no definition of the class names, in
/// [`extra_boxes`](Self::extra_boxes). A child of a type this crate does not
/// read that lies before `btrt` is read into the first part, and one that lies
/// after it into the third.
#[doc(alias = "metx", alias = "XMLMetaDataSampleEntry")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct XmlMetaDataSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// MIME type of the content encoding of the metadata, such as `application/zip`, empty where it is not encoded
    pub content_encoding: Utf8String,
    /// XML namespaces the documents conform to, one or more
    pub namespace: Utf8List,
    /// URLs of the XML schemas the documents conform to, zero or more, as `xsi:schemaLocation` lists them where there is more than one namespace
    pub schema_location: Utf8List,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl XmlMetaDataSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"metx");
}

impl TryFrom<&SampleEntry> for XmlMetaDataSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `metx`.
    /// * The failures of a child that does not frame, and of a `btrt` given
    ///   more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        of_format(entry, Self::FORMAT)?;
        let mut reader = FieldReader::new(&entry.remaining_data);
        let content_encoding = Utf8String::from_slice(read_string(&mut reader)?);
        let namespace = Utf8List::from_slice(read_string(&mut reader)?);
        let schema_location = Utf8List::from_slice(read_string(&mut reader)?);
        let children = BitRateChildren::read(reader.take_remainder())?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            content_encoding,
            namespace,
            schema_location,
            derived_boxes: children.derived_boxes,
            btrt: children.btrt,
            extra_boxes: children.extra_boxes,
        })
    }
}

impl TryFrom<&XmlMetaDataSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `metadata` into the bytes of an entry of the format `metx` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String): a
    ///   string or a list is not UTF-8.
    /// * The failures of writing a child.
    fn try_from(metadata: &XmlMetaDataSampleEntry) -> Result<Self, Error> {
        let remaining_len = children_len(
            &metadata.derived_boxes,
            metadata.btrt.as_ref().map(BoxEncode::encoded_len),
            &metadata.extra_boxes,
        )
        .saturating_add(metadata.content_encoding.encoded_len())
        .saturating_add(metadata.namespace.encoded_len())
        .saturating_add(metadata.schema_location.encoded_len());

        from_class(
            XmlMetaDataSampleEntry::FORMAT,
            metadata.data_reference_index,
            remaining_len,
            |writer| {
                let mut rest = metadata.content_encoding.encode(writer.take_remainder())?;
                rest = metadata.namespace.encode(rest)?;
                rest = metadata.schema_location.encode(rest)?;
                encode_children(
                    rest,
                    &metadata.derived_boxes,
                    |rest| encode_optional(rest, metadata.btrt.as_ref()),
                    &metadata.extra_boxes,
                )
            },
        )
    }
}

/// [`TextMetaDataSampleEntry`] (`mett`), ISO/IEC 14496-12 §12.3.3
///
/// The sample entry of a timed metadata track whose samples are of the text
/// format a MIME type identifies, a `MetaDataSampleEntry`, laid out as a
/// [`SimpleTextSampleEntry`](crate::SimpleTextSampleEntry) is. The bytes of a
/// [`SampleEntry`] after `data_reference_index` are read with [`TryFrom`] and
/// written back with [`SampleEntry::try_from`].
#[doc(alias = "mett")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct TextMetaDataSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// MIME type of the content encoding of the metadata, such as `application/zip`, empty where it is not encoded
    pub content_encoding: Utf8String,
    /// MIME type of the format of the samples, such as `text/html` or `text/plain`
    pub mime_format: Utf8String,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Text each document opens with
    pub txtc: Option<TextConfigBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl TextMetaDataSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"mett");
}

impl TryFrom<&SampleEntry> for TextMetaDataSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `mett`.
    /// * The failures of a child that does not frame, and of a box of
    ///   ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        of_format(entry, Self::FORMAT)?;
        let read = MimeFormat::read(entry)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            content_encoding: read.content_encoding,
            mime_format: read.mime_format,
            derived_boxes: read.derived_boxes,
            txtc: read.txtc,
            btrt: read.btrt,
            extra_boxes: read.extra_boxes,
        })
    }
}

impl TryFrom<&TextMetaDataSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `metadata` into the bytes of an entry of the format `mett` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String): a
    ///   string is not UTF-8.
    /// * The failures of writing a child.
    fn try_from(metadata: &TextMetaDataSampleEntry) -> Result<Self, Error> {
        from_class(
            TextMetaDataSampleEntry::FORMAT,
            metadata.data_reference_index,
            remaining_len(
                &metadata.content_encoding,
                &metadata.mime_format,
                &metadata.derived_boxes,
                metadata.txtc.as_ref(),
                metadata.btrt.as_ref(),
                &metadata.extra_boxes,
            ),
            |writer| {
                encode_remaining(
                    writer,
                    &metadata.content_encoding,
                    &metadata.mime_format,
                    &metadata.derived_boxes,
                    metadata.txtc.as_ref(),
                    metadata.btrt.as_ref(),
                    &metadata.extra_boxes,
                )
            },
        )
    }
}

/// [`UriMetaSampleEntry`] (`urim`), ISO/IEC 14496-12 §12.3.3
///
/// The sample entry of a timed metadata track of the form a URI defines, a
/// `MetaDataSampleEntry`, with the initialization data of the form. The bytes
/// of a [`SampleEntry`] after `data_reference_index` are read with [`TryFrom`]
/// and written back with [`SampleEntry::try_from`].
///
/// # Children
///
/// The children are laid out in three parts as those of a
/// [`XmlMetaDataSampleEntry`] are, the boxes of ISO/IEC 14496-12 being the
/// `uri ` the entry holds exactly one of, then `uriI` and `btrt`.
#[doc(alias = "urim", alias = "URIMetaSampleEntry")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct UriMetaSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// URI that defines the form of the metadata
    pub the_label: UriBox,
    /// Initialization data of the form
    pub init: Option<UriInitBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl UriMetaSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"urim");
}

impl TryFrom<&SampleEntry> for UriMetaSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `urim`.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox):
    ///   the entry holds no `uri `.
    /// * The failures of a child that does not frame, of a `uri ` that does
    ///   not read, and of a box of ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        of_format(entry, Self::FORMAT)?;
        let (derived_boxes, mut children) = split_children(&entry.remaining_data, &URI_DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            derived_boxes,
            the_label: children.take_exactly_one()?,
            init: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&UriMetaSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `metadata` into the bytes of an entry of the format `urim` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(metadata: &UriMetaSampleEntry) -> Result<Self, Error> {
        let remaining_len = children_len(
            &metadata.derived_boxes,
            [
                metadata.the_label.encoded_len(),
                metadata.init.as_ref().map_or(0, BoxEncode::encoded_len),
                metadata.btrt.as_ref().map_or(0, BoxEncode::encoded_len),
            ],
            &metadata.extra_boxes,
        );

        from_class(
            UriMetaSampleEntry::FORMAT,
            metadata.data_reference_index,
            remaining_len,
            |writer| {
                encode_children(
                    writer.take_remainder(),
                    &metadata.derived_boxes,
                    |rest| {
                        let rest = metadata.the_label.encode(rest)?;
                        encode_optional(
                            encode_optional(rest, metadata.init.as_ref())?,
                            metadata.btrt.as_ref(),
                        )
                    },
                    &metadata.extra_boxes,
                )
            },
        )
    }
}

/// [`BoxedMetadataSampleEntry`] (`mebx`), ISO/IEC 14496-12 §12.8.4
///
/// The sample entry of a multiplexed timed metadata track, whose samples are
/// value boxes of the keys its `keys` declares, a `MetaDataSampleEntry`. The
/// bytes of a [`SampleEntry`] after `data_reference_index` are read with
/// [`TryFrom`] and written back with [`SampleEntry::try_from`].
///
/// # Children
///
/// The children are laid out in three parts as those of a
/// [`XmlMetaDataSampleEntry`] are, the boxes of ISO/IEC 14496-12 being the
/// `keys` the entry holds exactly one of, then `btrt`.
#[doc(alias = "mebx")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct BoxedMetadataSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Keys the samples may hold values of
    pub keys: MetadataKeyTableBox,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl BoxedMetadataSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"mebx");
}

impl TryFrom<&SampleEntry> for BoxedMetadataSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `mebx`.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox):
    ///   the entry holds no `keys`.
    /// * The failures of a child that does not frame, of a `keys` that does
    ///   not read, and of a box of ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        of_format(entry, Self::FORMAT)?;
        let (derived_boxes, mut children) = split_children(&entry.remaining_data, &BOXED_DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            derived_boxes,
            keys: children.take_exactly_one()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&BoxedMetadataSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `metadata` into the bytes of an entry of the format `mebx` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(metadata: &BoxedMetadataSampleEntry) -> Result<Self, Error> {
        from_class(
            BoxedMetadataSampleEntry::FORMAT,
            metadata.data_reference_index,
            children_len(
                &metadata.derived_boxes,
                [
                    metadata.keys.encoded_len(),
                    metadata.btrt.as_ref().map_or(0, BoxEncode::encoded_len),
                ],
                &metadata.extra_boxes,
            ),
            |writer| {
                encode_children(
                    writer.take_remainder(),
                    &metadata.derived_boxes,
                    |rest| encode_optional(metadata.keys.encode(rest)?, metadata.btrt.as_ref()),
                    &metadata.extra_boxes,
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxType, ChildBoxes, Error, FourCc, OtherBoxes, Utf8List,
        Utf8String,
    };

    use super::{
        BoxedMetadataSampleEntry, TextMetaDataSampleEntry, UriMetaSampleEntry,
        XmlMetaDataSampleEntry,
    };
    use crate::btrt::BitRateBox;
    use crate::keys::MetadataKeyTableBox;
    use crate::sample_entry::SampleEntry;
    use crate::uri::UriBox;
    use crate::urii::UriInitBox;

    /// Box of a type this crate does not read
    const UNKNOWN: &[u8] = b"\0\0\0\x09xtra\x07";

    /// `keys` of the key `cprt`, of the namespace `uiso` and the user-data key `cprt`
    const KEYS: &[u8] = b"\0\0\0\x20keys\0\0\0\x18cprt\0\0\0\x10keyduisocprt";

    /// `btrt` of a buffer of 4096 bytes, at most 2 Mbit/s and 1 Mbit/s on average
    const BTRT: &[u8] = b"\0\0\0\x14btrt\0\0\x10\0\0\x1e\x84\x80\0\x0f\x42\x40";

    /// `uri ` of `urn:example:metadata`
    const URI: &[u8] = b"\0\0\0\x21uri \0\0\0\0urn:example:metadata\0";

    /// `uriI` of three bytes of initialization data
    const URII: &[u8] = b"\0\0\0\x0furiI\0\0\0\0\x01\x02\x03";

    fn sample_entry(format: &[u8; 4], remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*format),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    fn kept(children: &[&[u8]]) -> OtherBoxes {
        OtherBoxes::from(ChildBoxes::decode(&children.concat()).unwrap())
    }

    fn bit_rate() -> Option<BitRateBox> {
        Some(BitRateBox {
            buffer_size_db: 4096,
            max_bitrate: 2_000_000,
            avg_bitrate: 1_000_000,
        })
    }

    fn xml() -> XmlMetaDataSampleEntry {
        XmlMetaDataSampleEntry {
            data_reference_index: 1,
            namespace: Utf8List::from_slice(b"urn:example:a urn:example:b"),
            derived_boxes: kept(&[UNKNOWN]),
            btrt: bit_rate(),
            ..XmlMetaDataSampleEntry::default()
        }
    }

    fn uri() -> UriMetaSampleEntry {
        UriMetaSampleEntry {
            data_reference_index: 1,
            the_label: UriBox {
                the_uri: Utf8String::from_slice(b"urn:example:metadata"),
            },
            init: Some(UriInitBox {
                uri_initialization_data: alloc::vec![1, 2, 3],
            }),
            extra_boxes: kept(&[UNKNOWN]),
            ..UriMetaSampleEntry::default()
        }
    }

    #[test]
    fn an_xml_entry_reads_and_writes_back_its_strings_and_lists() {
        let entry = sample_entry(
            b"metx",
            &[b"\0urn:example:a urn:example:b\0\0", UNKNOWN, BTRT],
        );

        assert_eq!(
            (
                XmlMetaDataSampleEntry::try_from(&entry),
                SampleEntry::try_from(&xml())
            ),
            (Ok(xml()), Ok(entry))
        );
    }

    #[test]
    fn a_text_entry_reads_and_writes_back_its_strings() {
        let entry = sample_entry(b"mett", &[b"application/zip\0text/plain\0"]);
        let metadata = TextMetaDataSampleEntry {
            data_reference_index: 1,
            content_encoding: Utf8String::from_slice(b"application/zip"),
            mime_format: Utf8String::from_slice(b"text/plain"),
            ..TextMetaDataSampleEntry::default()
        };

        assert_eq!(
            (
                TextMetaDataSampleEntry::try_from(&entry),
                SampleEntry::try_from(&metadata)
            ),
            (Ok(metadata.clone()), Ok(entry))
        );
    }

    #[test]
    fn a_uri_entry_writes_its_uri_then_its_initialization_data_whatever_order_they_were_read_in() {
        let read =
            UriMetaSampleEntry::try_from(&sample_entry(b"urim", &[URII, UNKNOWN, URI])).unwrap();

        assert_eq!(
            (SampleEntry::try_from(&read), read),
            (Ok(sample_entry(b"urim", &[URI, URII, UNKNOWN])), uri())
        );
    }

    #[test]
    fn a_uri_entry_without_its_uri_is_refused() {
        assert_eq!(
            UriMetaSampleEntry::try_from(&sample_entry(b"urim", &[URII])).map(|_| ()),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"uri ")))
        );
    }

    #[test]
    fn a_boxed_entry_writes_its_keys_then_its_bit_rate_whatever_order_they_were_read_in() {
        let metadata = BoxedMetadataSampleEntry {
            data_reference_index: 1,
            keys: MetadataKeyTableBox::decode(KEYS).unwrap().0,
            btrt: bit_rate(),
            ..BoxedMetadataSampleEntry::default()
        };
        let read = BoxedMetadataSampleEntry::try_from(&sample_entry(b"mebx", &[BTRT, KEYS]));

        assert_eq!(
            (read, SampleEntry::try_from(&metadata)),
            (
                Ok(metadata.clone()),
                Ok(sample_entry(b"mebx", &[KEYS, BTRT]))
            )
        );
    }

    #[test]
    fn a_boxed_entry_without_its_keys_is_refused() {
        assert_eq!(
            BoxedMetadataSampleEntry::try_from(&sample_entry(b"mebx", &[BTRT])).map(|_| ()),
            Err(Error::missing_mandatory_box(MetadataKeyTableBox::BOX_TYPE))
        );
    }

    #[test]
    fn an_entry_of_another_format_is_refused() {
        assert_eq!(
            XmlMetaDataSampleEntry::try_from(&sample_entry(b"mett", &[])),
            Err(Error::unsupported_value("format"))
        );
    }
}
