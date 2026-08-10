//! [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FullBoxFields,
    FullBoxFlags,
};

use crate::field::{
    check_payload_len, split_field, split_field_mut, split_i32_array, split_time, split_u32_array,
    write_i32_array, write_time, write_u32_array,
};

/// Length of the payload when version 0 carries the times in 32 bits
const PAYLOAD_LEN_VERSION_0: u64 = 100;

/// Length of the payload when version 1 carries the times in 64 bits
const PAYLOAD_LEN_VERSION_1: u64 = 112;

/// Transformation matrix the spec gives as the template value
const UNITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// Playback rate the spec gives as the template value, 1.0 in 16.16 fixed point
const NORMAL_RATE: i32 = 0x0001_0000;

/// Playback volume the spec gives as the template value, 1.0 in 8.8 fixed point
const FULL_VOLUME: i16 = 0x0100;

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
/// use isobmff_core::{BoxDecode, BoxWrite};
///
/// // A movie of five seconds at millisecond resolution, with one track
/// let movie_header = MovieHeaderBox::new(0, 0, 1_000, 5_000, 2);
///
/// // Times that fit in 32 bits are written at version 0
/// assert_eq!(movie_header.encoded_len(), 108);
///
/// let mut buffer = vec![0; 108];
/// movie_header.encode(&mut buffer).unwrap();
/// assert_eq!(buffer.get(..12).unwrap(), b"\0\0\0lmvhd\0\0\0\0");
///
/// // A duration past the 32-bit limit moves the times to version 1
/// let long_movie = MovieHeaderBox::new(0, 0, 1_000, u64::from(u32::MAX) + 1, 2);
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
    creation_time: u64,
    modification_time: u64,
    timescale: u32,
    duration: u64,
    rate: i32,
    volume: i16,
    matrix: [i32; 9],
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
        creation_time: u64,
        modification_time: u64,
        timescale: u32,
        duration: u64,
        next_track_id: u32,
    ) -> Self {
        Self {
            creation_time,
            modification_time,
            timescale,
            duration,
            rate: NORMAL_RATE,
            volume: FULL_VOLUME,
            matrix: UNITY_MATRIX,
            pre_defined: [0; 6],
            next_track_id,
        }
    }

    /// Returns the time the presentation was created, in seconds since 1904-01-01
    #[must_use]
    pub const fn creation_time(&self) -> u64 {
        self.creation_time
    }

    /// Returns the time the presentation was last modified, on the same scale
    #[must_use]
    pub const fn modification_time(&self) -> u64 {
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

    /// Returns the playback rate, as 16.16 fixed point
    #[must_use]
    pub const fn rate(&self) -> i32 {
        self.rate
    }

    /// Returns the playback volume, as 8.8 fixed point
    #[must_use]
    pub const fn volume(&self) -> i16 {
        self.volume
    }

    /// Returns the transformation matrix the presentation is rendered under
    #[must_use]
    pub const fn matrix(&self) -> &[i32; 9] {
        &self.matrix
    }

    /// Returns the fields the spec reserves for a later definition
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
        let fits_in_32_bits = self.creation_time <= widest
            && self.modification_time <= widest
            && self.duration <= widest;

        if fits_in_32_bits { 0 } else { 1 }
    }
}

impl BoxDefinition for MovieHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mvhd");
}

impl BoxDecode for MovieHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](DecodeError::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](DecodeError::TruncatedPayload): the payload is
    ///   shorter than the version it declares requires.
    /// * [`TrailingBytes`](DecodeError::TrailingBytes): the payload is longer.
    fn decode_payload(payload: &[u8]) -> Result<Self, DecodeError> {
        let available = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        let (full_box_field, rest) = split_field::<4>(payload, 4, available)?;
        let version = FullBoxFields::from_bytes(full_box_field).version();

        let needed = match version {
            0 => PAYLOAD_LEN_VERSION_0,
            1 => PAYLOAD_LEN_VERSION_1,
            unsupported => return Err(DecodeError::UnsupportedVersion(unsupported)),
        };
        check_payload_len(needed, available)?;

        let (creation_time, rest) = split_time(version, rest, needed, available)?;
        let (modification_time, rest) = split_time(version, rest, needed, available)?;
        let (timescale_field, rest) = split_field::<4>(rest, needed, available)?;
        let (duration, rest) = split_time(version, rest, needed, available)?;
        let (rate_field, rest) = split_field::<4>(rest, needed, available)?;
        let (volume_field, rest) = split_field::<2>(rest, needed, available)?;
        let (_reserved, rest) = split_field::<10>(rest, needed, available)?;
        let (matrix, rest) = split_i32_array::<9>(rest, needed, available)?;
        let (pre_defined, rest) = split_u32_array::<6>(rest, needed, available)?;
        let (next_track_id_field, _rest) = split_field::<4>(rest, needed, available)?;

        Ok(Self {
            creation_time,
            modification_time,
            timescale: u32::from_be_bytes(*timescale_field),
            duration,
            rate: i32::from_be_bytes(*rate_field),
            volume: i16::from_be_bytes(*volume_field),
            matrix,
            pre_defined,
            next_track_id: u32::from_be_bytes(*next_track_id_field),
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

    fn encode_payload(&self, buffer: &mut [u8]) -> Result<(), EncodeError> {
        let expected = self.payload_len();
        let actual = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        let mismatch = EncodeError::BufferLengthMismatch { expected, actual };
        if actual != expected {
            return Err(mismatch);
        }

        let version = self.version();
        let (full_box_field, rest) = split_field_mut::<4>(buffer, mismatch)?;
        *full_box_field = FullBoxFields::new(version, FullBoxFlags::ZERO).to_bytes();

        let rest = write_time(version, self.creation_time, rest, mismatch)?;
        let rest = write_time(version, self.modification_time, rest, mismatch)?;
        let (timescale_field, rest) = split_field_mut::<4>(rest, mismatch)?;
        *timescale_field = self.timescale.to_be_bytes();
        let rest = write_time(version, self.duration, rest, mismatch)?;
        let (rate_field, rest) = split_field_mut::<4>(rest, mismatch)?;
        *rate_field = self.rate.to_be_bytes();
        let (volume_field, rest) = split_field_mut::<2>(rest, mismatch)?;
        *volume_field = self.volume.to_be_bytes();
        let (reserved_field, rest) = split_field_mut::<10>(rest, mismatch)?;
        *reserved_field = [0; 10];
        let rest = write_i32_array(&self.matrix, rest, mismatch)?;
        let rest = write_u32_array(&self.pre_defined, rest, mismatch)?;
        let (next_track_id_field, _rest) = split_field_mut::<4>(rest, mismatch)?;
        *next_track_id_field = self.next_track_id.to_be_bytes();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, BoxWrite as _, DecodeError};

    use super::MovieHeaderBox;

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(movie_header: &MovieHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(movie_header.payload_len()).unwrap()];
        movie_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn times_within_32_bits_are_written_at_version_0() {
        let movie_header = MovieHeaderBox::new(1, 2, 1_000, u64::from(u32::MAX), 3);

        let payload = encoded_payload(&movie_header);

        assert_eq!(payload.len(), 100);
        assert_eq!(payload.first(), Some(&0));
    }

    #[test]
    fn a_time_past_32_bits_moves_every_time_to_version_1() {
        let movie_header = MovieHeaderBox::new(1, 2, 1_000, u64::from(u32::MAX) + 1, 3);

        let payload = encoded_payload(&movie_header);

        assert_eq!(payload.len(), 112);
        assert_eq!(payload.first(), Some(&1));
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it_at_either_version() {
        for duration in [u64::from(u32::MAX), u64::from(u32::MAX) + 1] {
            let movie_header = MovieHeaderBox::new(1, 2, 1_000, duration, 3);

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
            let mut payload = encoded_payload(&MovieHeaderBox::new(0, 0, 1, 0, 1));
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
        let mut payload = encoded_payload(&MovieHeaderBox::new(0, 0, 1, 0, 1));
        *payload.first_mut().unwrap() = 2;

        assert!(matches!(
            MovieHeaderBox::decode_payload(&payload),
            Err(DecodeError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn a_payload_shorter_than_its_version_requires_is_rejected() {
        let payload = encoded_payload(&MovieHeaderBox::new(0, 0, 1, 0, 1));

        assert!(matches!(
            MovieHeaderBox::decode_payload(payload.get(..99).unwrap()),
            Err(DecodeError::TruncatedPayload {
                needed: 100,
                available: 99
            })
        ));
    }

    #[test]
    fn a_payload_longer_than_its_version_requires_is_rejected() {
        let mut payload = encoded_payload(&MovieHeaderBox::new(0, 0, 1, 0, 1));
        payload.push(0);

        assert!(matches!(
            MovieHeaderBox::decode_payload(&payload),
            Err(DecodeError::TrailingBytes { remaining: 1 })
        ));
    }

    #[test]
    fn the_whole_box_counts_its_header_on_top_of_the_payload() {
        assert_eq!(MovieHeaderBox::new(0, 0, 1, 0, 1).encoded_len(), 108);
    }
}
