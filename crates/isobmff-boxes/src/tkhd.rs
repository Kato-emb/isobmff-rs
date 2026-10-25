//! [`TrackHeaderBox`], the characteristics of a single track

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxHeader,
    I8F8, U16F16,
};

use crate::data_types::Matrix;
use crate::full_box::version_width;

/// Flag mask of `track_enabled`
const TRACK_ENABLED: u8 = 0x01;

/// Flag mask of `track_in_movie`
const TRACK_IN_MOVIE: u8 = 0x02;

/// Flag mask of `track_in_preview`
const TRACK_IN_PREVIEW: u8 = 0x04;

/// Flag value of `track_size_is_aspect_ratio`
const TRACK_SIZE_IS_ASPECT_RATIO: u8 = 0x08;

/// [`TrackHeaderBox`] (`tkhd`), ISO/IEC 14496-12 §8.3.2
///
/// The characteristics of a single track.
///
/// The box is written in the `version` it holds: version 0 carries the times
/// and the duration in 32 bits, version 1 in 64. A box read from a file keeps
/// the version it was read in. Of the `flags`, the four the specification
/// defines are held, and the others are read past and written as zero.
///
/// [`Default`] is the box of version 0 with every template field at its
/// template value — `volume` at 0, the value for a track that is not audio —
/// and every other field zero or `false`. A `track_id` of zero reads, but
/// writing it is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `track_ID`,
/// so a box built from [`Default`] is given one before it is written.
#[doc(alias = "tkhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackHeaderBox {
    /// Version of the box, 0 or 1, which sets the width of the times and the duration
    pub version: u8,
    /// Whether the track is enabled; a disabled track is treated as if it were not present
    pub track_enabled: bool,
    /// Whether the track, or one of its alternatives, forms a direct part of the presentation
    pub track_in_movie: bool,
    /// Flag with no assigned meaning, which readers should ignore
    ///
    /// Without further guidance, a writer gives it the value of
    /// `track_in_movie`.
    pub track_in_preview: bool,
    /// Whether `width` and `height` state only an aspect ratio, in units that are not pixels
    pub track_size_is_aspect_ratio: bool,
    /// Creation time of the track, in seconds since midnight, Jan. 1, 1904, in UTC time
    pub creation_time: u64,
    /// Most recent time the track was modified, in seconds since midnight, Jan. 1, 1904, in UTC time
    pub modification_time: u64,
    /// Identifier of the track over the entire life-time of the presentation, which is not zero
    pub track_id: u32,
    /// Duration of the track, in the timescale of the `MovieHeaderBox`
    ///
    /// All bits 1 — `u32::MAX` in version 0, `u64::MAX` in version 1 — means
    /// the duration is indefinite.
    pub duration: u64,
    /// Front-to-back ordering of video tracks; lower numbers are closer to the viewer
    pub layer: i16,
    /// Group of tracks that hold alternate data for one another, 0 for none
    pub alternate_group: i16,
    /// Relative audio volume of the track, 1.0 being full volume
    pub volume: I8F8,
    /// Transformation matrix for the video of the track
    pub matrix: Matrix,
    /// Visual presentation width of the track
    pub width: U16F16,
    /// Visual presentation height of the track
    pub height: U16F16,
}

impl Default for TrackHeaderBox {
    fn default() -> Self {
        Self {
            version: 0,
            track_enabled: false,
            track_in_movie: false,
            track_in_preview: false,
            track_size_is_aspect_ratio: false,
            creation_time: 0,
            modification_time: 0,
            track_id: 0,
            duration: 0,
            layer: 0,
            alternate_group: 0,
            volume: I8F8::ZERO,
            matrix: Matrix::UNITY,
            width: U16F16::ZERO,
            height: U16F16::ZERO,
        }
    }
}

impl BoxDefinition for TrackHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tkhd");
}

impl BoxDecode for TrackHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let width = version_width(header.version())?;
        let [.., flags] = header.flags().bits().to_be_bytes();
        let creation_time = reader.read_unsigned(width)?;
        let modification_time = reader.read_unsigned(width)?;
        let track_id = reader.read_u32()?;
        reader.read_bytes::<4>()?;
        let duration = reader.read_unsigned(width)?;
        reader.read_bytes::<{ 4 * 2 }>()?;
        let layer = reader.read_i16()?;
        let alternate_group = reader.read_i16()?;
        let volume = I8F8::from_raw(reader.read_i16()?);
        reader.read_bytes::<2>()?;

        Ok(Self {
            version: header.version(),
            track_enabled: flags & TRACK_ENABLED != 0,
            track_in_movie: flags & TRACK_IN_MOVIE != 0,
            track_in_preview: flags & TRACK_IN_PREVIEW != 0,
            track_size_is_aspect_ratio: flags & TRACK_SIZE_IS_ASPECT_RATIO != 0,
            creation_time,
            modification_time,
            track_id,
            duration,
            layer,
            alternate_group,
            volume,
            matrix: Matrix::read(reader)?,
            width: U16F16::from_raw(reader.read_u32()?),
            height: U16F16::from_raw(reader.read_u32()?),
        })
    }
}

impl BoxEncode for TrackHeaderBox {
    fn payload_len(&self) -> u64 {
        let times_len: u64 = if self.version == 1 {
            8 + 8 + 4 + 4 + 8
        } else {
            4 + 4 + 4 + 4 + 4
        };

        times_len.saturating_add(4 + 4 * 2 + 2 + 2 + 2 + 2 + Matrix::ENCODED_LEN + 4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        if self.track_id == 0 {
            return Err(Error::invalid_value("track_ID"));
        }
        let flags = [
            (self.track_enabled, TRACK_ENABLED),
            (self.track_in_movie, TRACK_IN_MOVIE),
            (self.track_in_preview, TRACK_IN_PREVIEW),
            (self.track_size_is_aspect_ratio, TRACK_SIZE_IS_ASPECT_RATIO),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |flags, (_, mask)| flags | mask);
        writer.write_bytes(&[self.version, 0, 0, flags])?;
        writer.write_unsigned(width, self.creation_time)?;
        writer.write_unsigned(width, self.modification_time)?;
        writer.write_u32(self.track_id)?;
        writer.write_bytes(&[0; 4])?;
        writer.write_unsigned(width, self.duration)?;
        writer.write_bytes(&[0; 4 * 2])?;
        writer.write_i16(self.layer)?;
        writer.write_i16(self.alternate_group)?;
        writer.write_i16(self.volume.raw())?;
        writer.write_bytes(&[0; 2])?;
        self.matrix.write(writer)?;
        writer.write_u32(self.width.raw())?;
        writer.write_u32(self.height.raw())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth, I8F8, U16F16};

    use super::TrackHeaderBox;
    use crate::data_types::Matrix;
    use crate::encoded;

    /// Fields after `duration`: layer -1, alternate group 1, volume 0, the unity matrix, width 320 and height 240
    const TEMPLATE_FIELDS: &[u8] = b"\0\0\0\0\0\0\0\0\xff\xff\0\x01\0\0\0\0\
        \0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x40\0\0\0\
        \x01\x40\0\0\0\xf0\0\0";

    /// `tkhd` of version 0, enabled and in the movie: created at 1, modified at 2, track 1 and duration 6000
    fn track_header_v0() -> Vec<u8> {
        [
            b"\0\0\0\x5ctkhd\0\0\0\x03".as_slice(),
            b"\0\0\0\x01\0\0\0\x02\0\0\0\x01\0\0\0\0\0\0\x17\x70",
            TEMPLATE_FIELDS,
        ]
        .concat()
    }

    /// `tkhd` of version 1, its size an aspect ratio: created at 1, modified at 2, track 1 and duration 2^33
    fn track_header_v1() -> Vec<u8> {
        [
            b"\0\0\0\x68tkhd\x01\0\0\x08".as_slice(),
            b"\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\x02\0\0\0\x01\0\0\0\0\0\0\0\x02\0\0\0\0",
            TEMPLATE_FIELDS,
        ]
        .concat()
    }

    fn track_header(version: u8, duration: u64) -> TrackHeaderBox {
        TrackHeaderBox {
            version,
            track_enabled: version == 0,
            track_in_movie: version == 0,
            track_in_preview: false,
            track_size_is_aspect_ratio: version == 1,
            creation_time: 1,
            modification_time: 2,
            track_id: 1,
            duration,
            layer: -1,
            alternate_group: 1,
            volume: I8F8::ZERO,
            matrix: Matrix::UNITY,
            width: U16F16::from_integer(320),
            height: U16F16::from_integer(240),
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                TrackHeaderBox::decode(&track_header_v0()),
                TrackHeaderBox::decode(&track_header_v1())
            ),
            (
                Ok((track_header(0, 6000), b"".as_slice())),
                Ok((track_header(1, 1 << 33), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (
                encoded(&track_header(0, 6000)),
                encoded(&track_header(1, 1 << 33))
            ),
            (track_header_v0(), track_header_v1())
        );
    }

    #[test]
    fn flags_the_specification_does_not_define_are_read_past_and_written_as_zero() {
        let mut bytes = track_header_v0();
        bytes.splice(9..12, [0x80, 0, 0x13]);
        let (read, _) = TrackHeaderBox::decode(&bytes).unwrap();

        assert_eq!(encoded(&read), track_header_v0());
    }

    #[test]
    fn a_track_id_of_zero_reads_but_is_refused_when_written() {
        let mut bytes = track_header_v0();
        bytes.splice(20..24, [0; 4]);
        let (read, _) = TrackHeaderBox::decode(&bytes).unwrap();

        assert_eq!(
            (read.track_id, read.encode(&mut [0; 92])),
            (0, Err(Error::invalid_value("track_ID")))
        );
    }

    #[test]
    fn version_0_refuses_a_duration_wider_than_32_bits() {
        assert_eq!(
            track_header(0, 1 << 33).encode(&mut [0; 92]),
            Err(Error::out_of_range(1 << 33, FieldWidth::Compact))
        );
    }

    #[test]
    fn a_payload_ending_inside_height_is_refused() {
        assert_eq!(
            TrackHeaderBox::decode_payload(track_header_v0().get(8..90).unwrap()),
            Err(Error::truncated_payload(84, 82))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        assert_eq!(
            (
                TrackHeaderBox::decode_payload(b"\x02\0\0\x01"),
                track_header(2, 6000).encode(&mut [0; 92])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
