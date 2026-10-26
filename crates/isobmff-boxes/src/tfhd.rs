//! [`TrackFragmentHeaderBox`], the defaults the track runs of a track fragment use

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::data_types::SampleFlags;
use crate::full_box::read_version_zero;

/// Flag mask of base-data-offset-present
const BASE_DATA_OFFSET_PRESENT: u32 = 0x00_0001;

/// Flag mask of sample-description-index-present
const SAMPLE_DESCRIPTION_INDEX_PRESENT: u32 = 0x00_0002;

/// Flag mask of default-sample-duration-present
const DEFAULT_SAMPLE_DURATION_PRESENT: u32 = 0x00_0008;

/// Flag mask of default-sample-size-present
const DEFAULT_SAMPLE_SIZE_PRESENT: u32 = 0x00_0010;

/// Flag mask of default-sample-flags-present
const DEFAULT_SAMPLE_FLAGS_PRESENT: u32 = 0x00_0020;

/// Flag mask of duration-is-empty
const DURATION_IS_EMPTY: u32 = 0x01_0000;

/// Flag mask of default-base-is-moof
const DEFAULT_BASE_IS_MOOF: u32 = 0x02_0000;

/// [`TrackFragmentHeaderBox`] (`tfhd`), ISO/IEC 14496-12 §8.8.7
///
/// Sets up the information and defaults the track runs of a track fragment
/// use, overriding those of the `TrackExtendsBox` of the track.
///
/// A flag of `tf_flags` that says a field is there is whether the field is
/// [`Some`]; the flags that carry a meaning of their own are held, and the
/// others are read past and written as zero.
///
/// Where `base_data_offset` is [`None`], its value and the origin it counts
/// from are inferred as Table 6 of ISO/IEC 14496-12 sets out:
///
/// | `default_base_is_moof` | data reference entry | first `traf` of the `moof` | `base_data_offset` | relative to |
/// |---|---|---|---|---|
/// | `true` | any | any | 0 | the first byte of the `MovieFragmentBox` |
/// | `false` | any | no | 1 | the end of the data of the preceding track fragment |
/// | `false` | `imdt` or `snim` | yes | 0 | the first byte of the payload of the `IdentifiedMediaDataBox` of the entry |
/// | `false` | any other | yes | 0 | the first byte of the `MovieFragmentBox` |
///
/// Where it is [`Some`], it counts from the first byte of the payload of the
/// `IdentifiedMediaDataBox` of a data reference entry `imdt` or `snim`, and
/// from the start of the file the entry identifies otherwise.
///
/// The box holds `base_data_offset` as it was read: it is the position of
/// data in the file, which a box written back elsewhere in a file no longer
/// points at unless the writer of the file adjusts it.
#[doc(alias = "tfhd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct TrackFragmentHeaderBox {
    /// Whether the durations the box and the `TrackExtendsBox` give are empty, no sample filling them
    ///
    /// A presentation does not have both edit lists in the `MovieBox` and
    /// track fragments of empty duration.
    pub duration_is_empty: bool,
    /// Whether, without a `base_data_offset`, the offsets count from the first byte of the `MovieFragmentBox`
    ///
    /// It is ignored where `base_data_offset` is [`Some`]. Brands before
    /// `iso5` do not use it.
    pub default_base_is_moof: bool,
    /// `track_ID` of the track the fragment is of
    pub track_id: u32,
    /// Base offset of the data offsets of the track runs, the base-data-offset-present flag
    pub base_data_offset: Option<u64>,
    /// Index of the sample entry of the samples of the fragment, the sample-description-index-present flag
    pub sample_description_index: Option<u32>,
    /// Default duration of the samples of the fragment, the default-sample-duration-present flag
    pub default_sample_duration: Option<u32>,
    /// Default size of the samples of the fragment, the default-sample-size-present flag
    pub default_sample_size: Option<u32>,
    /// Default flags of the samples of the fragment, the default-sample-flags-present flag
    pub default_sample_flags: Option<SampleFlags>,
}

impl BoxDefinition for TrackFragmentHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tfhd");
}

impl BoxDecode for TrackFragmentHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let tf_flags = read_version_zero(reader)?.bits();
        let track_id = reader.read_u32()?;
        let base_data_offset = if tf_flags & BASE_DATA_OFFSET_PRESENT == 0 {
            None
        } else {
            Some(reader.read_u64()?)
        };
        let mut read_if = |mask: u32| -> Result<Option<u32>, Error> {
            if tf_flags & mask == 0 {
                Ok(None)
            } else {
                reader.read_u32().map(Some)
            }
        };

        Ok(Self {
            duration_is_empty: tf_flags & DURATION_IS_EMPTY != 0,
            default_base_is_moof: tf_flags & DEFAULT_BASE_IS_MOOF != 0,
            track_id,
            base_data_offset,
            sample_description_index: read_if(SAMPLE_DESCRIPTION_INDEX_PRESENT)?,
            default_sample_duration: read_if(DEFAULT_SAMPLE_DURATION_PRESENT)?,
            default_sample_size: read_if(DEFAULT_SAMPLE_SIZE_PRESENT)?,
            default_sample_flags: read_if(DEFAULT_SAMPLE_FLAGS_PRESENT)?
                .map(SampleFlags::from_bits),
        })
    }
}

impl BoxEncode for TrackFragmentHeaderBox {
    fn payload_len(&self) -> u64 {
        [
            (self.base_data_offset.is_some(), 8),
            (self.sample_description_index.is_some(), 4),
            (self.default_sample_duration.is_some(), 4),
            (self.default_sample_size.is_some(), 4),
            (self.default_sample_flags.is_some(), 4),
        ]
        .into_iter()
        .filter_map(|(present, len)| present.then_some(len))
        .fold(4 + 4, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let tf_flags = [
            (self.base_data_offset.is_some(), BASE_DATA_OFFSET_PRESENT),
            (
                self.sample_description_index.is_some(),
                SAMPLE_DESCRIPTION_INDEX_PRESENT,
            ),
            (
                self.default_sample_duration.is_some(),
                DEFAULT_SAMPLE_DURATION_PRESENT,
            ),
            (
                self.default_sample_size.is_some(),
                DEFAULT_SAMPLE_SIZE_PRESENT,
            ),
            (
                self.default_sample_flags.is_some(),
                DEFAULT_SAMPLE_FLAGS_PRESENT,
            ),
            (self.duration_is_empty, DURATION_IS_EMPTY),
            (self.default_base_is_moof, DEFAULT_BASE_IS_MOOF),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |tf_flags, (_, mask)| tf_flags | mask);

        writer.write_u32(tf_flags)?;
        writer.write_u32(self.track_id)?;
        if let Some(base_data_offset) = self.base_data_offset {
            writer.write_u64(base_data_offset)?;
        }
        if let Some(sample_description_index) = self.sample_description_index {
            writer.write_u32(sample_description_index)?;
        }
        if let Some(default_sample_duration) = self.default_sample_duration {
            writer.write_u32(default_sample_duration)?;
        }
        if let Some(default_sample_size) = self.default_sample_size {
            writer.write_u32(default_sample_size)?;
        }
        if let Some(default_sample_flags) = self.default_sample_flags {
            writer.write_u32(default_sample_flags.to_bits()?)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::TrackFragmentHeaderBox;
    use crate::data_types::SampleFlags;
    use crate::encoded;

    /// `tfhd` of track 1 with every optional field: base offset 2^32, description 2, duration 1001, size 300, flags `0x0101_0000`; empty and based at the `moof`
    const TFHD_EVERY_FIELD: &[u8] = b"\0\0\0\x28tfhd\0\x03\0\x3b\0\0\0\x01\0\0\0\x01\0\0\0\0\
        \0\0\0\x02\0\0\x03\xe9\0\0\x01\x2c\x01\x01\0\0";

    /// `tfhd` of track 1 based at the `moof`, with a default size of 300 alone
    const TFHD_SIZE_ALONE: &[u8] = b"\0\0\0\x14tfhd\0\x02\0\x10\0\0\0\x01\0\0\x01\x2c";

    fn every_field() -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox {
            duration_is_empty: true,
            default_base_is_moof: true,
            track_id: 1,
            base_data_offset: Some(1 << 32),
            sample_description_index: Some(2),
            default_sample_duration: Some(1001),
            default_sample_size: Some(300),
            default_sample_flags: Some(SampleFlags::from_bits(0x0101_0000)),
        }
    }

    fn size_alone() -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox {
            default_base_is_moof: true,
            track_id: 1,
            default_sample_size: Some(300),
            ..TrackFragmentHeaderBox::default()
        }
    }

    #[test]
    fn reads_the_fields_its_flags_say_are_there() {
        assert_eq!(
            (
                TrackFragmentHeaderBox::decode(TFHD_EVERY_FIELD),
                TrackFragmentHeaderBox::decode(TFHD_SIZE_ALONE)
            ),
            (
                Ok((every_field(), b"".as_slice())),
                Ok((size_alone(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_flags_of_the_fields_that_are_there() {
        assert_eq!(
            (encoded(&every_field()), encoded(&size_alone())),
            (TFHD_EVERY_FIELD.to_vec(), TFHD_SIZE_ALONE.to_vec())
        );
    }

    #[test]
    fn a_flag_the_specification_does_not_define_is_read_past_and_written_as_zero() {
        let (read, _) =
            TrackFragmentHeaderBox::decode(b"\0\0\0\x14tfhd\0\x82\0\x14\0\0\0\x01\0\0\x01\x2c")
                .unwrap();

        assert_eq!(encoded(&read), TFHD_SIZE_ALONE);
    }

    #[test]
    fn default_sample_flags_wider_than_their_bits_are_refused_when_written() {
        let mut wide = size_alone();
        wide.default_sample_flags = Some(SampleFlags {
            is_leading: 4,
            ..SampleFlags::default()
        });

        assert_eq!(
            wide.encode(&mut [0; 24]),
            Err(Error::invalid_value("is_leading"))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_its_flags_say_is_there_is_refused() {
        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(b"\0\0\0\x10\0\0\0\x01\0\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            TrackFragmentHeaderBox::decode_payload(b"\x01\0\0\0\0\0\0\x01"),
            Err(Error::unsupported_version(1))
        );
    }
}
