//! [`TrackHeaderBox`] (`tkhd`), ISO/IEC 14496-12 §8.3.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, DecodeError, EncodeError, FullBoxFields,
    FullBoxFlags,
};

use crate::field::{
    check_payload_len, split_field, split_field_mut, split_i32_array, split_time, write_i32_array,
    write_time,
};

/// Length of the payload when version 0 carries the times in 32 bits
const PAYLOAD_LEN_VERSION_0: u64 = 84;

/// Length of the payload when version 1 carries the times in 64 bits
const PAYLOAD_LEN_VERSION_1: u64 = 96;

/// Transformation matrix the spec gives as the template value
const UNITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// Box that holds the declarations one track applies as a whole
///
/// [`TrackHeaderBox`] (`tkhd`), ISO/IEC 14496-12 §8.3.2. Unlike the other header
/// boxes this one carries meaningful `flags` — `track_enabled`,
/// `track_in_movie`, `track_in_preview`, and `track_size_is_aspect_ratio` — so
/// they are held and written back as they were read.
///
/// The version is not held: it selects how wide the times are written, so
/// [`encode_payload`](BoxEncode::encode_payload) picks the narrower one whenever
/// the times fit in 32 bits.
#[doc(alias = "tkhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackHeaderBox {
    flags: FullBoxFlags,
    creation_time: u64,
    modification_time: u64,
    track_id: u32,
    duration: u64,
    layer: i16,
    alternate_group: i16,
    volume: i16,
    matrix: [i32; 9],
    width: u32,
    height: u32,
}

impl TrackHeaderBox {
    /// Creates the box from the declarations that have no template value
    ///
    /// The `layer`, `alternate_group`, `volume`, `width`, and `height` are left
    /// at zero and the `matrix` takes the unity matrix the spec gives it. A
    /// track that is audio, or that has a visual size, states those afterwards.
    #[must_use]
    pub const fn new(
        flags: FullBoxFlags,
        creation_time: u64,
        modification_time: u64,
        track_id: u32,
        duration: u64,
    ) -> Self {
        Self {
            flags,
            creation_time,
            modification_time,
            track_id,
            duration,
            layer: 0,
            alternate_group: 0,
            volume: 0,
            matrix: UNITY_MATRIX,
            width: 0,
            height: 0,
        }
    }

    /// Returns the flags stating where the track takes part
    #[must_use]
    pub const fn flags(&self) -> FullBoxFlags {
        self.flags
    }

    /// Returns the time the track was created, in seconds since 1904-01-01
    #[must_use]
    pub const fn creation_time(&self) -> u64 {
        self.creation_time
    }

    /// Returns the time the track was last modified, on the same scale
    #[must_use]
    pub const fn modification_time(&self) -> u64 {
        self.modification_time
    }

    /// Returns the identifier that tells this track from the others
    #[must_use]
    pub const fn track_id(&self) -> u32 {
        self.track_id
    }

    /// Returns the length of the track, in the movie's time scale
    #[must_use]
    pub const fn duration(&self) -> u64 {
        self.duration
    }

    /// Returns the front-to-back ordering of this track against the others
    #[must_use]
    pub const fn layer(&self) -> i16 {
        self.layer
    }

    /// Returns the group of tracks only one of which is played at a time
    #[must_use]
    pub const fn alternate_group(&self) -> i16 {
        self.alternate_group
    }

    /// Returns the playback volume of the track, as 8.8 fixed point
    #[must_use]
    pub const fn volume(&self) -> i16 {
        self.volume
    }

    /// Returns the transformation matrix the track is rendered under
    #[must_use]
    pub const fn matrix(&self) -> &[i32; 9] {
        &self.matrix
    }

    /// Returns the visual width the track presents at, as 16.16 fixed point
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the visual height the track presents at, as 16.16 fixed point
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
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

impl BoxDefinition for TrackHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tkhd");
}

impl BoxDecode for TrackHeaderBox {
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
        let full_box = FullBoxFields::from_bytes(full_box_field);

        let needed = match full_box.version() {
            0 => PAYLOAD_LEN_VERSION_0,
            1 => PAYLOAD_LEN_VERSION_1,
            unsupported => return Err(DecodeError::UnsupportedVersion(unsupported)),
        };
        check_payload_len(needed, available)?;

        let version = full_box.version();
        let (creation_time, rest) = split_time(version, rest, needed, available)?;
        let (modification_time, rest) = split_time(version, rest, needed, available)?;
        let (track_id, rest) = split_field::<4>(rest, needed, available)?;
        let (_reserved, rest) = split_field::<4>(rest, needed, available)?;
        let (duration, rest) = split_time(version, rest, needed, available)?;
        let (_reserved, rest) = split_field::<8>(rest, needed, available)?;
        let (layer, rest) = split_field::<2>(rest, needed, available)?;
        let (alternate_group, rest) = split_field::<2>(rest, needed, available)?;
        let (volume, rest) = split_field::<2>(rest, needed, available)?;
        let (_reserved, rest) = split_field::<2>(rest, needed, available)?;
        let (matrix, rest) = split_i32_array::<9>(rest, needed, available)?;
        let (width, rest) = split_field::<4>(rest, needed, available)?;
        let (height, _rest) = split_field::<4>(rest, needed, available)?;

        Ok(Self {
            flags: full_box.flags(),
            creation_time,
            modification_time,
            track_id: u32::from_be_bytes(*track_id),
            duration,
            layer: i16::from_be_bytes(*layer),
            alternate_group: i16::from_be_bytes(*alternate_group),
            volume: i16::from_be_bytes(*volume),
            matrix,
            width: u32::from_be_bytes(*width),
            height: u32::from_be_bytes(*height),
        })
    }
}

impl BoxEncode for TrackHeaderBox {
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
        *full_box_field = FullBoxFields::new(version, self.flags).to_bytes();

        let rest = write_time(version, self.creation_time, rest, mismatch)?;
        let rest = write_time(version, self.modification_time, rest, mismatch)?;
        let (track_id, rest) = split_field_mut::<4>(rest, mismatch)?;
        *track_id = self.track_id.to_be_bytes();
        let (reserved, rest) = split_field_mut::<4>(rest, mismatch)?;
        *reserved = [0; 4];
        let rest = write_time(version, self.duration, rest, mismatch)?;
        let (reserved, rest) = split_field_mut::<8>(rest, mismatch)?;
        *reserved = [0; 8];
        let (layer, rest) = split_field_mut::<2>(rest, mismatch)?;
        *layer = self.layer.to_be_bytes();
        let (alternate_group, rest) = split_field_mut::<2>(rest, mismatch)?;
        *alternate_group = self.alternate_group.to_be_bytes();
        let (volume, rest) = split_field_mut::<2>(rest, mismatch)?;
        *volume = self.volume.to_be_bytes();
        let (reserved, rest) = split_field_mut::<2>(rest, mismatch)?;
        *reserved = [0; 2];
        let rest = write_i32_array(&self.matrix, rest, mismatch)?;
        let (width, rest) = split_field_mut::<4>(rest, mismatch)?;
        *width = self.width.to_be_bytes();
        let (height, _rest) = split_field_mut::<4>(rest, mismatch)?;
        *height = self.height.to_be_bytes();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, DecodeError, FullBoxFlags};

    use super::TrackHeaderBox;

    /// Flags a writer sets on a track that plays as part of the movie
    fn enabled_in_movie() -> FullBoxFlags {
        FullBoxFlags::new(0x3).unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(track_header: &TrackHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(track_header.payload_len()).unwrap()];
        track_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it_at_either_version() {
        for duration in [u64::from(u32::MAX), u64::from(u32::MAX) + 1] {
            let track_header = TrackHeaderBox::new(enabled_in_movie(), 1, 2, 1, duration);

            let payload = encoded_payload(&track_header);

            assert_eq!(
                TrackHeaderBox::decode_payload(&payload).unwrap(),
                track_header
            );
        }
    }

    #[test]
    fn the_flags_the_track_declares_survive_a_round_trip() {
        let track_header = TrackHeaderBox::new(enabled_in_movie(), 0, 0, 1, 0);

        let payload = encoded_payload(&track_header);

        assert_eq!(
            TrackHeaderBox::decode_payload(&payload).unwrap().flags(),
            enabled_in_movie()
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 84];
        *payload.first_mut().unwrap() = 2;

        assert!(matches!(
            TrackHeaderBox::decode_payload(&payload),
            Err(DecodeError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn a_payload_shorter_than_its_version_requires_is_rejected() {
        assert!(matches!(
            TrackHeaderBox::decode_payload(&[0; 83]),
            Err(DecodeError::TruncatedPayload {
                needed: 84,
                available: 83
            })
        ));
    }
}
