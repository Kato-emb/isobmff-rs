//! [`VisualSampleEntry`] (ISO/IEC 14496-12 §12.1.3) and [`AudioSampleEntry`]
//! (§12.2.3), the sample entry classes a coding derives its own from
//!
//! Neither is a box of its own: §8.5.2.2 declares `SampleEntry(format)` as an
//! abstract class, and a derived specification names the concrete class by the
//! coding it stands for — `avc1`, `mp4a`. The types here are the fields those
//! classes open with, read and written by the derived entry that composes them.
//! The boxes a sample entry may hold after its fields — `clap`, `pasp`, `srat`,
//! `btrt`, the ones a coding adds — are the derived entry's to sort.

use isobmff_core::{CompressorName, Error, FieldReader, FieldWriter, U16F16};

/// Fields a visual sample entry opens with
///
/// [`VisualSampleEntry`], ISO/IEC 14496-12 §12.1.3. The `pre_defined` and
/// `reserved` fields are not held: they are written as the spec fixes them,
/// and read past.
///
/// [`new`](Self::new) fills the template fields with the values the spec
/// gives them — a resolution of 72 dpi, one frame per sample, a depth of
/// `0x0018` for colour with no alpha — and leaves `compressorname` empty; a
/// decoded entry holds whatever the file stated.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct VisualSampleEntry {
    data_reference_index: u16,
    width: u16,
    height: u16,
    horiz_resolution: U16F16,
    vert_resolution: U16F16,
    frame_count: u16,
    compressor_name: CompressorName,
    depth: u16,
}

impl VisualSampleEntry {
    /// Length the fields occupy, the `SampleEntry` fields included
    pub const LEN: u64 = 78;

    /// Resolution the spec gives as the template value, 72 dpi
    const TEMPLATE_RESOLUTION: U16F16 = U16F16::from_raw(0x0048_0000);

    /// Depth the spec gives as the template value, colour with no alpha
    const TEMPLATE_DEPTH: u16 = 0x0018;

    /// Creates the fields for a picture of `width` by `height` pixels, with
    /// every template field at the value the spec gives it
    #[must_use]
    pub const fn new(data_reference_index: u16, width: u16, height: u16) -> Self {
        Self {
            data_reference_index,
            width,
            height,
            horiz_resolution: Self::TEMPLATE_RESOLUTION,
            vert_resolution: Self::TEMPLATE_RESOLUTION,
            frame_count: 1,
            compressor_name: CompressorName::EMPTY,
            depth: Self::TEMPLATE_DEPTH,
        }
    }

    /// Returns the index of the data reference the samples are read through
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// Returns the width in pixels the coding delivers
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Returns the height in pixels the coding delivers
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// Returns the horizontal resolution in pixels per inch
    #[must_use]
    pub const fn horiz_resolution(&self) -> U16F16 {
        self.horiz_resolution
    }

    /// Returns the vertical resolution in pixels per inch
    #[must_use]
    pub const fn vert_resolution(&self) -> U16F16 {
        self.vert_resolution
    }

    /// Returns how many frames of compressed video each sample holds
    #[must_use]
    pub const fn frame_count(&self) -> u16 {
        self.frame_count
    }

    /// Returns the name of the compressor, for information
    #[must_use]
    pub const fn compressor_name(&self) -> &CompressorName {
        &self.compressor_name
    }

    /// Returns the depth, `0x0018` for colour with no alpha
    #[must_use]
    pub const fn depth(&self) -> u16 {
        self.depth
    }

    /// Reads the fields off the front of `reader`, leaving the boxes that follow
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends before the fields do.
    pub fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let _reserved = reader.read_bytes::<6>()?;
        let data_reference_index = reader.read_u16()?;
        let _pre_defined = reader.read_bytes::<16>()?;
        let width = reader.read_u16()?;
        let height = reader.read_u16()?;
        let horiz_resolution = U16F16::from_raw(reader.read_u32()?);
        let vert_resolution = U16F16::from_raw(reader.read_u32()?);
        let _reserved = reader.read_u32()?;
        let frame_count = reader.read_u16()?;
        let compressor_name = CompressorName::from_bytes(*reader.read_bytes::<32>()?);
        let depth = reader.read_u16()?;
        let _pre_defined = reader.read_i16()?;

        Ok(Self {
            data_reference_index,
            width,
            height,
            horiz_resolution,
            vert_resolution,
            frame_count,
            compressor_name,
            depth,
        })
    }

    /// Writes the fields into the front of `writer`, leaving room for the boxes
    /// that follow
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): `writer`
    ///   has less than [`LEN`](Self::LEN) bytes left.
    pub fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[0; 6])?;
        writer.write_u16(self.data_reference_index)?;
        writer.write_bytes(&[0; 16])?;
        writer.write_u16(self.width)?;
        writer.write_u16(self.height)?;
        writer.write_u32(self.horiz_resolution.raw())?;
        writer.write_u32(self.vert_resolution.raw())?;
        writer.write_u32(0)?;
        writer.write_u16(self.frame_count)?;
        writer.write_bytes(self.compressor_name.as_bytes())?;
        writer.write_u16(self.depth)?;
        writer.write_i16(-1)
    }
}

/// Fields an audio sample entry opens with
///
/// [`AudioSampleEntry`] and [`AudioSampleEntryV1`], ISO/IEC 14496-12 §12.2.3.
/// The two classes lay the same 28 bytes out, differing in the first field:
/// version 0 opens with reserved bytes, version 1 with an `entry_version` of 1.
/// One value stands for either and reports which through
/// [`entry_version`](Self::entry_version). A version 1 entry must lie in a
/// `stsd` of version 1.
///
/// The `pre_defined` and `reserved` fields are not held: they are written as
/// the spec fixes them, and read past. A layout of another version — the
/// QuickTime file format writes versions 1 and 2 of its own with more fields —
/// is not read.
///
/// [`AudioSampleEntryV1`]: Self
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct AudioSampleEntry {
    entry_version: u16,
    data_reference_index: u16,
    channel_count: u16,
    sample_size: u16,
    sample_rate: U16F16,
}

impl AudioSampleEntry {
    /// Length the fields occupy, the `SampleEntry` fields included
    pub const LEN: u64 = 28;

    /// Sample size in bits the spec gives as the template value
    const TEMPLATE_SAMPLE_SIZE: u16 = 16;

    /// Creates the version 0 fields for audio of `channel_count` channels
    /// sampled `sample_rate` times a second, at the template sample size of 16
    /// bits
    ///
    /// The rate is stated whole, as the integer part of the 16.16 field it is
    /// written to. A rate past `u16` cannot be stated this way; version 1
    /// carries it in a `SamplingRateBox` instead.
    #[must_use]
    pub const fn new(data_reference_index: u16, channel_count: u16, sample_rate: u16) -> Self {
        Self {
            entry_version: 0,
            data_reference_index,
            channel_count,
            sample_size: Self::TEMPLATE_SAMPLE_SIZE,
            sample_rate: U16F16::from_raw((sample_rate as u32) << 16),
        }
    }

    /// Creates the version 1 fields for audio of `channel_count` channels, at
    /// the template sample size of 16 bits and the template `samplerate` of 1
    ///
    /// The sampling rate is stated by the `SamplingRateBox` that follows the
    /// fields, which the derived entry holds.
    #[must_use]
    pub const fn new_v1(data_reference_index: u16, channel_count: u16) -> Self {
        Self {
            entry_version: 1,
            data_reference_index,
            channel_count,
            sample_size: Self::TEMPLATE_SAMPLE_SIZE,
            sample_rate: U16F16::ONE,
        }
    }

    /// Returns the version of the entry, 0 or 1
    #[must_use]
    pub const fn entry_version(&self) -> u16 {
        self.entry_version
    }

    /// Returns the index of the data reference the samples are read through
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// Returns the number of channels, 1 for mono or 2 for stereo
    #[must_use]
    pub const fn channel_count(&self) -> u16 {
        self.channel_count
    }

    /// Returns the sample size in bits
    #[must_use]
    pub const fn sample_size(&self) -> u16 {
        self.sample_size
    }

    /// Returns the `samplerate` field, the sampling rate as a 16.16 number when
    /// no `SamplingRateBox` states it
    #[must_use]
    pub const fn sample_rate(&self) -> U16F16 {
        self.sample_rate
    }

    /// Reads the fields off the front of `reader`, leaving the boxes that follow
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends before the fields do.
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
    ///   entry opens with a version other than 0 or 1.
    pub fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let _reserved = reader.read_bytes::<6>()?;
        let data_reference_index = reader.read_u16()?;
        let entry_version = reader.read_u16()?;
        if entry_version > 1 {
            // Why not carry the field whole: the failure names a `version` as a
            // full box states it, one byte wide, and a version this field
            // states past that byte is unsupported all the same.
            return Err(Error::unsupported_version(
                u8::try_from(entry_version).unwrap_or(u8::MAX),
            ));
        }
        let _reserved = reader.read_bytes::<6>()?;
        let channel_count = reader.read_u16()?;
        let sample_size = reader.read_u16()?;
        let _pre_defined = reader.read_u16()?;
        let _reserved = reader.read_u16()?;
        let sample_rate = U16F16::from_raw(reader.read_u32()?);

        Ok(Self {
            entry_version,
            data_reference_index,
            channel_count,
            sample_size,
            sample_rate,
        })
    }

    /// Writes the fields into the front of `writer`, leaving room for the boxes
    /// that follow
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): `writer`
    ///   has less than [`LEN`](Self::LEN) bytes left.
    pub fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&[0; 6])?;
        writer.write_u16(self.data_reference_index)?;
        writer.write_u16(self.entry_version)?;
        writer.write_bytes(&[0; 6])?;
        writer.write_u16(self.channel_count)?;
        writer.write_u16(self.sample_size)?;
        writer.write_u16(0)?;
        writer.write_u16(0)?;
        writer.write_u32(self.sample_rate.raw())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{Error, FieldReader, FieldWriter, U16F16};

    use super::{AudioSampleEntry, VisualSampleEntry};

    fn encoded_visual(entry: &VisualSampleEntry) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(VisualSampleEntry::LEN).unwrap()];
        let mut writer = FieldWriter::new(&mut buffer);
        entry.encode_fields(&mut writer).unwrap();
        writer.finish().unwrap();

        buffer
    }

    fn encoded_audio(entry: &AudioSampleEntry) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(AudioSampleEntry::LEN).unwrap()];
        let mut writer = FieldWriter::new(&mut buffer);
        entry.encode_fields(&mut writer).unwrap();
        writer.finish().unwrap();

        buffer
    }

    #[test]
    fn visual_fields_read_back_as_the_value_that_wrote_them() {
        let entry = VisualSampleEntry::new(1, 1920, 1080);

        let bytes = encoded_visual(&entry);

        assert_eq!(
            VisualSampleEntry::decode_fields(&mut FieldReader::new(&bytes)).unwrap(),
            entry
        );
    }

    #[test]
    fn visual_fields_are_laid_out_as_the_spec_states_them() {
        let bytes = encoded_visual(&VisualSampleEntry::new(1, 1920, 1080));

        assert_eq!(
            bytes,
            [
                b"\0\0\0\0\0\0\0\x01".as_slice(),
                &[0; 16],
                b"\x07\x80\x04\x38\0\x48\0\0\0\x48\0\0\0\0\0\0\0\x01",
                &[0; 32],
                b"\0\x18\xff\xff",
            ]
            .concat()
        );
    }

    #[test]
    fn visual_fields_leave_the_boxes_that_follow_unread() {
        let bytes = [
            encoded_visual(&VisualSampleEntry::new(1, 16, 16)),
            vec![0xab; 4],
        ]
        .concat();
        let mut reader = FieldReader::new(&bytes);

        VisualSampleEntry::decode_fields(&mut reader).unwrap();

        assert_eq!(reader.remainder(), [0xab; 4]);
    }

    #[test]
    fn visual_fields_cut_short_are_rejected_as_truncated() {
        assert_eq!(
            VisualSampleEntry::decode_fields(&mut FieldReader::new(&[0; 77])),
            Err(Error::truncated_payload(78, 77))
        );
    }

    #[test]
    fn audio_fields_read_back_as_the_value_that_wrote_them_at_either_version() {
        for entry in [
            AudioSampleEntry::new(1, 2, 48_000),
            AudioSampleEntry::new_v1(1, 6),
        ] {
            let bytes = encoded_audio(&entry);

            assert_eq!(
                AudioSampleEntry::decode_fields(&mut FieldReader::new(&bytes)).unwrap(),
                entry
            );
        }
    }

    #[test]
    fn version_0_audio_fields_carry_the_rate_above_the_point() {
        let bytes = encoded_audio(&AudioSampleEntry::new(1, 2, 48_000));

        assert_eq!(
            bytes,
            b"\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\0\0\x02\0\x10\0\0\0\0\xbb\x80\0\0"
        );
    }

    #[test]
    fn version_1_audio_fields_state_their_version_and_a_rate_of_one() {
        let entry = AudioSampleEntry::new_v1(1, 6);

        assert_eq!(entry.sample_rate(), U16F16::ONE);
        assert_eq!(
            encoded_audio(&entry),
            b"\0\0\0\0\0\0\0\x01\0\x01\0\0\0\0\0\0\0\x06\0\x10\0\0\0\0\0\x01\0\0"
        );
    }

    #[test]
    fn an_audio_entry_version_the_fields_do_not_read_is_rejected() {
        let mut bytes = encoded_audio(&AudioSampleEntry::new(1, 2, 44_100));
        bytes
            .get_mut(8..10)
            .unwrap()
            .copy_from_slice(&2_u16.to_be_bytes());

        assert_eq!(
            AudioSampleEntry::decode_fields(&mut FieldReader::new(&bytes)),
            Err(Error::unsupported_version(2))
        );
    }

    #[test]
    fn audio_fields_cut_short_are_rejected_as_truncated() {
        assert_eq!(
            AudioSampleEntry::decode_fields(&mut FieldReader::new(&[0; 27])),
            Err(Error::truncated_payload(28, 27))
        );
    }
}
