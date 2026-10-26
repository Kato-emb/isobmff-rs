//! [`SampleEntry`], the fields every sample entry opens with, and the bytes the class derived for its format lays out

mod audio;
mod audio_v1;
mod font;
mod haptic;
mod metadata;
mod subtitle;
mod text;
mod visual;
mod volumetric;

use alloc::vec;
use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader, FieldWriter, FourCc,
    OtherBoxes, RawBox,
};

use crate::btrt::BitRateBox;

pub use audio::{AudioSampleEntry, AudioSampleEntryVariant};
pub use audio_v1::AudioSampleEntryV1;
pub use font::FontSampleEntry;
pub use haptic::HapticSampleEntry;
pub use metadata::{
    BoxedMetadataSampleEntry, TextMetaDataSampleEntry, UriMetaSampleEntry, XmlMetaDataSampleEntry,
};
pub use subtitle::{TextSubtitleSampleEntry, XmlSubtitleSampleEntry};
pub use text::SimpleTextSampleEntry;
pub use visual::VisualSampleEntry;
pub use volumetric::VolumetricVisualSampleEntry;

/// [`SampleEntry`], ISO/IEC 14496-12 §8.5.2
///
/// One entry of a [`SampleDescriptionBox`](crate::SampleDescriptionBox): the
/// fields of the abstract class every sample entry derives from, and the
/// bytes after them as found. The class that lays those bytes out depends on
/// both the [`format`](Self::format) and the handler type of the track, so
/// the entry is read into that class by a caller who knows the handler type,
/// and written back from it.
///
/// The classes ISO/IEC 14496-12 defines, by the handler type of the track:
///
/// * `vide`: [`VisualSampleEntry`].
/// * `soun`: [`AudioSampleEntry`] or [`AudioSampleEntryV1`], which
///   [`AudioSampleEntryVariant`] reads by their `entry_version`.
/// * `meta`: [`XmlMetaDataSampleEntry`] (`metx`),
///   [`TextMetaDataSampleEntry`] (`mett`), [`UriMetaSampleEntry`] (`urim`) and
///   [`BoxedMetadataSampleEntry`] (`mebx`).
/// * `text`: [`SimpleTextSampleEntry`] (`stxt`).
/// * `subt`: [`XmlSubtitleSampleEntry`] (`stpp`) and
///   [`TextSubtitleSampleEntry`] (`sbtt`).
/// * `fdsm`: [`FontSampleEntry`].
/// * `volv`: [`VolumetricVisualSampleEntry`].
/// * `hapt`: [`HapticSampleEntry`].
///
/// A class of a fixed format reads only an entry of that format. A class
/// that takes a coding name, such as [`VisualSampleEntry`], reads an entry of
/// any format and keeps it.
///
/// The format is the box type of the entry, which a
/// [`SampleDescriptionBox`](crate::SampleDescriptionBox) reads into
/// [`format`](Self::format) and writes from it, so an entry of any format,
/// registered or not, reads. The six `reserved` bytes are read past and
/// written as zero.
///
/// The entry has no box type of its own, so it implements neither
/// [`BoxDecode`](isobmff_core::BoxDecode) nor
/// [`BoxDefinition`](isobmff_core::BoxDefinition): its [`BoxEncode`] writes
/// its payload, and a [`SampleDescriptionBox`](crate::SampleDescriptionBox)
/// writes the header.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleEntry {
    /// Format of the samples the entry describes, such as `avc1` or `mp4a`
    ///
    /// It is written as the box type, so a `format` of `uuid` is refused with
    /// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `format`
    /// when the entry is written.
    pub format: FourCc,
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) in the `dref` of the track from which the samples the entry describes are retrieved
    pub data_reference_index: u16,
    /// Bytes after `data_reference_index`, which the class derived for the format and the handler type lays out
    pub remaining_data: Vec<u8>,
}

impl Default for SampleEntry {
    fn default() -> Self {
        Self {
            format: FourCc::new([0; 4]),
            data_reference_index: 0,
            remaining_data: Vec::new(),
        }
    }
}

impl SampleEntry {
    /// Reads the payload of a box of the box type `format`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside `data_reference_index`.
    pub(crate) fn decode_payload(format: FourCc, payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);
        reader.read_bytes::<6>()?;

        Ok(Self {
            format,
            data_reference_index: reader.read_u16()?,
            remaining_data: reader.take_remainder().to_vec(),
        })
    }
}

/// Returns the entry whose bytes after `data_reference_index`, `remaining_len` of them, `encode_remaining` writes, as a class derived from [`SampleEntry`] lays them out
///
/// # Errors
///
/// * [`TrailingBuffer`](isobmff_core::ErrorKind::TrailingBuffer) or
///   [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): what
///   `encode_remaining` writes is not `remaining_len` long.
/// * The failures of `encode_remaining`.
pub(crate) fn from_class(
    format: FourCc,
    data_reference_index: u16,
    remaining_len: u64,
    encode_remaining: impl FnOnce(&mut FieldWriter<'_>) -> Result<(), Error>,
) -> Result<SampleEntry, Error> {
    let mut remaining_data = vec![0; usize::try_from(remaining_len).unwrap_or(usize::MAX)];
    let mut writer = FieldWriter::new(&mut remaining_data);
    encode_remaining(&mut writer)?;
    writer.finish()?;

    Ok(SampleEntry {
        format,
        data_reference_index,
        remaining_data,
    })
}

/// Splits the children of an entry before the first child of a type in `defined`, the boxes its class defines
///
/// Returns the children before it, which are the boxes of a derived
/// specification where the entry lays its children out in the order its
/// class does, and the children from it on, from which the class takes its
/// boxes and keeps the rest as the extra boxes.
///
/// # Errors
///
/// * The failures of [`ChildBoxes::decode`]: a child does not frame.
pub(crate) fn split_children<'payload>(
    payload: &'payload [u8],
    defined: &[BoxType],
) -> Result<(OtherBoxes, ChildBoxes<'payload>), Error> {
    let mut rest = payload;
    while !rest.is_empty() {
        let (child, after) = RawBox::split_first(rest)?;
        if defined.contains(&child.header().box_type()) {
            break;
        }
        rest = after;
    }
    let (derived, defined_onward) = payload.split_at(payload.len().saturating_sub(rest.len()));

    Ok((
        OtherBoxes::from(ChildBoxes::decode(derived)?),
        ChildBoxes::decode(defined_onward)?,
    ))
}

/// Checks that `entry` is of the format `format`, the format of a class that takes no coding name
///
/// # Errors
///
/// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
///   `format`: the entry is of another format.
pub(crate) fn of_format(entry: &SampleEntry, format: FourCc) -> Result<(), Error> {
    if entry.format != format {
        return Err(Error::unsupported_value("format"));
    }

    Ok(())
}

/// Reads a string field of an entry, which a NUL ends, or the end of the bytes where none does, and returns its bytes, the NUL included
pub(crate) fn read_string<'payload>(
    reader: &mut FieldReader<'payload>,
) -> Result<&'payload [u8], Error> {
    let rest = reader.remainder();
    let length = rest
        .iter()
        .position(|byte| *byte == 0)
        .map_or(rest.len(), |terminator| terminator.saturating_add(1));

    reader.read_slice(length)
}

/// Children of a class that defines no box of its own, which holds a `btrt` alone, as any sample entry may
pub(crate) struct BitRateChildren {
    pub(crate) derived_boxes: OtherBoxes,
    pub(crate) btrt: Option<BitRateBox>,
    pub(crate) extra_boxes: OtherBoxes,
}

impl BitRateChildren {
    /// Reads the children `payload` holds, split before the `btrt` as [`split_children`] splits them
    ///
    /// # Errors
    ///
    /// * The failures of a child that does not frame, and of a `btrt` given
    ///   more than once.
    pub(crate) fn read(payload: &[u8]) -> Result<Self, Error> {
        let (derived_boxes, mut children) = split_children(payload, &[BitRateBox::BOX_TYPE])?;

        Ok(Self {
            derived_boxes,
            btrt: children.take_zero_or_one_if_decoded()?,
            extra_boxes: OtherBoxes::from(children),
        })
    }
}

/// Returns the length of the children of an entry: `derived_boxes`, the boxes of ISO/IEC 14496-12 the class defines, of the lengths `defined`, and `extra_boxes`
pub(crate) fn children_len(
    derived_boxes: &OtherBoxes,
    defined: impl IntoIterator<Item = u64>,
    extra_boxes: &OtherBoxes,
) -> u64 {
    derived_boxes
        .as_slice()
        .iter()
        .chain(extra_boxes.as_slice())
        .map(AnyBox::encoded_len)
        .chain(defined)
        .fold(0, u64::saturating_add)
}

/// Writes the children of an entry onto the front of `rest` in the order of the three parts: `derived_boxes`, the boxes of ISO/IEC 14496-12 that `encode_defined` writes, then `extra_boxes`
///
/// # Errors
///
/// * The failures of writing a child.
pub(crate) fn encode_children<'buffer>(
    mut rest: &'buffer mut [u8],
    derived_boxes: &OtherBoxes,
    encode_defined: impl FnOnce(&'buffer mut [u8]) -> Result<&'buffer mut [u8], Error>,
    extra_boxes: &OtherBoxes,
) -> Result<(), Error> {
    for kept in derived_boxes.as_slice() {
        rest = kept.encode(rest)?;
    }
    rest = encode_defined(rest)?;
    for kept in extra_boxes.as_slice() {
        rest = kept.encode(rest)?;
    }

    Ok(())
}

/// Writes `child`, if there is one, onto the front of `rest`, and returns what is left
///
/// # Errors
///
/// * The failures of writing `child`.
pub(crate) fn encode_optional<'buffer, Child: BoxEncode + BoxDefinition>(
    rest: &'buffer mut [u8],
    child: Option<&Child>,
) -> Result<&'buffer mut [u8], Error> {
    match child {
        Some(child) => child.encode(rest),
        None => Ok(rest),
    }
}

impl BoxEncode for SampleEntry {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.remaining_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(6 + 2)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[0; 6])?;
        writer.write_u16(self.data_reference_index)?;
        writer.write_slice(&self.remaining_data)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxEncode, Error, FourCc};

    use super::SampleEntry;

    /// Payload of an entry of data reference 1, with two bytes its class lays out
    const PAYLOAD: &[u8] = b"\0\0\0\0\0\0\0\x01\xab\xcd";

    fn entry() -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"xvid"),
            data_reference_index: 1,
            remaining_data: vec![0xab, 0xcd],
        }
    }

    #[test]
    fn reads_the_data_reference_index_past_the_reserved_bytes_and_keeps_the_rest() {
        assert_eq!(
            SampleEntry::decode_payload(
                FourCc::new(*b"xvid"),
                b"\xff\xff\xff\xff\xff\xff\0\x01\xab\xcd"
            ),
            Ok(entry())
        );
    }

    #[test]
    fn writes_the_reserved_bytes_as_zero_then_the_data_reference_index_and_the_rest() {
        let mut payload = [0xff; 10];
        entry().encode_payload(&mut payload).unwrap();

        assert_eq!(payload, PAYLOAD);
    }

    #[test]
    fn a_payload_ending_inside_the_data_reference_index_is_refused() {
        assert_eq!(
            SampleEntry::decode_payload(FourCc::new(*b"xvid"), b"\0\0\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 7))
        );
    }
}
