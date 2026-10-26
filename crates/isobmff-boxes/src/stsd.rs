//! [`SampleDescriptionBox`], the sample entries that describe the coding of the samples of a track

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter,
    FullBoxFlags, FullBoxHeader, RawBox,
};

use crate::box_type_argument;
use crate::sample_entry::SampleEntry;
use crate::table::write_entry_count;

/// One entry of a [`SampleDescriptionBox`]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum SampleDescription {
    /// Entry of the format the box type gives
    Typed(SampleEntry),
    /// Entry under a `uuid` box type, or one that does not decode, kept as its bytes
    Other(AnyBox),
}

/// [`SampleDescriptionBox`] (`stsd`), ISO/IEC 14496-12 §8.5.2
///
/// The sample entries of a track, which give the coding type of its samples
/// and what initializing the decoder needs.
///
/// Every entry under a compact box type is read as a [`SampleEntry`] of that
/// format. An entry under a `uuid` box type, and one whose payload ends
/// inside `data_reference_index`, is kept as its bytes. The entries are kept,
/// and written back, in the order they came.
///
/// The box reads version 0 and version 1 and keeps the version it was read
/// in, which it is written in. §8.5.2.3 treats version 1 as version 0, while
/// an `AudioSampleEntryV1` lies in a box of version 1 alone, so neither
/// version is checked against the entries the box holds. The `flags` are read
/// past and written as zero.
///
/// # Untrusted input
///
/// The box reads the `entry_count` entries it states, after holding the count
/// against the payload, and allocates room for an entry only once it has read
/// it, so what it allocates is bounded by the length of the payload, which the
/// caller sets.
#[doc(alias = "stsd")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct SampleDescriptionBox {
    /// Version of the box, 0 or 1
    ///
    /// A version other than 0 or 1 is refused with
    /// [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion)
    /// when the box is written.
    pub version: u8,
    /// Entries, in the order the `sample_description_index` of `stsc` and of a track fragment counts them from 1
    pub sample_descriptions: Vec<SampleDescription>,
}

impl BoxDefinition for SampleDescriptionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsd");
}

/// Returns `version` if the box reads it
///
/// # Errors
///
/// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
///   version is neither 0 nor 1.
fn supported(version: u8) -> Result<u8, Error> {
    match version {
        0 | 1 => Ok(version),
        _ => Err(Error::unsupported_version(version)),
    }
}

impl BoxDecode for SampleDescriptionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = supported(FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version())?;
        let entry_count = reader.read_u32()?;
        // Why not the sixteen bytes of the fields every entry opens with: an
        // entry that does not decode is kept, and may be a box of its header
        // alone, eight bytes.
        reader.require(u64::from(entry_count).saturating_mul(8))?;
        let mut rest = reader.take_remainder();
        // Why not room for `entry_count` entries up front: an entry may take only
        // the eight bytes of its header, while a `SampleDescription` takes several
        // times that, so the room would outgrow the payload it was read from.
        let mut sample_descriptions = Vec::new();
        for _ in 0..entry_count {
            let (entry, after) = RawBox::split_first(rest)?;
            rest = after;
            let box_type = entry.header().box_type();
            let payload = entry.payload();
            let read = match box_type {
                BoxType::Compact(format) => SampleEntry::decode_payload(format.four_cc(), payload)
                    .map(SampleDescription::Typed)
                    .ok(),
                BoxType::Extended(_) => None,
            };
            sample_descriptions.push(read.unwrap_or_else(|| {
                SampleDescription::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self {
            version,
            sample_descriptions,
        })
    }
}

impl BoxEncode for SampleDescriptionBox {
    fn payload_len(&self) -> u64 {
        self.sample_descriptions
            .iter()
            .map(|entry| match entry {
                SampleDescription::Typed(typed) => box_type_argument::encoded_len(typed),
                SampleDescription::Other(other) => other.encoded_len(),
            })
            .fold(4 + 4, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = supported(self.version)?;
        writer.write_bytes(&FullBoxHeader::new(version, FullBoxFlags::ZERO).to_bytes())?;
        write_entry_count(writer, self.sample_descriptions.len())?;
        let mut rest = writer.take_remainder();
        for entry in &self.sample_descriptions {
            rest = match entry {
                SampleDescription::Typed(typed) => {
                    box_type_argument::encode(typed.format, "format", typed, rest)?
                }
                SampleDescription::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error, FourCc};

    use super::{SampleDescription, SampleDescriptionBox};
    use crate::encoded;
    use crate::sample_entry::SampleEntry;

    /// `mp4a` entry of data reference 1, with two bytes its class lays out
    const MP4A: &[u8] = b"\0\0\0\x12mp4a\0\0\0\0\0\0\0\x01\xab\xcd";

    /// Entry of a format ISO/IEC 14496-12 does not register, of data reference 2
    const UNREGISTERED: &[u8] = b"\0\0\0\x10xvid\0\0\0\0\0\0\0\x02";

    /// Entry under a `uuid` box type
    const EXTENDED: &[u8] =
        b"\0\0\0\x1cuuid\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\0\0\0\x07";

    fn typed(
        format: &[u8; 4],
        data_reference_index: u16,
        remaining_data: &[u8],
    ) -> SampleDescription {
        SampleDescription::Typed(SampleEntry {
            format: FourCc::new(*format),
            data_reference_index,
            remaining_data: remaining_data.to_vec(),
        })
    }

    fn descriptions(version: u8) -> SampleDescriptionBox {
        SampleDescriptionBox {
            version,
            sample_descriptions: vec![
                typed(b"mp4a", 1, &[0xab, 0xcd]),
                SampleDescription::Other(AnyBox::from_raw_bytes(
                    BoxType::extended(
                        *b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10",
                    ),
                    b"\0\0\0\x07".to_vec(),
                )),
                typed(b"xvid", 2, &[]),
            ],
        }
    }

    /// Bytes of the box [`descriptions`] holds
    fn descriptions_bytes(version: u8) -> Vec<u8> {
        [
            b"\0\0\0\x4estsd".as_slice(),
            &[version, 0, 0, 0, 0, 0, 0, 3],
            MP4A,
            EXTENDED,
            UNREGISTERED,
        ]
        .concat()
    }

    #[test]
    fn reads_every_entry_of_a_compact_type_as_an_entry_of_that_format_in_the_order_they_came() {
        assert_eq!(
            SampleDescriptionBox::decode(&descriptions_bytes(0)),
            Ok((descriptions(0), b"".as_slice()))
        );
    }

    #[test]
    fn writes_each_entry_under_its_format_in_the_order_it_holds_them() {
        assert_eq!(encoded(&descriptions(0)), descriptions_bytes(0));
    }

    #[test]
    fn a_box_of_version_1_reads_and_writes_back_in_version_1() {
        let bytes = descriptions_bytes(1);
        let (read, _) = SampleDescriptionBox::decode(&bytes).unwrap();

        assert_eq!((encoded(&read), read), (bytes, descriptions(1)));
    }

    #[test]
    fn the_flags_are_read_past_and_written_as_zero() {
        let (read, _) = SampleDescriptionBox::decode(
            b"\0\0\0\x20stsd\0\0\0\x01\0\0\0\x01\0\0\0\x10xvid\0\0\0\0\0\0\0\x02",
        )
        .unwrap();

        assert_eq!(
            encoded(&read),
            b"\0\0\0\x20stsd\0\0\0\0\0\0\0\x01\0\0\0\x10xvid\0\0\0\0\0\0\0\x02"
        );
    }

    #[test]
    fn an_entry_ending_inside_its_data_reference_index_is_kept_as_its_bytes() {
        let expected = SampleDescriptionBox {
            version: 0,
            sample_descriptions: vec![SampleDescription::Other(AnyBox::from_raw_bytes(
                BoxType::compact(*b"mp4a"),
                b"\0\0\0\0\0\0\0".to_vec(),
            ))],
        };

        assert_eq!(
            SampleDescriptionBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0\0\x0fmp4a\0\0\0\0\0\0\0"),
            Ok(expected)
        );
    }

    #[test]
    fn a_count_the_payload_cannot_hold_is_refused_before_an_entry_is_read() {
        assert_eq!(
            SampleDescriptionBox::decode_payload(
                b"\0\0\0\0\xff\xff\xff\xff\0\0\0\x10xvid\0\0\0\0\0\0\0\x02"
            ),
            Err(Error::truncated_payload(8 + u64::from(u32::MAX) * 8, 24))
        );
    }

    #[test]
    fn a_payload_ending_inside_the_entry_count_is_refused() {
        assert_eq!(
            SampleDescriptionBox::decode_payload(b"\0\0\0\0\0\0"),
            Err(Error::truncated_payload(8, 6))
        );
    }

    #[test]
    fn a_version_other_than_0_or_1_is_refused() {
        assert_eq!(
            SampleDescriptionBox::decode_payload(b"\x02\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_version_other_than_0_or_1_is_refused_when_written() {
        let descriptions = SampleDescriptionBox {
            version: 2,
            ..SampleDescriptionBox::default()
        };

        assert_eq!(
            descriptions.encode(&mut [0; 16]),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn an_entry_of_the_format_uuid_is_refused_when_written() {
        let descriptions = SampleDescriptionBox {
            version: 0,
            sample_descriptions: vec![typed(b"uuid", 1, &[])],
        };

        assert_eq!(
            descriptions.encode(&mut [0; 32]),
            Err(Error::invalid_value("format"))
        );
    }
}
