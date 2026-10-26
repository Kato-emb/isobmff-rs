//! [`TrackGroupBox`], the track groups a track belongs to

use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, FieldReader, FieldWriter,
    FourCc, FullBoxFlags, FullBoxHeader,
};

use crate::box_type_argument;
use crate::free::{Free, FreeType, Skip};
use crate::full_box::read_version_zero;

/// Bit of `left_view_flag`, the most significant of the 32 bits it shares with `reserved`
const LEFT_VIEW_FLAG: u32 = 1 << 31;

/// [`TrackGroupTypeBox`], ISO/IEC 14496-12 §8.3.4
///
/// Membership of the track in the track group of a `track_group_type` and a
/// `track_group_id`. The type is the box type of the box, which
/// [`TrackGroupBox`] reads into [`track_group_type`](Self::track_group_type)
/// and writes from it, so a box of any type, registered or not, reads.
/// [`StereoVideoGroupBox`], of the type `ster`, is a type of its own.
///
/// The box reads version 0 alone, the version ISO/IEC 14496-12 states unless
/// the definition of a `track_group_type` states another.
///
/// The box has no box type of its own, so it implements neither
/// [`BoxDecode`] nor [`BoxDefinition`]: its [`BoxEncode`] writes its payload,
/// and a [`TrackGroupBox`] writes the header.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackGroupTypeBox {
    /// Type of the grouping, such as `msrc`
    ///
    /// It is written as the box type, so a `track_group_type` of `uuid` is
    /// refused with [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue)
    /// for `track_group_type` when the box is written. A box written under
    /// `ster` reads back as a [`StereoVideoGroupBox`], and one under `free` or
    /// `skip` as the bytes of free space.
    pub track_group_type: FourCc,
    /// Flags of the box, kept as read
    ///
    /// Bit 0 set states that `track_group_id` is no `track_ID` and no
    /// `track_group_id` of a track group of another `track_group_type`, so a
    /// [`TrackReferenceTypeBox`](crate::TrackReferenceTypeBox) can refer to
    /// the track group by it. It is set in every box of the same
    /// `track_group_type` and `track_group_id`, if in one. Bits 1 to 11 are
    /// reserved, and bits 12 to 23 are zero unless the definition of the
    /// `track_group_type` gives them a meaning.
    pub flags: FullBoxFlags,
    /// Identifier of the track group, shared by the tracks that belong to it
    pub track_group_id: u32,
    /// Bytes after `track_group_id`, which the definition of the `track_group_type` lays out
    pub remaining_data: Vec<u8>,
}

impl Default for TrackGroupTypeBox {
    fn default() -> Self {
        Self {
            track_group_type: FourCc::new([0; 4]),
            flags: FullBoxFlags::ZERO,
            track_group_id: 0,
            remaining_data: Vec::new(),
        }
    }
}

impl TrackGroupTypeBox {
    /// Reads the payload of a box of the box type `track_group_type`
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside `track_group_id`.
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion):
    ///   the version is not 0.
    fn decode_payload(track_group_type: FourCc, payload: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(payload);

        Ok(Self {
            track_group_type,
            flags: read_version_zero(&mut reader)?,
            track_group_id: reader.read_u32()?,
            remaining_data: reader.take_remainder().to_vec(),
        })
    }
}

impl BoxEncode for TrackGroupTypeBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.remaining_data.len())
            .unwrap_or(u64::MAX)
            .saturating_add(4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(0, self.flags).to_bytes())?;
        writer.write_u32(self.track_group_id)?;
        writer.write_slice(&self.remaining_data)
    }
}

/// [`StereoVideoGroupBox`] (`ster`), ISO/IEC 14496-12 §8.3.4.4.2
///
/// Membership of the track, as the left or the right view, in a stereo pair
/// suitable for playback on a stereoscopic display. At most two tracks share
/// a `track_group_id`, and two that do differ in `left_view_flag`.
///
/// The box reads version 0 alone. The 31 bits of `reserved` are read past and
/// written as zero.
#[doc(alias = "ster")]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StereoVideoGroupBox {
    /// Flags of the box, kept as read, as [`TrackGroupTypeBox::flags`] states them
    pub flags: FullBoxFlags,
    /// Identifier of the stereo pair, shared by its two tracks
    pub track_group_id: u32,
    /// Whether the track is the left view of the stereo pair rather than the right
    pub left_view_flag: bool,
}

impl Default for StereoVideoGroupBox {
    fn default() -> Self {
        Self {
            flags: FullBoxFlags::ZERO,
            track_group_id: 0,
            left_view_flag: false,
        }
    }
}

impl BoxDefinition for StereoVideoGroupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"ster");
}

impl BoxDecode for StereoVideoGroupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            flags: read_version_zero(reader)?,
            track_group_id: reader.read_u32()?,
            left_view_flag: reader.read_u32()? & LEFT_VIEW_FLAG != 0,
        })
    }
}

impl BoxEncode for StereoVideoGroupBox {
    fn payload_len(&self) -> u64 {
        4 + 4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(0, self.flags).to_bytes())?;
        writer.write_u32(self.track_group_id)?;
        writer.write_u32(if self.left_view_flag {
            LEFT_VIEW_FLAG
        } else {
            0
        })
    }
}

/// One child of a [`TrackGroupBox`]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum TrackGroup {
    /// Membership in a stereo pair
    StereoVideo(StereoVideoGroupBox),
    /// Membership in a track group of the type the box type gives
    Typed(TrackGroupTypeBox),
    /// Child under the box type `free`, `skip` or `uuid`, or one that does not decode, kept as its bytes
    Other(AnyBox),
}

/// [`TrackGroupBox`] (`trgr`), ISO/IEC 14496-12 §8.3.4
///
/// Track groups the track belongs to, each named by the box type and the
/// `track_group_id` of a child.
///
/// A `ster` child is read as a [`StereoVideoGroupBox`], and any other child
/// under a compact box type as a [`TrackGroupTypeBox`] of that type. A child
/// under `free` or `skip`, the free space the box may hold, or under a `uuid`
/// box type, and one that does not decode, such as one of a version other
/// than 0, is kept as its bytes. The children are kept, and written back, in
/// the order they came.
///
/// # Untrusted input
///
/// The box reads each child as it frames it, and allocates room for a child
/// only once it has read it, so what it allocates is bounded by the length of
/// the payload, which the caller sets.
#[doc(alias = "trgr")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct TrackGroupBox {
    /// Children, in the order they came
    pub track_groups: Vec<TrackGroup>,
}

impl BoxDefinition for TrackGroupBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trgr");
}

impl BoxDecode for TrackGroupBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut track_groups = Vec::new();
        for child in Boxes::new(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();
            let payload = child.payload();
            let read = match box_type {
                StereoVideoGroupBox::BOX_TYPE => StereoVideoGroupBox::decode_payload(payload)
                    .map(TrackGroup::StereoVideo)
                    .ok(),
                Free::FREE_TYPE | Skip::FREE_TYPE | BoxType::Extended(_) => None,
                BoxType::Compact(track_group_type) => {
                    TrackGroupTypeBox::decode_payload(track_group_type.four_cc(), payload)
                        .map(TrackGroup::Typed)
                        .ok()
                }
            };
            track_groups.push(read.unwrap_or_else(|| {
                TrackGroup::Other(AnyBox::from_raw_bytes(box_type, payload.to_vec()))
            }));
        }

        Ok(Self { track_groups })
    }
}

impl BoxEncode for TrackGroupBox {
    fn payload_len(&self) -> u64 {
        self.track_groups
            .iter()
            .map(|child| match child {
                TrackGroup::StereoVideo(stereo_video) => stereo_video.encoded_len(),
                TrackGroup::Typed(typed) => box_type_argument::encoded_len(typed),
                TrackGroup::Other(other) => other.encoded_len(),
            })
            .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        for child in &self.track_groups {
            rest = match child {
                TrackGroup::StereoVideo(stereo_video) => stereo_video.encode(rest)?,
                TrackGroup::Typed(typed) => box_type_argument::encode(
                    typed.track_group_type,
                    "track_group_type",
                    typed,
                    rest,
                )?,
                TrackGroup::Other(other) => other.encode(rest)?,
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType, Error, FourCc, FullBoxFlags};

    use super::{StereoVideoGroupBox, TrackGroup, TrackGroupBox, TrackGroupTypeBox};
    use crate::encoded;
    use crate::free::{Free, FreeType};

    /// `ster` of the left view of the stereo pair 5, whose identifier no track takes
    const STER: &[u8] = b"\0\0\0\x14ster\0\0\0\x01\0\0\0\x05\x80\0\0\0";

    /// `msrc` of the track group 9, with two bytes the type does not lay out
    const MSRC: &[u8] = b"\0\0\0\x12msrc\0\0\0\0\0\0\0\x09\xab\xcd";

    /// Free space
    const FREE: &[u8] = b"\0\0\0\x0afree\0\0";

    fn groups() -> TrackGroupBox {
        TrackGroupBox {
            track_groups: vec![
                TrackGroup::Typed(TrackGroupTypeBox {
                    track_group_type: FourCc::new(*b"msrc"),
                    flags: FullBoxFlags::ZERO,
                    track_group_id: 9,
                    remaining_data: vec![0xab, 0xcd],
                }),
                TrackGroup::Other(AnyBox::from_raw_bytes(Free::FREE_TYPE, vec![0, 0])),
                TrackGroup::StereoVideo(StereoVideoGroupBox {
                    flags: FullBoxFlags::new(1).unwrap(),
                    track_group_id: 5,
                    left_view_flag: true,
                }),
            ],
        }
    }

    #[test]
    fn reads_a_stereo_pair_as_its_own_type_and_other_groups_by_their_type_in_the_order_they_came() {
        let bytes = [b"\0\0\0\x38trgr".as_slice(), MSRC, FREE, STER].concat();

        assert_eq!(
            TrackGroupBox::decode(&bytes),
            Ok((groups(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_each_child_under_its_type_in_the_order_it_holds_them() {
        assert_eq!(
            encoded(&groups()),
            [b"\0\0\0\x38trgr".as_slice(), MSRC, FREE, STER].concat()
        );
    }

    #[test]
    fn a_child_of_a_version_other_than_0_is_kept_as_its_bytes() {
        let expected = TrackGroupBox {
            track_groups: vec![TrackGroup::Other(AnyBox::from_raw_bytes(
                BoxType::compact(*b"msrc"),
                b"\x01\0\0\0\0\0\0\x09".to_vec(),
            ))],
        };

        assert_eq!(
            TrackGroupBox::decode_payload(b"\0\0\0\x10msrc\x01\0\0\0\0\0\0\x09"),
            Ok(expected)
        );
    }

    #[test]
    fn a_child_ending_inside_its_track_group_id_is_kept_as_its_bytes() {
        let expected = TrackGroupBox {
            track_groups: vec![TrackGroup::Other(AnyBox::from_raw_bytes(
                BoxType::compact(*b"msrc"),
                b"\0\0\0\0\0\0".to_vec(),
            ))],
        };

        assert_eq!(
            TrackGroupBox::decode_payload(b"\0\0\0\x0emsrc\0\0\0\0\0\0"),
            Ok(expected)
        );
    }

    #[test]
    fn a_track_group_of_the_type_uuid_is_refused_when_written() {
        let groups = TrackGroupBox {
            track_groups: vec![TrackGroup::Typed(TrackGroupTypeBox {
                track_group_type: FourCc::new(*b"uuid"),
                ..TrackGroupTypeBox::default()
            })],
        };

        assert_eq!(
            groups.encode(&mut [0; 24]),
            Err(Error::invalid_value("track_group_type"))
        );
    }

    #[test]
    fn a_stereo_pair_reads_its_view_past_the_reserved_bits_and_writes_them_as_zero() {
        let stereo_video = StereoVideoGroupBox {
            track_group_id: 5,
            ..StereoVideoGroupBox::default()
        };

        assert_eq!(
            StereoVideoGroupBox::decode(b"\0\0\0\x14ster\0\0\0\0\0\0\0\x05\x7f\xff\xff\xff"),
            Ok((stereo_video, b"".as_slice()))
        );
        assert_eq!(
            encoded(&stereo_video),
            b"\0\0\0\x14ster\0\0\0\0\0\0\0\x05\0\0\0\0"
        );
    }

    #[test]
    fn a_stereo_pair_ending_inside_its_view_is_refused() {
        assert_eq!(
            StereoVideoGroupBox::decode_payload(b"\0\0\0\0\0\0\0\x05\x80\0"),
            Err(Error::truncated_payload(12, 10))
        );
    }

    #[test]
    fn a_stereo_pair_of_a_version_other_than_0_is_refused() {
        assert_eq!(
            StereoVideoGroupBox::decode_payload(b"\x01\0\0\0\0\0\0\x05\x80\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
