//! [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags, I8F8, I16F16, Matrix, QuickTimeDateTime,
};

/// Length of the payload when version 0 carries the times in 32 bits
const PAYLOAD_LEN_VERSION_0: u64 = 100;

/// Length of the payload when version 1 carries the times in 64 bits
const PAYLOAD_LEN_VERSION_1: u64 = 112;

/// Box that holds the declarations a presentation applies as a whole
///
/// [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2. The `timescale` sets
/// the unit every time in the movie is counted in, and `duration` is the length
/// of the longest track in it. A `moov` carries exactly one.
///
/// The version is not held: it selects how wide the times are written, so
/// [`encode_payload`](BoxEncode::encode_payload) picks the narrower one whenever
/// the times fit in 32 bits. The `flags` are not held either — the spec declares
/// them zero for this box.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::MovieHeaderBox;
/// use isobmff_core::{BoxDecode, BoxWrite, QuickTimeDateTime};
///
/// // A movie of five seconds at millisecond resolution, with one track
/// let epoch = QuickTimeDateTime::from_seconds(0);
/// let movie_header = MovieHeaderBox::new(epoch, epoch, 1_000, 5_000, 2);
///
/// // Times that fit in 32 bits are written at version 0
/// assert_eq!(movie_header.encoded_len(), 108);
///
/// let mut buffer = vec![0; 108];
/// movie_header.encode(&mut buffer).unwrap();
/// assert_eq!(buffer.get(..12).unwrap(), b"\0\0\0lmvhd\0\0\0\0");
///
/// // A duration past the 32-bit limit moves the times to version 1
/// let long_movie = MovieHeaderBox::new(epoch, epoch, 1_000, u64::from(u32::MAX) + 1, 2);
/// assert_eq!(long_movie.encoded_len(), 120);
///
/// // Either way the box reads back as the value that wrote it
/// assert_eq!(
///     MovieHeaderBox::decode_payload(buffer.get(8..).unwrap()).unwrap(),
///     movie_header
/// );
/// ```
#[doc(alias = "mvhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MovieHeaderBox {
    creation_time: QuickTimeDateTime,
    modification_time: QuickTimeDateTime,
    timescale: u32,
    duration: u64,
    rate: I16F16,
    volume: I8F8,
    matrix: Matrix,
    pre_defined: [u32; 6],
    next_track_id: u32,
}

impl MovieHeaderBox {
    /// Creates the box from the declarations that have no template value
    ///
    /// The `rate`, `volume`, and `matrix` take the template values the spec
    /// gives them — normal speed, full volume, and the unity matrix — and
    /// `pre_defined` is left zero.
    #[must_use]
    pub const fn new(
        creation_time: QuickTimeDateTime,
        modification_time: QuickTimeDateTime,
        timescale: u32,
        duration: u64,
        next_track_id: u32,
    ) -> Self {
        Self {
            creation_time,
            modification_time,
            timescale,
            duration,
            rate: I16F16::ONE,
            volume: I8F8::ONE,
            matrix: Matrix::UNITY,
            pre_defined: [0; 6],
            next_track_id,
        }
    }

    /// Returns the time the presentation was created
    #[must_use]
    pub const fn creation_time(&self) -> QuickTimeDateTime {
        self.creation_time
    }

    /// Returns the time the presentation was last modified
    #[must_use]
    pub const fn modification_time(&self) -> QuickTimeDateTime {
        self.modification_time
    }

    /// Returns how many units of the movie's time scale pass in one second
    #[must_use]
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Returns the length of the longest track, in the movie's time scale
    #[must_use]
    pub const fn duration(&self) -> u64 {
        self.duration
    }

    /// Returns the playback rate
    #[must_use]
    pub const fn rate(&self) -> I16F16 {
        self.rate
    }

    /// Returns the playback volume
    #[must_use]
    pub const fn volume(&self) -> I8F8 {
        self.volume
    }

    /// Returns the transformation matrix the presentation is rendered under
    #[must_use]
    pub const fn matrix(&self) -> Matrix {
        self.matrix
    }

    /// Returns the fields the spec reserves for a later definition
    ///
    /// The values are carried through un-inspected: this box reads no meaning
    /// into them and does not zero them on the way out, so a file that puts
    /// data here — as writers in the QuickTime line do — reads back as the
    /// bytes it was written with.
    #[must_use]
    pub const fn pre_defined(&self) -> &[u32; 6] {
        &self.pre_defined
    }

    /// Returns the track identifier no track has and the next one may take
    #[must_use]
    pub const fn next_track_id(&self) -> u32 {
        self.next_track_id
    }

    /// Returns the version whose field widths carry the times of this box
    const fn version(&self) -> u8 {
        let widest = u32::MAX as u64;
        let fits_in_32_bits = self.creation_time.seconds() <= widest
            && self.modification_time.seconds() <= widest
            && self.duration <= widest;

        if fits_in_32_bits { 0 } else { 1 }
    }

    /// Returns the width the given version carries the times of this box at
    const fn field_width(version: u8) -> FieldWidth {
        match version {
            0 => FieldWidth::Compact,
            _ => FieldWidth::Extended,
        }
    }
}

impl BoxDefinition for MovieHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mvhd");
}

impl BoxDecode for MovieHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let field_width = Self::field_width(version);

        let creation_time = QuickTimeDateTime::from_seconds(reader.read_unsigned(field_width)?);
        let modification_time = QuickTimeDateTime::from_seconds(reader.read_unsigned(field_width)?);
        let timescale = reader.read_u32()?;
        let duration = reader.read_unsigned(field_width)?;
        let rate = I16F16::from_raw(reader.read_i32()?);
        let volume = I8F8::from_raw(reader.read_i16()?);
        let _reserved = reader.read_bytes::<10>()?;
        let matrix = Matrix::from_bytes(reader.read_bytes::<36>()?);
        let mut pre_defined = [0; 6];
        for field in &mut pre_defined {
            *field = reader.read_u32()?;
        }
        let next_track_id = reader.read_u32()?;

        Ok(Self {
            creation_time,
            modification_time,
            timescale,
            duration,
            rate,
            volume,
            matrix,
            pre_defined,
            next_track_id,
        })
    }
}

impl BoxEncode for MovieHeaderBox {
    fn payload_len(&self) -> u64 {
        if self.version() == 0 {
            PAYLOAD_LEN_VERSION_0
        } else {
            PAYLOAD_LEN_VERSION_1
        }
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = self.version();
        let field_width = Self::field_width(version);

        writer.write_bytes(&FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes())?;
        writer.write_unsigned(field_width, self.creation_time.seconds())?;
        writer.write_unsigned(field_width, self.modification_time.seconds())?;
        writer.write_u32(self.timescale)?;
        writer.write_unsigned(field_width, self.duration)?;
        writer.write_i32(self.rate.raw())?;
        writer.write_i16(self.volume.raw())?;
        writer.write_bytes(&[0; 10])?;
        writer.write_bytes(&self.matrix.to_bytes())?;
        for field in self.pre_defined {
            writer.write_u32(field)?;
        }
        writer.write_u32(self.next_track_id)?;

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, BoxWrite as _, Error, QuickTimeDateTime};

    use super::MovieHeaderBox;

    /// Movie header carrying the times a file written at the epoch declares
    pub(crate) fn movie_header(duration: u64) -> MovieHeaderBox {
        MovieHeaderBox::new(
            QuickTimeDateTime::from_seconds(1),
            QuickTimeDateTime::from_seconds(2),
            1_000,
            duration,
            3,
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(movie_header: &MovieHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(movie_header.payload_len()).unwrap()];
        movie_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn times_within_32_bits_are_written_at_version_0() {
        let payload = encoded_payload(&movie_header(u64::from(u32::MAX)));

        assert_eq!(payload.len(), 100);
        assert_eq!(payload.first(), Some(&0));
    }

    #[test]
    fn a_time_past_32_bits_moves_every_time_to_version_1() {
        let payload = encoded_payload(&movie_header(u64::from(u32::MAX) + 1));

        assert_eq!(payload.len(), 112);
        assert_eq!(payload.first(), Some(&1));
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it_at_either_version() {
        for duration in [u64::from(u32::MAX), u64::from(u32::MAX) + 1] {
            let movie_header = movie_header(duration);

            let payload = encoded_payload(&movie_header);

            assert_eq!(
                MovieHeaderBox::decode_payload(&payload).unwrap(),
                movie_header
            );
        }
    }

    #[test]
    fn the_fields_the_spec_reserves_survive_a_round_trip() {
        let payload = {
            let mut payload = encoded_payload(&movie_header(0));
            payload
                .get_mut(72..96)
                .unwrap()
                .copy_from_slice(&[0xab; 24]);
            payload
        };

        let movie_header = MovieHeaderBox::decode_payload(&payload).unwrap();

        assert_eq!(movie_header.pre_defined(), &[0xabab_abab_u32; 6]);
        assert_eq!(encoded_payload(&movie_header), payload);
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = encoded_payload(&movie_header(0));
        *payload.first_mut().unwrap() = 2;

        assert_eq!(
            MovieHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_payload_shorter_than_its_version_requires_is_rejected() {
        let payload = encoded_payload(&movie_header(0));

        assert_eq!(
            MovieHeaderBox::decode_payload(payload.get(..99).unwrap()),
            Err(Error::truncated_payload(100, 99))
        );
    }

    #[test]
    fn a_payload_longer_than_its_version_requires_is_rejected() {
        let mut payload = encoded_payload(&movie_header(0));
        payload.push(0);

        assert_eq!(
            MovieHeaderBox::decode_payload(&payload),
            Err(Error::trailing_payload(100, 101))
        );
    }

    #[test]
    fn the_whole_box_counts_its_header_on_top_of_the_payload() {
        assert_eq!(movie_header(0).encoded_len(), 108);
    }
}
