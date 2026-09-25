//! [`MediaHeaderBox`] (`mdhd`), ISO/IEC 14496-12 §8.4.2

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags, LanguageCode, Mp4EpochSeconds,
};

use crate::data_types::HeaderDuration;

/// Length of the payload when version 0 carries the times in 32 bits
const PAYLOAD_LEN_VERSION_0: u64 = 24;

/// Length of the payload when version 1 carries the times in 64 bits
const PAYLOAD_LEN_VERSION_1: u64 = 36;

/// Box that holds the declarations the media of one track applies
///
/// [`MediaHeaderBox`] (`mdhd`), ISO/IEC 14496-12 §8.4.2. The `timescale` here is
/// the track's own, which every sample time in the track is counted in — it is
/// not the movie's, so a reader converting between the two goes through both.
///
/// The version is not held: it selects how wide the times are written, so
/// [`encode_payload`](BoxEncode::encode_payload) picks the narrower one whenever
/// the times fit in 32 bits.
///
/// The `duration` is a [`HeaderDuration`], which states one that cannot be
/// determined as well (§8.4.2.3).
#[doc(alias = "mdhd")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MediaHeaderBox {
    creation_time: Mp4EpochSeconds,
    modification_time: Mp4EpochSeconds,
    timescale: u32,
    duration: HeaderDuration,
    language: LanguageCode,
    pre_defined: u16,
}

impl MediaHeaderBox {
    /// Creates the box from the declarations of one track's media
    #[must_use]
    pub const fn new(
        creation_time: Mp4EpochSeconds,
        modification_time: Mp4EpochSeconds,
        timescale: u32,
        duration: HeaderDuration,
        language: LanguageCode,
    ) -> Self {
        Self {
            creation_time,
            modification_time,
            timescale,
            duration,
            language,
            pre_defined: 0,
        }
    }

    /// Sets the time the media was created
    #[must_use]
    pub const fn with_creation_time(self, creation_time: Mp4EpochSeconds) -> Self {
        Self {
            creation_time,
            ..self
        }
    }

    /// Sets the time the media was last modified
    #[must_use]
    pub const fn with_modification_time(self, modification_time: Mp4EpochSeconds) -> Self {
        Self {
            modification_time,
            ..self
        }
    }

    /// Sets the length of the track, in the track's own time scale
    #[must_use]
    pub const fn with_duration(self, duration: HeaderDuration) -> Self {
        Self { duration, ..self }
    }

    /// Returns the time the media was created
    #[must_use]
    pub const fn creation_time(&self) -> Mp4EpochSeconds {
        self.creation_time
    }

    /// Returns the time the media was last modified
    #[must_use]
    pub const fn modification_time(&self) -> Mp4EpochSeconds {
        self.modification_time
    }

    /// Returns how many units of the track's time scale pass in one second
    #[must_use]
    pub const fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Returns the length of the track, in the track's own time scale
    #[must_use]
    pub const fn duration(&self) -> HeaderDuration {
        self.duration
    }

    /// Returns the language the media is in
    #[must_use]
    pub const fn language(&self) -> LanguageCode {
        self.language
    }

    /// Returns the field the spec reserves for a later definition
    ///
    /// The value is carried through un-inspected: this box reads no meaning
    /// into it and does not zero it on the way out, so a file that puts data
    /// here — as writers in the QuickTime line do — reads back as the bytes it
    /// was written with.
    #[must_use]
    pub const fn pre_defined(&self) -> u16 {
        self.pre_defined
    }

    /// Returns the version whose field widths carry the times of this box
    const fn version(&self) -> u8 {
        let widest = u32::MAX as u64;
        let fits_in_32_bits = self.creation_time.seconds() <= widest
            && self.modification_time.seconds() <= widest
            && self.duration.fits_in_32_bits();

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

impl BoxDefinition for MediaHeaderBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"mdhd");
}

impl BoxDecode for MediaHeaderBox {
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

        let creation_time = Mp4EpochSeconds::from_seconds(reader.read_unsigned(field_width)?);
        let modification_time = Mp4EpochSeconds::from_seconds(reader.read_unsigned(field_width)?);
        let timescale = reader.read_u32()?;
        let duration = HeaderDuration::read(reader, field_width)?;
        let language = LanguageCode::from_raw(reader.read_u16()?);
        let pre_defined = reader.read_u16()?;

        Ok(Self {
            creation_time,
            modification_time,
            timescale,
            duration,
            language,
            pre_defined,
        })
    }
}

impl BoxEncode for MediaHeaderBox {
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
        self.duration.write(writer, field_width)?;
        writer.write_u16(self.language.raw())?;
        writer.write_u16(self.pre_defined)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, LanguageCode, Mp4EpochSeconds};

    use super::{HeaderDuration, MediaHeaderBox};

    /// Media header of a track whose language is left undetermined
    fn media_header(duration: u64) -> MediaHeaderBox {
        MediaHeaderBox::new(
            Mp4EpochSeconds::from_seconds(1),
            Mp4EpochSeconds::from_seconds(2),
            90_000,
            HeaderDuration::new(duration).unwrap(),
            LanguageCode::UND,
        )
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(media_header: &MediaHeaderBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(media_header.payload_len()).unwrap()];
        media_header.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it_at_either_version() {
        let early = Mp4EpochSeconds::from_seconds(0);
        let late = Mp4EpochSeconds::from_seconds(u64::from(u32::MAX) + 1);
        let cases = [
            (HeaderDuration::new(u64::from(u32::MAX) - 1).unwrap(), early),
            (HeaderDuration::new(u64::from(u32::MAX)).unwrap(), early),
            (HeaderDuration::INDETERMINATE, early),
            (HeaderDuration::INDETERMINATE, late),
        ];

        for (duration, creation_time) in cases {
            let media_header = media_header(0)
                .with_duration(duration)
                .with_creation_time(creation_time);

            let payload = encoded_payload(&media_header);

            assert_eq!(
                MediaHeaderBox::decode_payload(&payload).unwrap(),
                media_header
            );
        }
    }

    #[test]
    fn each_time_and_the_duration_are_set_over_what_the_box_was_created_with() {
        let created = Mp4EpochSeconds::from_seconds(10);
        let modified = Mp4EpochSeconds::from_seconds(20);

        assert_eq!(
            media_header(5_000)
                .with_creation_time(created)
                .with_modification_time(modified)
                .with_duration(HeaderDuration::INDETERMINATE),
            MediaHeaderBox::new(
                created,
                modified,
                90_000,
                HeaderDuration::INDETERMINATE,
                LanguageCode::UND
            )
        );
    }

    #[test]
    fn the_pad_bit_above_the_language_is_dropped_on_the_way_in() {
        let media_header = media_header(0);
        let mut payload = encoded_payload(&media_header);
        let language = payload.get_mut(20..22).unwrap();
        language.copy_from_slice(&(LanguageCode::UND.raw() | 0x8000).to_be_bytes());

        assert_eq!(
            MediaHeaderBox::decode_payload(&payload).unwrap(),
            media_header
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let mut payload = vec![0; 24];
        *payload.first_mut().unwrap() = 2;

        assert_eq!(
            MediaHeaderBox::decode_payload(&payload),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn a_payload_longer_than_its_version_requires_is_rejected() {
        assert_eq!(
            MediaHeaderBox::decode_payload(&[0; 25]),
            Err(Error::trailing_payload(24, 25))
        );
    }
}
