//! [`SimpleTextSampleEntry`], the sample entry of a timed text track

use alloc::vec::Vec;

use isobmff_core::{Error, FourCc, OtherBoxes, Utf8String};

use super::children::AnyEntryBoxes;
use super::entry::{SampleEntry, from_class, of_format};
use super::mime_format::{MimeFormat, encode_remaining, remaining_len};
use crate::btrt::BitRateBox;
use crate::rinf::RestrictedSchemeInfoBox;
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;
use crate::txtc::TextConfigBox;

/// [`SimpleTextSampleEntry`] (`stxt`), ISO/IEC 14496-12 §12.5.3
///
/// The sample entry of a timed text track whose samples are of the format a
/// MIME type identifies, a `PlainTextSampleEntry`. The bytes of a
/// [`SampleEntry`] after `data_reference_index` are read with [`TryFrom`] and
/// written back with [`SampleEntry::try_from`].
///
/// A string field ends at its NUL, or at the end of the entry where it has
/// none, and is written with its NUL.
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// `txtC`, then the boxes any sample entry may hold, `sinf`, `scrb`, `rinf`
/// and `btrt` in this order; and the boxes no definition of the class names, in
/// [`extra_boxes`](Self::extra_boxes). A child of a type this crate does not
/// read that lies before every box of the second part is read into the first
/// part, and one that lies after one of them into the third.
#[doc(alias = "stxt")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SimpleTextSampleEntry {
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// MIME type of the content encoding of the text, such as `application/zip`, empty where the text is not encoded
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

impl SimpleTextSampleEntry {
    /// Format of the class, which the box type of its entries is
    pub const FORMAT: FourCc = FourCc::new(*b"stxt");
}

impl TryFrom<&SampleEntry> for SimpleTextSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
    ///   `format`: the entry is of a format other than `stxt`.
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

impl TryFrom<&SimpleTextSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `text` into the bytes of an entry of the format `stxt` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String): a
    ///   string is not UTF-8.
    /// * The failures of writing a child.
    fn try_from(text: &SimpleTextSampleEntry) -> Result<Self, Error> {
        let any_entry = AnyEntryBoxes {
            sinf: &text.sinf,
            scrb: &text.scrb,
            rinf: text.rinf.as_ref(),
            btrt: text.btrt.as_ref(),
        };

        from_class(
            SimpleTextSampleEntry::FORMAT,
            text.data_reference_index,
            remaining_len(
                &text.content_encoding,
                &text.mime_format,
                &text.derived_boxes,
                text.txtc.as_ref(),
                &any_entry,
                &text.extra_boxes,
            ),
            |writer| {
                encode_remaining(
                    writer,
                    &text.content_encoding,
                    &text.mime_format,
                    &text.derived_boxes,
                    text.txtc.as_ref(),
                    &any_entry,
                    &text.extra_boxes,
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{ChildBoxes, Error, FourCc, OtherBoxes, Utf8String};

    use super::SimpleTextSampleEntry;
    use crate::btrt::BitRateBox;
    use crate::sample_entry::SampleEntry;
    use crate::txtc::TextConfigBox;

    /// Fields of an entry of no content encoding and the format `text/plain`
    const FIELDS: &[u8] = b"\0text/plain\0";

    /// Box of a type this crate does not read
    const UNKNOWN: &[u8] = b"\0\0\0\x09xtra\x07";

    /// `txtC` of the text `#`
    const TXTC: &[u8] = b"\0\0\0\x0etxtC\0\0\0\0#\0";

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

    fn text(derived_boxes: &[&[u8]], extra_boxes: &[&[u8]]) -> SimpleTextSampleEntry {
        SimpleTextSampleEntry {
            data_reference_index: 1,
            mime_format: Utf8String::from_slice(b"text/plain"),
            derived_boxes: kept(derived_boxes),
            txtc: Some(TextConfigBox {
                text_config: Utf8String::from_slice(b"#"),
            }),
            btrt: Some(BitRateBox {
                buffer_size_db: 4096,
                max_bitrate: 2_000_000,
                avg_bitrate: 1_000_000,
            }),
            extra_boxes: kept(extra_boxes),
            ..SimpleTextSampleEntry::default()
        }
    }

    #[test]
    fn reads_its_strings_then_its_boxes_split_by_where_they_lie() {
        assert_eq!(
            SimpleTextSampleEntry::try_from(&sample_entry(
                b"stxt",
                &[FIELDS, UNKNOWN, BTRT, UNKNOWN, TXTC]
            )),
            Ok(text(&[UNKNOWN], &[UNKNOWN]))
        );
    }

    #[test]
    fn writes_its_strings_then_its_boxes_in_the_order_of_its_class() {
        assert_eq!(
            SampleEntry::try_from(&text(&[UNKNOWN], &[UNKNOWN])),
            Ok(sample_entry(
                b"stxt",
                &[FIELDS, UNKNOWN, TXTC, BTRT, UNKNOWN]
            ))
        );
    }

    #[test]
    fn a_string_without_its_terminator_runs_to_the_end_of_the_entry() {
        let expected = SimpleTextSampleEntry {
            data_reference_index: 1,
            mime_format: Utf8String::from_slice(b"text/plain"),
            ..SimpleTextSampleEntry::default()
        };

        assert_eq!(
            SimpleTextSampleEntry::try_from(&sample_entry(b"stxt", &[b"\0text/plain"])),
            Ok(expected)
        );
    }

    #[test]
    fn an_entry_of_another_format_is_refused() {
        assert_eq!(
            SimpleTextSampleEntry::try_from(&sample_entry(b"tx3g", &[FIELDS])),
            Err(Error::unsupported_value("format"))
        );
    }
}
