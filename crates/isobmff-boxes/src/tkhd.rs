//! [`TrackHeaderBox`] (`tkhd`), ISO/IEC 14496-12 §8.3.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags, I8F8, Matrix, Mp4EpochSeconds, U16F16,
};

/// Length of the payload when version 0 carries the times in 32 bits
const PAYLOAD_LEN_VERSION_0: u64 = 84;

/// Length of the payload when version 1 carries the times in 64 bits
const PAYLOAD_LEN_VERSION_1: u64 = 96;

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
    creation_time: Mp4EpochSeconds,
    modification_time: Mp4EpochSeconds,
    track_id: u32,
    duration: u64,
    layer: i16,
    alternate_group: i16,
    volume: I8F8,
    matrix: Matrix,
    width: U16F16,
    height: U16F16,
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
        creation_time: Mp4EpochSeconds,
        modification_time: Mp4EpochSeconds,
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
            volume: I8F8::ZERO,
            matrix: Matrix::UNITY,
            width: U16F16::ZERO,
            height: U16F16::ZERO,
        }
    }

    /// Returns the flags stating where the track takes part
    #[must_use]
    pub const fn flags(&self) -> FullBoxFlags {
        self.flags
    }

    /// Returns the time the track was created
    #[must_use]
    pub const fn creation_time(&self) -> Mp4EpochSeconds {
        self.creation_time
    }

    /// Returns the time the track was last modified
    #[must_use]
    pub const fn modification_time(&self) -> Mp4EpochSeconds {
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

    /// Returns the playback volume of the track
    #[must_use]
    pub const fn volume(&self) -> I8F8 {
        self.volume
    }

    /// Returns the transformation matrix the track is rendered under
    #[must_use]
    pub const fn matrix(&self) -> Matrix {
        self.matrix
    }

    /// Returns the visual width the track presents at
    #[must_use]
    pub const fn width(&self) -> U16F16 {
        self.width
    }

    /// Returns the visual height the track presents at
    #[must_use]
    pub const fn height(&self) -> U16F16 {
        self.height
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

impl BoxDefinition for TrackHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tkhd");
}

impl BoxDecode for TrackHeaderBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0 or 1.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let full_box = FullBoxFields::from_bytes(reader.read_bytes::<4>()?);
        let version = full_box.version();
        if version > 1 {
            return Err(Error::unsupported_version(version));
        }
        let field_width = Self::field_width(version);

        let creation_time = Mp4EpochSeconds::from_seconds(reader.read_unsigned(field_width)?);
        let modification_time = Mp4EpochSeconds::from_seconds(reader.read_unsigned(field_width)?);
        let track_id = reader.read_u32()?;
        let _reserved = reader.read_bytes::<4>()?;
        let duration = reader.read_unsigned(field_width)?;
        let _reserved = reader.read_bytes::<8>()?;
        let layer = reader.read_i16()?;
        let alternate_group = reader.read_i16()?;
        let volume = I8F8::from_raw(reader.read_i16()?);
        let _reserved = reader.read_bytes::<2>()?;
        let matrix = Matrix::from_bytes(reader.read_bytes::<36>()?);
        let width = U16F16::from_raw(reader.read_u32()?);
        let height = U16F16::from_raw(reader.read_u32()?);

        Ok(Self {
            flags: full_box.flags(),
            creation_time,
            modification_time,
            track_id,
            duration,
            layer,
            alternate_group,
            volume,
            matrix,
            width,
            height,
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

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let version = self.version();
        let field_width = Self::field_width(version);

        writer.write_bytes(&FullBoxFields::new(version, self.flags).to_bytes())?;
        writer.write_unsigned(field_width, self.creation_time.seconds())?;
        writer.write_unsigned(field_width, self.modification_time.seconds())?;
        writer.write_u32(self.track_id)?;
        writer.write_bytes(&[0; 4])?;
        writer.write_unsigned(field_width, self.duration)?;
        writer.write_bytes(&[0; 8])?;
        writer.write_i16(self.layer)?;
        writer.write_i16(self.alternate_group)?;
        writer.write_i16(self.volume.raw())?;
        writer.write_bytes(&[0; 2])?;
        writer.write_bytes(&self.matrix.to_bytes())?;
        writer.write_u32(self.width.raw())?;
        writer.write_u32(self.height.raw())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FullBoxFlags, Mp4EpochSeconds};

    use super::TrackHeaderBox;

    /// Flags a writer sets on a track that plays as part of the movie
    fn enabled_in_movie() -> FullBoxFlags {
        FullBoxFlags::new(0x3).unwrap()
    }

    /// Track header of the one track a file declares
    fn track_header(duration: u64) -> TrackHeaderBox {
        TrackHeaderBox::new(
            enabled_in_movie(),
            Mp4EpochSeconds::from_seconds(1),
            Mp4EpochSeconds::from_seconds(2),
            1,
            duration,
        )
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
            let track_header = track_header(duration);

            let payload = encoded_payload(&track_header);

            assert_eq!(
                TrackHeaderBox::decode_payload(&payload).unwrap(),
                track_header
            );
        }
    }

    #[test]
    fn the_flags_the_track_declares_survive_a_round_trip() {
        let payload = encoded_payload(&track_header(0));

        assert_eq!(
            TrackHeaderBox::decode_payload(&payload).unwrap().flags(),
            enabled_in_movie()
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 84];
        *payload.first_mut().unwrap() = 2;

        assert_eq!(
            TrackHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_payload_shorter_than_its_version_requires_is_rejected() {
        assert_eq!(
            TrackHeaderBox::decode_payload(&[0; 83]),
            Err(Error::truncated_payload(84, 83))
        );
    }
}
