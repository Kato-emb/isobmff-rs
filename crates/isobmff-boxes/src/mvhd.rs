//! [`MovieHeaderBox`], the media-independent information of a presentation

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FullBoxFlags,
    FullBoxHeader, I8F8, I16F16,
};

use crate::data_types::Matrix;
use crate::full_box::version_width;

/// [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2
///
/// Information that is media-independent and relevant to the presentation as
/// a whole.
///
/// The box is written in the `version` it holds: version 0 carries the times
/// and the duration in 32 bits, version 1 in 64. A box read from a file keeps
/// the version it was read in.
///
/// [`Default`] is the box of version 0 with every template field at its
/// template value and every other field zero. A `next_track_id` of zero
/// reads, but writing it is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `next_track_ID`, so a box built from [`Default`] is given one before it
/// is written.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::MovieHeaderBox;
/// use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};
///
/// // A box is built from its default, field by field
/// let mut movie_header = MovieHeaderBox::default();
/// movie_header.timescale = 1000;
/// movie_header.duration = 5000;
/// movie_header.next_track_id = 2;
///
/// // It reads back from the bytes it writes
/// let mut buffer = [0; 108];
/// movie_header.encode(&mut buffer).unwrap();
/// assert_eq!(MovieHeaderBox::decode(&buffer), Ok((movie_header.clone(), b"".as_slice())));
///
/// // Version 0 cannot carry a duration of more than 32 bits
/// movie_header.duration = 1 << 32;
/// assert_eq!(
///     movie_header.encode(&mut buffer),
///     Err(Error::out_of_range(1 << 32, FieldWidth::Compact))
/// );
/// ```
#[doc(alias = "mvhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MovieHeaderBox {
    /// Version of the box, 0 or 1, which sets the width of the times and the duration
    pub version: u8,
    /// Creation time of the presentation, in seconds since midnight, Jan. 1, 1904, in UTC time
    pub creation_time: u64,
    /// Most recent time the presentation was modified, in seconds since midnight, Jan. 1, 1904, in UTC time
    pub modification_time: u64,
    /// Number of time units that pass in one second, for the presentation as a whole
    pub timescale: u32,
    /// Length of the presentation, in `timescale` units
    ///
    /// All bits 1 — `u32::MAX` in version 0, `u64::MAX` in version 1 — means
    /// the duration cannot be determined.
    pub duration: u64,
    /// Preferred rate to play the presentation at, 1.0 being normal forward playback
    pub rate: I16F16,
    /// Preferred playback volume, 1.0 being full volume
    pub volume: I8F8,
    /// Transformation matrix for the video of the presentation
    pub matrix: Matrix,
    /// Value to use for the `track_ID` of the next track added to the presentation, which is not zero
    ///
    /// All bits 1 means a writer adding a track searches the file for an
    /// unused `track_ID`.
    pub next_track_id: u32,
}

impl Default for MovieHeaderBox {
    fn default() -> Self {
        Self {
            version: 0,
            creation_time: 0,
            modification_time: 0,
            timescale: 0,
            duration: 0,
            rate: I16F16::ONE,
            volume: I8F8::ONE,
            matrix: Matrix::UNITY,
            next_track_id: 0,
        }
    }
}

impl BoxDefinition for MovieHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mvhd");
}

impl BoxDecode for MovieHeaderBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?).version();
        let width = version_width(version)?;
        let creation_time = reader.read_unsigned(width)?;
        let modification_time = reader.read_unsigned(width)?;
        let timescale = reader.read_u32()?;
        let duration = reader.read_unsigned(width)?;
        let rate = I16F16::from_raw(reader.read_i32()?);
        let volume = I8F8::from_raw(reader.read_i16()?);
        reader.read_bytes::<{ 2 + 4 * 2 }>()?;
        let matrix = Matrix::read(reader)?;
        reader.read_bytes::<{ 4 * 6 }>()?;

        Ok(Self {
            version,
            creation_time,
            modification_time,
            timescale,
            duration,
            rate,
            volume,
            matrix,
            next_track_id: reader.read_u32()?,
        })
    }
}

impl BoxEncode for MovieHeaderBox {
    fn payload_len(&self) -> u64 {
        let times_len: u64 = if self.version == 1 {
            8 + 8 + 4 + 8
        } else {
            4 + 4 + 4 + 4
        };

        times_len.saturating_add(4 + 4 + 2 + 2 + 4 * 2 + Matrix::ENCODED_LEN + 4 * 6 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        if self.next_track_id == 0 {
            return Err(Error::invalid_value("next_track_ID"));
        }
        writer.write_bytes(&FullBoxHeader::new(self.version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_unsigned(width, self.creation_time)?;
        writer.write_unsigned(width, self.modification_time)?;
        writer.write_u32(self.timescale)?;
        writer.write_unsigned(width, self.duration)?;
        writer.write_i32(self.rate.raw())?;
        writer.write_i16(self.volume.raw())?;
        writer.write_bytes(&[0; 2 + 4 * 2])?;
        self.matrix.write(writer)?;
        writer.write_bytes(&[0; 4 * 6])?;
        writer.write_u32(self.next_track_id)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth, I8F8, I16F16};

    use super::MovieHeaderBox;
    use crate::data_types::Matrix;
    use crate::encoded;

    /// Fields after `duration`: rate 1.0, volume 1.0, the unity matrix and `next_track_ID` 3
    const TEMPLATE_FIELDS: &[u8] = b"\0\x01\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\
        \0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x40\0\0\0\
        \0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x03";

    /// `mvhd` of version 0: created at 1, modified at 2, timescale 600 and duration 6000
    fn movie_header_v0() -> Vec<u8> {
        [
            b"\0\0\0\x6cmvhd\0\0\0\0".as_slice(),
            b"\0\0\0\x01\0\0\0\x02\0\0\x02\x58\0\0\x17\x70",
            TEMPLATE_FIELDS,
        ]
        .concat()
    }

    /// `mvhd` of version 1: created at 2^32, modified at 2^32 + 1, timescale 600 and duration 2^33
    fn movie_header_v1() -> Vec<u8> {
        [
            b"\0\0\0\x78mvhd\x01\0\0\0".as_slice(),
            b"\0\0\0\x01\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\x02\x58\0\0\0\x02\0\0\0\0",
            TEMPLATE_FIELDS,
        ]
        .concat()
    }

    fn movie_header(version: u8, times: [u64; 2], duration: u64) -> MovieHeaderBox {
        MovieHeaderBox {
            version,
            creation_time: times[0],
            modification_time: times[1],
            timescale: 600,
            duration,
            rate: I16F16::ONE,
            volume: I8F8::ONE,
            matrix: Matrix::UNITY,
            next_track_id: 3,
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (
                MovieHeaderBox::decode(&movie_header_v0()),
                MovieHeaderBox::decode(&movie_header_v1())
            ),
            (
                Ok((movie_header(0, [1, 2], 6000), b"".as_slice())),
                Ok((
                    movie_header(1, [1 << 32, (1 << 32) + 1], 1 << 33),
                    b"".as_slice()
                ))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (
                encoded(&movie_header(0, [1, 2], 6000)),
                encoded(&movie_header(1, [1 << 32, (1 << 32) + 1], 1 << 33))
            ),
            (movie_header_v0(), movie_header_v1())
        );
    }

    #[test]
    fn version_0_refuses_a_time_wider_than_32_bits() {
        let mut buffer = [0; 108];

        assert_eq!(
            movie_header(0, [1 << 32, 2], 6000).encode(&mut buffer),
            Err(Error::out_of_range(1 << 32, FieldWidth::Compact))
        );
    }

    #[test]
    fn a_next_track_id_of_zero_reads_but_is_refused_when_written() {
        let mut bytes = movie_header_v0();
        bytes.splice(104..108, [0; 4]);
        let (read, _) = MovieHeaderBox::decode(&bytes).unwrap();

        assert_eq!(
            (read.next_track_id, read.encode(&mut [0; 108])),
            (0, Err(Error::invalid_value("next_track_ID")))
        );
    }

    #[test]
    fn a_payload_ending_inside_next_track_id_is_refused() {
        assert_eq!(
            MovieHeaderBox::decode_payload(movie_header_v0().get(8..106).unwrap()),
            Err(Error::truncated_payload(100, 98))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        assert_eq!(
            (
                MovieHeaderBox::decode_payload(b"\x02\0\0\0"),
                movie_header(2, [1, 2], 6000).encode(&mut [0; 108])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
