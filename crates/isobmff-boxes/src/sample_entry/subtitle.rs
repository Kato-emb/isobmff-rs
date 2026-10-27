//! [`XmlSubtitleSampleEntry`] and [`TextSubtitleSampleEntry`], the sample entries of a subtitle track

use alloc::vec::Vec;

use isobmff_core::{
    BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FourCc, OtherBoxes, Utf8List, Utf8String,
};

use crate::btrt::BitRateBox;
use crate::mime::MimeBox;
use crate::rinf::RestrictedSchemeInfoBox;
use crate::sample_entry::text::{MimeFormat, encode_remaining, remaining_len};
use crate::sample_entry::{
    AnyEntryBoxes, SampleEntry, children_len, encode_children, encode_optional, from_class,
    of_format, read_string, split_children,
};
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;
use crate::txtc::TextConfigBox;

/// Box types of the boxes ISO/IEC 14496-12 defines for [`XmlSubtitleSampleEntry`]
const DEFINED: [BoxType; 1] = [MimeBox::BOX_TYPE];

/// [`XmlSubtitleSampleEntry`] (`stpp`), ISO/IEC 14496-12 §12.6.3
///
/// The sample entry of a subtitle track whose samples are XML documents, a
/// `SubtitleSampleEntry`. The bytes of a [`SampleEntry`] after
/// `data_reference_index` are read with [`TryFrom`] and written back with
/// [`SampleEntry::try_from`].
///
/// A list field ends at its NUL, or at the end of the entry where it has none,
/// and is written with its NUL.
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// `mime`, then the boxes any sample entry may hold, `sinf`, `scrb`, `rinf`
/// and `btrt` in this order, which §12.6.3.3 lets `mime` and `btrt` lie in either
/// order among; and the boxes no definition of the class names, in
/// [`extra_boxes`](Self::extra_boxes). A child of a type this crate does not
/// read that lies before every box of the second part is read into the first
/// part, and one that lies after one of them into the third.
#[doc(alias = "stpp", alias = "XMLSubtitleSampleEntry")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct XmlSubtitleSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// XML namespaces the documents conform to, one or more
    pub namespace: Utf8List,
    /// URLs of the XML schemas the documents conform to, zero or more, as `xsi:schemaLocation` lists them where there is more than one namespace
    pub schema_location: Utf8List,
    /// Media types of the auxiliary resources, such as images and fonts, stored as sub-samples, given where there are such resources
    pub auxiliary_mime_types: Utf8List,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// MIME type of each document when delivered on its own
    pub mime: Option<MimeBox>,
    /// Protection schemes of a protected sample entry, alternative descriptions of the same protection
    pub sinf: Vec<ProtectionSchemeInfoBox>,
    /// Scrambling schemes applied to the samples
    pub scrb: Vec<ScrambleSchemeInfoBox>,
    /// Restriction of a restricted sample entry, or advisory information on the restriction of one whose type was not transformed
    pub rinf: Option<RestrictedSchemeInfoBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl XmlSubtitleSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"stpp");
}

impl TryFrom<&SampleEntry> for XmlSubtitleSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `stpp`.
    /// * The failures of a child that does not frame, and of a box of
    ///   ISO/IEC 14496-12 given more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        of_format(entry, Self::FORMAT)?;
        let mut reader = FieldReader::new(&entry.remaining_data);
        let namespace = Utf8List::from_slice(read_string(&mut reader)?);
        let schema_location = Utf8List::from_slice(read_string(&mut reader)?);
        let auxiliary_mime_types = Utf8List::from_slice(read_string(&mut reader)?);
        let (derived_boxes, mut children) = split_children(reader.take_remainder(), &DEFINED)?;

        Ok(Self {
            data_reference_index: entry.data_reference_index,
            namespace,
            schema_location,
            auxiliary_mime_types,
            derived_boxes,
            mime: children.take_zero_or_one_if_decoded()?,
            sinf: children.take_zero_or_more_if_decoded(),
            scrb: children.take_zero_or_more_if_decoded(),
            rinf: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

impl TryFrom<&XmlSubtitleSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `subtitle` into the bytes of an entry of the format `stpp` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String): a
    ///   list is not UTF-8.
    /// * The failures of writing a child.
    fn try_from(subtitle: &XmlSubtitleSampleEntry) -> Result<Self, Error> {
        let any_entry = AnyEntryBoxes {
            sinf: &subtitle.sinf,
            scrb: &subtitle.scrb,
            rinf: subtitle.rinf.as_ref(),
            btrt: subtitle.btrt.as_ref(),
        };
        let remaining_len = children_len(
            &subtitle.derived_boxes,
            [
                subtitle.mime.as_ref().map_or(0, BoxEncode::encoded_len),
                any_entry.encoded_len(),
            ],
            &subtitle.extra_boxes,
        )
        .saturating_add(subtitle.namespace.encoded_len())
        .saturating_add(subtitle.schema_location.encoded_len())
        .saturating_add(subtitle.auxiliary_mime_types.encoded_len());

        from_class(
            XmlSubtitleSampleEntry::FORMAT,
            subtitle.data_reference_index,
            remaining_len,
            |writer| {
                let mut rest = subtitle.namespace.encode(writer.take_remainder())?;
                rest = subtitle.schema_location.encode(rest)?;
                encode_children(
                    subtitle.auxiliary_mime_types.encode(rest)?,
                    &subtitle.derived_boxes,
                    |rest| any_entry.encode(encode_optional(rest, subtitle.mime.as_ref())?),
                    &subtitle.extra_boxes,
                )
            },
        )
    }
}

/// [`TextSubtitleSampleEntry`] (`sbtt`), ISO/IEC 14496-12 §12.6.3
///
/// The sample entry of a subtitle track whose samples are of the text format
/// a MIME type identifies, a `SubtitleSampleEntry`, laid out as a
/// [`SimpleTextSampleEntry`](crate::SimpleTextSampleEntry) is. The bytes of a
/// [`SampleEntry`] after `data_reference_index` are read with [`TryFrom`] and
/// written back with [`SampleEntry::try_from`].
#[doc(alias = "sbtt")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct TextSubtitleSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// MIME type of the content encoding of the subtitles, such as `application/zip`, empty where they are not encoded
    pub content_encoding: Utf8String,
    /// MIME type of the format of the samples, such as `text/html` or `text/plain`
    pub mime_format: Utf8String,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Text each document opens with
    pub txtc: Option<TextConfigBox>,
    /// Protection schemes of a protected sample entry, alternative descriptions of the same protection
    pub sinf: Vec<ProtectionSchemeInfoBox>,
    /// Scrambling schemes applied to the samples
    pub scrb: Vec<ScrambleSchemeInfoBox>,
    /// Restriction of a restricted sample entry, or advisory information on the restriction of one whose type was not transformed
    pub rinf: Option<RestrictedSchemeInfoBox>,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl TextSubtitleSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"sbtt");
}

impl TryFrom<&SampleEntry> for TextSubtitleSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `sbtt`.
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
            sinf: read.sinf,
            scrb: read.scrb,
            rinf: read.rinf,
            btrt: read.btrt,
            extra_boxes: read.extra_boxes,
        })
    }
}

impl TryFrom<&TextSubtitleSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `subtitle` into the bytes of an entry of the format `sbtt` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String): a
    ///   string is not UTF-8.
    /// * The failures of writing a child.
    fn try_from(subtitle: &TextSubtitleSampleEntry) -> Result<Self, Error> {
        let any_entry = AnyEntryBoxes {
            sinf: &subtitle.sinf,
            scrb: &subtitle.scrb,
            rinf: subtitle.rinf.as_ref(),
            btrt: subtitle.btrt.as_ref(),
        };

        from_class(
            TextSubtitleSampleEntry::FORMAT,
            subtitle.data_reference_index,
            remaining_len(
                &subtitle.content_encoding,
                &subtitle.mime_format,
                &subtitle.derived_boxes,
                subtitle.txtc.as_ref(),
                &any_entry,
                &subtitle.extra_boxes,
            ),
            |writer| {
                encode_remaining(
                    writer,
                    &subtitle.content_encoding,
                    &subtitle.mime_format,
                    &subtitle.derived_boxes,
                    subtitle.txtc.as_ref(),
                    &any_entry,
                    &subtitle.extra_boxes,
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{ChildBoxes, Error, FourCc, OtherBoxes, Utf8List, Utf8String};

    use super::{TextSubtitleSampleEntry, XmlSubtitleSampleEntry};
    use crate::btrt::BitRateBox;
    use crate::mime::MimeBox;
    use crate::sample_entry::SampleEntry;

    /// Lists of an entry of the namespace of TTML, no schema and the auxiliary resources of PNG images
    const LISTS: &[u8] = b"http://www.w3.org/ns/ttml\0\0image/png\0";

    /// Box of a type this crate does not read
    const UNKNOWN: &[u8] = b"\0\0\0\x09xtra\x07";

    /// `mime` of the type `application/ttml+xml`
    const MIME: &[u8] = b"\0\0\0\x21mime\0\0\0\0application/ttml+xml\0";

    /// `btrt` of a buffer of 4096 bytes, at most 2 Mbit/s and 1 Mbit/s on average
    const BTRT: &[u8] = b"\0\0\0\x14btrt\0\0\x10\0\0\x1e\x84\x80\0\x0f\x42\x40";

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

    fn xml(derived_boxes: &[&[u8]], extra_boxes: &[&[u8]]) -> XmlSubtitleSampleEntry {
        XmlSubtitleSampleEntry {
            data_reference_index: 1,
            namespace: Utf8List::from_slice(b"http://www.w3.org/ns/ttml"),
            auxiliary_mime_types: Utf8List::from_slice(b"image/png"),
            derived_boxes: kept(derived_boxes),
            mime: Some(MimeBox {
                content_type: Utf8String::from_slice(b"application/ttml+xml"),
            }),
            btrt: Some(BitRateBox {
                buffer_size_db: 4096,
                max_bitrate: 2_000_000,
                avg_bitrate: 1_000_000,
            }),
            extra_boxes: kept(extra_boxes),
            ..XmlSubtitleSampleEntry::default()
        }
    }

    #[test]
    fn reads_its_lists_then_its_boxes_split_by_where_they_lie() {
        assert_eq!(
            XmlSubtitleSampleEntry::try_from(&sample_entry(
                b"stpp",
                &[LISTS, UNKNOWN, BTRT, UNKNOWN, MIME]
            )),
            Ok(xml(&[UNKNOWN], &[UNKNOWN]))
        );
    }

    #[test]
    fn writes_its_lists_then_its_boxes_in_the_order_of_its_class() {
        assert_eq!(
            SampleEntry::try_from(&xml(&[UNKNOWN], &[UNKNOWN])),
            Ok(sample_entry(
                b"stpp",
                &[LISTS, UNKNOWN, MIME, BTRT, UNKNOWN]
            ))
        );
    }

    #[test]
    fn lists_missing_at_the_end_of_the_entry_read_as_empty() {
        let expected = XmlSubtitleSampleEntry {
            data_reference_index: 1,
            namespace: Utf8List::from_slice(b"http://www.w3.org/ns/ttml"),
            ..XmlSubtitleSampleEntry::default()
        };

        assert_eq!(
            XmlSubtitleSampleEntry::try_from(&sample_entry(
                b"stpp",
                &[b"http://www.w3.org/ns/ttml\0"]
            )),
            Ok(expected)
        );
    }

    #[test]
    fn a_text_subtitle_entry_reads_and_writes_back_its_strings() {
        let entry = sample_entry(b"sbtt", &[b"\0text/vtt\0"]);
        let subtitle = TextSubtitleSampleEntry {
            data_reference_index: 1,
            mime_format: Utf8String::from_slice(b"text/vtt"),
            ..TextSubtitleSampleEntry::default()
        };

        assert_eq!(
            (
                TextSubtitleSampleEntry::try_from(&entry),
                SampleEntry::try_from(&subtitle)
            ),
            (Ok(subtitle.clone()), Ok(entry))
        );
    }

    #[test]
    fn an_entry_of_another_format_is_refused() {
        assert_eq!(
            (
                XmlSubtitleSampleEntry::try_from(&sample_entry(b"sbtt", &[LISTS])),
                TextSubtitleSampleEntry::try_from(&sample_entry(b"stpp", &[LISTS]))
            ),
            (
                Err(Error::unsupported_value("format")),
                Err(Error::unsupported_value("format"))
            )
        );
    }
}
