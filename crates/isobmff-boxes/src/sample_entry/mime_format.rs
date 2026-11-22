//! [`MimeFormat`], the fields and boxes of the sample entries that identify the format of their samples by a MIME type

use alloc::vec::Vec;

use isobmff_core::{
    BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, OtherBoxes, Utf8String,
};

use super::children::{
    AnyEntryBoxes, children_len, encode_children, encode_optional, split_children,
};
use super::entry::{SampleEntry, read_string};
use crate::btrt::BitRateBox;
use crate::rinf::RestrictedSchemeInfoBox;
use crate::scrb::ScrambleSchemeInfoBox;
use crate::sinf::ProtectionSchemeInfoBox;
use crate::txtc::TextConfigBox;

/// Box types of the boxes ISO/IEC 14496-12 defines for a class that names a MIME type
const DEFINED: [BoxType; 1] = [TextConfigBox::BOX_TYPE];

/// Fields and boxes the classes share that identify the format of their samples by a MIME type: [`SimpleTextSampleEntry`](crate::SimpleTextSampleEntry), [`TextMetaDataSampleEntry`](crate::TextMetaDataSampleEntry) and [`TextSubtitleSampleEntry`](crate::TextSubtitleSampleEntry)
pub(super) struct MimeFormat {
    pub(super) content_encoding: Utf8String,
    pub(super) mime_format: Utf8String,
    pub(super) derived_boxes: OtherBoxes,
    pub(super) txtc: Option<TextConfigBox>,
    pub(super) sinf: Vec<ProtectionSchemeInfoBox>,
    pub(super) scrb: Vec<ScrambleSchemeInfoBox>,
    pub(super) rinf: Option<RestrictedSchemeInfoBox>,
    pub(super) btrt: Option<BitRateBox>,
    pub(super) extra_boxes: OtherBoxes,
}

impl MimeFormat {
    /// Reads the fields and the boxes from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of a child that does not frame, and of a box of
    ///   ISO/IEC 14496-12 given more than once.
    pub(super) fn read(entry: &SampleEntry) -> Result<Self, Error> {
        let mut reader = FieldReader::new(&entry.remaining_data);
        let content_encoding = Utf8String::from_slice(read_string(&mut reader)?);
        let mime_format = Utf8String::from_slice(read_string(&mut reader)?);
        let (derived_boxes, mut children) = split_children(reader.take_remainder(), &DEFINED)?;

        Ok(Self {
            content_encoding,
            mime_format,
            derived_boxes,
            txtc: children.take_zero_or_one_if_decoded()?,
            sinf: children.take_zero_or_more_if_decoded(),
            scrb: children.take_zero_or_more_if_decoded(),
            rinf: children.take_zero_or_one_if_decoded()?,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

/// Returns the length of the bytes a class that names a MIME type lays out after `data_reference_index`
pub(super) fn remaining_len(
    content_encoding: &Utf8String,
    mime_format: &Utf8String,
    derived_boxes: &OtherBoxes,
    txtc: Option<&TextConfigBox>,
    any_entry: &AnyEntryBoxes<'_>,
    extra_boxes: &OtherBoxes,
) -> u64 {
    children_len(
        derived_boxes,
        [
            txtc.map_or(0, BoxEncode::encoded_len),
            any_entry.encoded_len(),
        ],
        extra_boxes,
    )
    .saturating_add(content_encoding.encoded_len())
    .saturating_add(mime_format.encoded_len())
}

/// Writes the bytes a class that names a MIME type lays out after `data_reference_index`
///
/// # Errors
///
/// * [`InvalidUtf8String`](isobmff_core::ErrorKind::InvalidUtf8String): a
///   string is not UTF-8.
/// * The failures of writing a child.
pub(super) fn encode_remaining(
    writer: &mut FieldWriter<'_>,
    content_encoding: &Utf8String,
    mime_format: &Utf8String,
    derived_boxes: &OtherBoxes,
    txtc: Option<&TextConfigBox>,
    any_entry: &AnyEntryBoxes<'_>,
    extra_boxes: &OtherBoxes,
) -> Result<(), Error> {
    let rest = content_encoding.encode(writer.take_remainder())?;
    encode_children(
        mime_format.encode(rest)?,
        derived_boxes,
        |rest| any_entry.encode(encode_optional(rest, txtc)?),
        extra_boxes,
    )
}
