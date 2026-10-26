//! [`FontSampleEntry`], the sample entry of a font track

use isobmff_core::{BoxEncode, Error, FourCc, OtherBoxes};

use crate::btrt::BitRateBox;
use crate::sample_entry::{
    BitRateChildren, SampleEntry, children_len, encode_children, encode_optional, from_class,
};

/// [`FontSampleEntry`], ISO/IEC 14496-12 §12.7.3
///
/// The sample entry of a font track, which holds no field of its own: the
/// boxes of a specification deriving the class configure the font stream.
/// The bytes of a [`SampleEntry`] after `data_reference_index` are read with
/// [`TryFrom`] and written back with [`SampleEntry::try_from`].
///
/// # Children
///
/// The children are laid out in three parts, the order in which the entry
/// writes them, whatever order they were read in: the boxes a specification
/// deriving the class defines, in [`derived_boxes`](Self::derived_boxes);
/// `btrt`, which any sample entry may hold; and the boxes no definition of
/// the class names, in [`extra_boxes`](Self::extra_boxes). A child of a type
/// this crate does not read that lies before `btrt` is read into the first
/// part, and one that lies after it into the third.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct FontSampleEntry {
    /// Coding name of the samples, the `format` of the [`SampleEntry`]
    pub format: FourCc,
    /// Index from 1 of the [`DataEntry`](crate::DataEntry) the samples are retrieved from
    pub data_reference_index: u16,
    /// Children a specification deriving the class defines, which lay before the boxes of ISO/IEC 14496-12
    pub derived_boxes: OtherBoxes,
    /// Bit rates and decoding buffer size of the stream
    pub btrt: Option<BitRateBox>,
    /// Children no definition of the class names, which lay after a box of ISO/IEC 14496-12
    pub extra_boxes: OtherBoxes,
}

impl Default for FontSampleEntry {
    fn default() -> Self {
        Self {
            format: FourCc::new([0; 4]),
            data_reference_index: 0,
            derived_boxes: OtherBoxes::new(),
            btrt: None,
            extra_boxes: OtherBoxes::new(),
        }
    }
}

impl TryFrom<&SampleEntry> for FontSampleEntry {
    type Error = Error;

    /// Reads the entry from the bytes of `entry` after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of a child that does not frame, and of a `btrt` given
    ///   more than once.
    fn try_from(entry: &SampleEntry) -> Result<Self, Error> {
        let children = BitRateChildren::read(&entry.remaining_data)?;

        Ok(Self {
            format: entry.format,
            data_reference_index: entry.data_reference_index,
            derived_boxes: children.derived_boxes,
            btrt: children.btrt,
            extra_boxes: children.extra_boxes,
        })
    }
}

impl TryFrom<&FontSampleEntry> for SampleEntry {
    type Error = Error;

    /// Writes `entry` into the bytes of an entry after `data_reference_index`
    ///
    /// # Errors
    ///
    /// * The failures of writing a child.
    fn try_from(entry: &FontSampleEntry) -> Result<Self, Error> {
        from_class(
            entry.format,
            entry.data_reference_index,
            children_len(
                &entry.derived_boxes,
                entry.btrt.as_ref().map(BoxEncode::encoded_len),
                &entry.extra_boxes,
            ),
            |writer| {
                encode_children(
                    writer.take_remainder(),
                    &entry.derived_boxes,
                    |rest| encode_optional(rest, entry.btrt.as_ref()),
                    &entry.extra_boxes,
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{ChildBoxes, FourCc, OtherBoxes};

    use super::FontSampleEntry;
    use crate::btrt::BitRateBox;
    use crate::sample_entry::SampleEntry;

    /// Configuration of a derived specification, of a type this crate does not read
    const CONFIGURATION: &[u8] = b"\0\0\0\x09conf\x01";

    /// `btrt` of a buffer of 4096 bytes, at most 2 Mbit/s and 1 Mbit/s on average
    const BTRT: &[u8] = b"\0\0\0\x14btrt\0\0\x10\0\0\x1e\x84\x80\0\x0f\x42\x40";

    fn sample_entry(remaining: &[&[u8]]) -> SampleEntry {
        SampleEntry {
            format: FourCc::new(*b"otto"),
            data_reference_index: 1,
            remaining_data: remaining.concat(),
        }
    }

    fn kept(children: &[&[u8]]) -> OtherBoxes {
        OtherBoxes::from(ChildBoxes::decode(&children.concat()).unwrap())
    }

    fn entry(extra_boxes: &[&[u8]]) -> FontSampleEntry {
        FontSampleEntry {
            format: FourCc::new(*b"otto"),
            data_reference_index: 1,
            derived_boxes: kept(&[CONFIGURATION]),
            btrt: Some(BitRateBox {
                buffer_size_db: 4096,
                max_bitrate: 2_000_000,
                avg_bitrate: 1_000_000,
            }),
            extra_boxes: kept(extra_boxes),
        }
    }

    #[test]
    fn reads_its_boxes_split_by_where_they_lie() {
        assert_eq!(
            FontSampleEntry::try_from(&sample_entry(&[CONFIGURATION, BTRT, CONFIGURATION])),
            Ok(entry(&[CONFIGURATION]))
        );
    }

    #[test]
    fn writes_its_boxes_in_the_order_of_its_class() {
        assert_eq!(
            SampleEntry::try_from(&entry(&[CONFIGURATION])),
            Ok(sample_entry(&[CONFIGURATION, BTRT, CONFIGURATION]))
        );
    }
}
