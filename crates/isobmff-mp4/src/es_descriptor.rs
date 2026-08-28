//! [`ESDescriptor`] and the descriptors it holds, ISO/IEC 14496-1 §7.2.6

use alloc::vec::Vec;

use isobmff_core::{FieldReader, FieldWriter};

use crate::descriptor::{DescriptorTag, RawDescriptor, decode_header, encode_header, encoded_len};
use crate::error::Error;

/// Decoder configuration this crate reads and writes as the spec lays it out
///
/// `DecoderConfigDescriptor`, ISO/IEC 14496-1 §7.2.6.6. The
/// `DecoderSpecificInfo` it may hold is kept as the bytes it lies as — for an
/// AAC stream the `AudioSpecificConfig` of ISO/IEC 14496-3 — and every other
/// descriptor after the fixed fields is kept raw.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DecoderConfigDescriptor {
    object_type_indication: u8,
    stream_type: u8,
    up_stream: bool,
    buffer_size_db: u32,
    max_bitrate: u32,
    avg_bitrate: u32,
    decoder_specific_info: Option<DecoderSpecificInfo>,
    other_descriptors: Vec<RawDescriptor>,
}

impl DecoderConfigDescriptor {
    /// `objectTypeIndication` of an MPEG-4 AAC stream, ISO/IEC 14496-1 Table 5
    pub const OBJECT_TYPE_AUDIO_ISO_14496_3: u8 = 0x40;

    /// `streamType` of an audio stream, ISO/IEC 14496-1 Table 6
    pub const STREAM_TYPE_AUDIO: u8 = 0x05;

    /// `streamType` of a visual stream, ISO/IEC 14496-1 Table 6
    pub const STREAM_TYPE_VISUAL: u8 = 0x04;

    /// Creates the descriptor for a stream stored in the file, `upStream` off
    ///
    /// Returns `None` when `stream_type` is past its six bits or
    /// `buffer_size_db` past its 24.
    #[must_use]
    pub fn new(
        object_type_indication: u8,
        stream_type: u8,
        buffer_size_db: u32,
        max_bitrate: u32,
        avg_bitrate: u32,
        decoder_specific_info: Option<DecoderSpecificInfo>,
    ) -> Option<Self> {
        if stream_type > 0x3f || buffer_size_db > 0x00ff_ffff {
            return None;
        }

        Some(Self {
            object_type_indication,
            stream_type,
            up_stream: false,
            buffer_size_db,
            max_bitrate,
            avg_bitrate,
            decoder_specific_info,
            other_descriptors: Vec::new(),
        })
    }

    /// Returns the `objectTypeIndication`, the coding of the stream
    #[must_use]
    pub const fn object_type_indication(&self) -> u8 {
        self.object_type_indication
    }

    /// Returns the `streamType`, the kind of stream
    #[must_use]
    pub const fn stream_type(&self) -> u8 {
        self.stream_type
    }

    /// Returns `upStream`, whether the stream flows to the sender
    #[must_use]
    pub const fn up_stream(&self) -> bool {
        self.up_stream
    }

    /// Returns `bufferSizeDB`, the decoding buffer size in bytes
    #[must_use]
    pub const fn buffer_size_db(&self) -> u32 {
        self.buffer_size_db
    }

    /// Returns `maxBitrate`, in bits per second over any one-second window
    #[must_use]
    pub const fn max_bitrate(&self) -> u32 {
        self.max_bitrate
    }

    /// Returns `avgBitrate`, in bits per second over the whole stream
    #[must_use]
    pub const fn avg_bitrate(&self) -> u32 {
        self.avg_bitrate
    }

    /// Returns the `DecoderSpecificInfo`, when the descriptor holds one
    #[must_use]
    pub const fn decoder_specific_info(&self) -> Option<&DecoderSpecificInfo> {
        self.decoder_specific_info.as_ref()
    }

    /// Returns the descriptors no field claims, in the order they came
    #[must_use]
    pub fn other_descriptors(&self) -> &[RawDescriptor] {
        &self.other_descriptors
    }

    fn body_len(&self) -> u64 {
        let info = self
            .decoder_specific_info
            .as_ref()
            .map_or(0, |info| info.0.encoded_len());

        13_u64
            .saturating_add(info)
            .saturating_add(raw_descriptors_len(&self.other_descriptors))
    }

    fn decode_body(body: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(body);
        let [object_type_indication, flags] = *reader.read_bytes::<2>()?;
        let [high, middle, low] = *reader.read_bytes::<3>()?;
        let buffer_size_db = u32::from_be_bytes([0, high, middle, low]);
        let max_bitrate = reader.read_u32()?;
        let avg_bitrate = reader.read_u32()?;

        let mut decoder_specific_info = None;
        let mut other_descriptors = Vec::new();
        while !reader.remainder().is_empty() {
            let (tag, body) = decode_header(&mut reader)?;
            let descriptor = RawDescriptor::decoded(tag, body);
            if tag == DescriptorTag::DECODER_SPECIFIC_INFO {
                if decoder_specific_info.is_some() {
                    return Err(Error::duplicate_descriptor(tag));
                }
                decoder_specific_info = Some(DecoderSpecificInfo(descriptor));
            } else {
                other_descriptors.push(descriptor);
            }
        }

        Ok(Self {
            object_type_indication,
            stream_type: flags >> 2,
            up_stream: flags & 0b10 != 0,
            buffer_size_db,
            max_bitrate,
            avg_bitrate,
            decoder_specific_info,
            other_descriptors,
        })
    }

    fn encode(&self, writer: &mut FieldWriter<'_>) -> Result<(), isobmff_core::Error> {
        encode_header(writer, DescriptorTag::DECODER_CONFIG, self.body_len())?;
        let up_stream = if self.up_stream { 0b10 } else { 0 };
        writer.write_bytes(&[
            self.object_type_indication,
            (self.stream_type << 2) | up_stream | 1,
        ])?;
        let [_, high, middle, low] = self.buffer_size_db.to_be_bytes();
        writer.write_bytes(&[high, middle, low])?;
        writer.write_u32(self.max_bitrate)?;
        writer.write_u32(self.avg_bitrate)?;
        if let Some(info) = &self.decoder_specific_info {
            info.0.encode(writer)?;
        }
        for descriptor in &self.other_descriptors {
            descriptor.encode(writer)?;
        }

        Ok(())
    }
}

/// Bytes a decoder of the stream is configured with, as the coding lays them out
///
/// `DecoderSpecificInfo`, ISO/IEC 14496-1 §7.2.6.7. The bytes are the
/// coding's — the `AudioSpecificConfig` of ISO/IEC 14496-3 for AAC — and are
/// not read.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DecoderSpecificInfo(RawDescriptor);

impl DecoderSpecificInfo {
    /// Creates the descriptor from the bytes the coding lays out
    ///
    /// Returns `None` when `bytes` is longer than the 28 bits an expandable
    /// size can state.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        RawDescriptor::new(DescriptorTag::DECODER_SPECIFIC_INFO, bytes).map(Self)
    }

    /// Returns the bytes the coding lays out
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.body()
    }
}

/// Configuration of the sync layer, read as far as its `predefined` field
///
/// `SLConfigDescriptor`, ISO/IEC 14496-1 §7.3.2.3. A stream stored in an MP4
/// file uses `predefined` 2 (ISO/IEC 14496-14 §4.1.2), and the descriptor
/// then holds nothing more. One that states a `predefined` of 0 lays its
/// configuration out field by field; those bytes are kept as they lie.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SLConfigDescriptor {
    predefined: u8,
    remainder: Vec<u8>,
}

impl SLConfigDescriptor {
    /// The descriptor of a stream stored in the file, `predefined` 2
    pub const MP4: Self = Self {
        predefined: 2,
        remainder: Vec::new(),
    };

    /// Returns the `predefined` value
    #[must_use]
    pub const fn predefined(&self) -> u8 {
        self.predefined
    }

    /// Returns the bytes after `predefined`, the fields a `predefined` of 0 lays out
    #[must_use]
    pub fn remainder(&self) -> &[u8] {
        &self.remainder
    }

    fn body_len(&self) -> u64 {
        1_u64.saturating_add(self.remainder.len() as u64)
    }

    fn decode_body(body: &[u8]) -> Result<Self, Error> {
        let mut reader = FieldReader::new(body);
        let predefined = reader.read_bytes::<1>()?[0];

        Ok(Self {
            predefined,
            remainder: reader.take_remainder().to_vec(),
        })
    }

    fn encode(&self, writer: &mut FieldWriter<'_>) -> Result<(), isobmff_core::Error> {
        encode_header(writer, DescriptorTag::SL_CONFIG, self.body_len())?;
        writer.write_bytes(&[self.predefined])?;
        writer.write_slice(&self.remainder)
    }
}

/// Descriptor of one elementary stream, the whole of an `esds` payload
///
/// `ES_Descriptor`, ISO/IEC 14496-1 §7.2.6.5. The fields ISO/IEC 14496-14
/// §4.1.2 fixes for a stream stored in the file — `ES_ID` 0, no dependence,
/// no URL, no OCR stream — are held as read, so a file that states others
/// reads and writes back as it came; [`for_mp4_file`](Self::for_mp4_file)
/// writes the fixed values. Descriptors after the two every stream holds are
/// kept raw.
///
/// # Examples
///
/// ```
/// use isobmff_mp4::{DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor};
///
/// // An AAC-LC stereo stream at 48 kHz: the AudioSpecificConfig its encoder emitted
/// let audio_specific_config = DecoderSpecificInfo::new(vec![0x11, 0x90]).unwrap();
/// let decoder_config = DecoderConfigDescriptor::new(
///     DecoderConfigDescriptor::OBJECT_TYPE_AUDIO_ISO_14496_3,
///     DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
///     6144,
///     128_000,
///     128_000,
///     Some(audio_specific_config),
/// )
/// .unwrap();
///
/// let descriptor = ESDescriptor::for_mp4_file(decoder_config);
/// assert_eq!(descriptor.es_id(), 0);
/// assert_eq!(descriptor.sl_config().predefined(), 2);
/// ```
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ESDescriptor {
    es_id: u16,
    stream_priority: u8,
    depends_on_es_id: Option<u16>,
    url: Option<Vec<u8>>,
    ocr_es_id: Option<u16>,
    decoder_config: DecoderConfigDescriptor,
    sl_config: SLConfigDescriptor,
    other_descriptors: Vec<RawDescriptor>,
}

impl ESDescriptor {
    /// Creates the descriptor of a stream stored in the file, with every field
    /// ISO/IEC 14496-14 §4.1.2 fixes at its value
    #[must_use]
    pub const fn for_mp4_file(decoder_config: DecoderConfigDescriptor) -> Self {
        Self {
            es_id: 0,
            stream_priority: 0,
            depends_on_es_id: None,
            url: None,
            ocr_es_id: None,
            decoder_config,
            sl_config: SLConfigDescriptor::MP4,
            other_descriptors: Vec::new(),
        }
    }

    /// Returns `ES_ID`, 0 for a stream stored in the file
    #[must_use]
    pub const fn es_id(&self) -> u16 {
        self.es_id
    }

    /// Returns `streamPriority`
    #[must_use]
    pub const fn stream_priority(&self) -> u8 {
        self.stream_priority
    }

    /// Returns `dependsOn_ES_ID`, when `streamDependenceFlag` is set
    #[must_use]
    pub const fn depends_on_es_id(&self) -> Option<u16> {
        self.depends_on_es_id
    }

    /// Returns `URLstring`, when `URL_Flag` is set
    #[must_use]
    pub fn url(&self) -> Option<&[u8]> {
        self.url.as_deref()
    }

    /// Returns `OCR_ES_Id`, when `OCRstreamFlag` is set
    #[must_use]
    pub const fn ocr_es_id(&self) -> Option<u16> {
        self.ocr_es_id
    }

    /// Returns the decoder configuration
    #[must_use]
    pub const fn decoder_config(&self) -> &DecoderConfigDescriptor {
        &self.decoder_config
    }

    /// Returns the sync layer configuration
    #[must_use]
    pub const fn sl_config(&self) -> &SLConfigDescriptor {
        &self.sl_config
    }

    /// Returns the descriptors no field claims, in the order they came
    #[must_use]
    pub fn other_descriptors(&self) -> &[RawDescriptor] {
        &self.other_descriptors
    }

    fn body_len(&self) -> u64 {
        let depends_on = self.depends_on_es_id.map_or(0, |_| 2);
        let url = self
            .url
            .as_ref()
            .map_or(0, |url| (url.len() as u64).saturating_add(1));
        let ocr = self.ocr_es_id.map_or(0, |_| 2);

        3_u64
            .saturating_add(depends_on)
            .saturating_add(url)
            .saturating_add(ocr)
            .saturating_add(encoded_len(self.decoder_config.body_len()))
            .saturating_add(encoded_len(self.sl_config.body_len()))
            .saturating_add(raw_descriptors_len(&self.other_descriptors))
    }

    /// Returns the length the descriptor occupies, tag and size included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        encoded_len(self.body_len())
    }

    /// Reads the descriptor that opens `reader`
    ///
    /// # Errors
    ///
    /// * [`DescriptorTagMismatch`](crate::ErrorKind::DescriptorTagMismatch): the
    ///   descriptor is not an `ES_Descriptor`.
    /// * [`ExpandableSizeTooLong`](crate::ErrorKind::ExpandableSizeTooLong): a size
    ///   runs past four bytes.
    /// * [`MissingDescriptor`](crate::ErrorKind::MissingDescriptor): no
    ///   `DecoderConfigDescriptor` or no `SLConfigDescriptor` is held.
    /// * [`DuplicateDescriptor`](crate::ErrorKind::DuplicateDescriptor): one of
    ///   those, or a `DecoderSpecificInfo`, is held more than once.
    /// * [`Box`](crate::ErrorKind::Box): the bytes end inside a field or a
    ///   descriptor.
    pub fn decode(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (tag, body) = decode_header(reader)?;
        if tag != DescriptorTag::ES {
            return Err(Error::descriptor_tag_mismatch(DescriptorTag::ES, tag));
        }

        let mut reader = FieldReader::new(body);
        let es_id = reader.read_u16()?;
        let flags = reader.read_bytes::<1>()?[0];
        let depends_on_es_id = if flags & 0x80 != 0 {
            Some(reader.read_u16()?)
        } else {
            None
        };
        let url = if flags & 0x40 != 0 {
            let length = usize::from(reader.read_bytes::<1>()?[0]);
            Some(reader.read_slice(length)?.to_vec())
        } else {
            None
        };
        let ocr_es_id = if flags & 0x20 != 0 {
            Some(reader.read_u16()?)
        } else {
            None
        };

        let mut decoder_config = None;
        let mut sl_config = None;
        let mut other_descriptors = Vec::new();
        while !reader.remainder().is_empty() {
            let (tag, body) = decode_header(&mut reader)?;
            match tag {
                DescriptorTag::DECODER_CONFIG => {
                    if decoder_config.is_some() {
                        return Err(Error::duplicate_descriptor(tag));
                    }
                    decoder_config = Some(DecoderConfigDescriptor::decode_body(body)?);
                }
                DescriptorTag::SL_CONFIG => {
                    if sl_config.is_some() {
                        return Err(Error::duplicate_descriptor(tag));
                    }
                    sl_config = Some(SLConfigDescriptor::decode_body(body)?);
                }
                _other => other_descriptors.push(RawDescriptor::decoded(tag, body)),
            }
        }

        Ok(Self {
            es_id,
            stream_priority: flags & 0x1f,
            depends_on_es_id,
            url,
            ocr_es_id,
            decoder_config: decoder_config
                .ok_or(Error::missing_descriptor(DescriptorTag::DECODER_CONFIG))?,
            sl_config: sl_config.ok_or(Error::missing_descriptor(DescriptorTag::SL_CONFIG))?,
            other_descriptors,
        })
    }

    /// Writes the descriptor into the front of `writer`
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): `writer`
    ///   has less than [`encoded_len`](Self::encoded_len) bytes left.
    pub fn encode(&self, writer: &mut FieldWriter<'_>) -> Result<(), isobmff_core::Error> {
        encode_header(writer, DescriptorTag::ES, self.body_len())?;
        writer.write_u16(self.es_id)?;
        let mut flags = self.stream_priority & 0x1f;
        if self.depends_on_es_id.is_some() {
            flags |= 0x80;
        }
        if self.url.is_some() {
            flags |= 0x40;
        }
        if self.ocr_es_id.is_some() {
            flags |= 0x20;
        }
        writer.write_bytes(&[flags])?;
        if let Some(depends_on) = self.depends_on_es_id {
            writer.write_u16(depends_on)?;
        }
        if let Some(url) = &self.url {
            // Why not fail: the URL is read behind a one-byte length, so every
            // value that exists fits it, and a degenerate value stands in for
            // the panic the lints forbid.
            writer.write_bytes(&[u8::try_from(url.len()).unwrap_or(u8::MAX)])?;
            writer.write_slice(url)?;
        }
        if let Some(ocr) = self.ocr_es_id {
            writer.write_u16(ocr)?;
        }
        self.decoder_config.encode(writer)?;
        self.sl_config.encode(writer)?;
        for descriptor in &self.other_descriptors {
            descriptor.encode(writer)?;
        }

        Ok(())
    }
}

/// Returns the length `descriptors` occupy together
fn raw_descriptors_len(descriptors: &[RawDescriptor]) -> u64 {
    descriptors.iter().fold(0_u64, |total, descriptor| {
        total.saturating_add(descriptor.encoded_len())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{FieldReader, FieldWriter};

    use super::{DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor};
    use crate::descriptor::{DescriptorTag, RawDescriptor};
    use crate::error::Error;

    /// The descriptor of an AAC-LC stereo stream at 48 kHz
    pub(crate) fn aac_descriptor() -> ESDescriptor {
        ESDescriptor::for_mp4_file(
            DecoderConfigDescriptor::new(
                DecoderConfigDescriptor::OBJECT_TYPE_AUDIO_ISO_14496_3,
                DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                6144,
                128_000,
                128_000,
                Some(DecoderSpecificInfo::new(vec![0x11, 0x90]).unwrap()),
            )
            .unwrap(),
        )
    }

    /// The bytes of [`aac_descriptor`], as the spec lays them out
    pub(crate) fn aac_descriptor_bytes() -> Vec<u8> {
        vec![
            0x03, 0x19, 0x00, 0x00, 0x00, // ES_Descriptor: ES_ID 0, no flags
            0x04, 0x11, 0x40, 0x15, 0x00, 0x18, 0x00, // DecoderConfig: AAC, audio, 6144
            0x00, 0x01, 0xf4, 0x00, 0x00, 0x01, 0xf4, 0x00, // 128000 / 128000
            0x05, 0x02, 0x11, 0x90, // DecoderSpecificInfo
            0x06, 0x01, 0x02, // SLConfig predefined 2
        ]
    }

    fn encoded(descriptor: &ESDescriptor) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(descriptor.encoded_len()).unwrap()];
        let mut writer = FieldWriter::new(&mut buffer);
        descriptor.encode(&mut writer).unwrap();
        writer.finish().unwrap();

        buffer
    }

    #[test]
    fn a_descriptor_for_an_mp4_file_is_laid_out_as_the_spec_states_it() {
        assert_eq!(encoded(&aac_descriptor()), aac_descriptor_bytes());
    }

    #[test]
    fn a_descriptor_reads_back_as_the_value_that_wrote_it() {
        let bytes = encoded(&aac_descriptor());

        assert_eq!(
            ESDescriptor::decode(&mut FieldReader::new(&bytes)).unwrap(),
            aac_descriptor()
        );
    }

    #[test]
    fn sizes_written_in_four_bytes_read_as_the_same_descriptor() {
        let bytes = [
            0x03, 0x80, 0x80, 0x80, 0x22, 0x00, 0x00, 0x00, //
            0x04, 0x80, 0x80, 0x80, 0x14, 0x40, 0x15, 0x00, 0x18, 0x00, //
            0x00, 0x01, 0xf4, 0x00, 0x00, 0x01, 0xf4, 0x00, //
            0x05, 0x80, 0x80, 0x80, 0x02, 0x11, 0x90, //
            0x06, 0x80, 0x80, 0x80, 0x01, 0x02,
        ];

        assert_eq!(
            ESDescriptor::decode(&mut FieldReader::new(&bytes)).unwrap(),
            aac_descriptor()
        );
    }

    #[test]
    fn the_optional_fields_of_the_flags_are_read_and_written_back() {
        let bytes = [
            0x03, 0x1b, 0x00, 0x07, 0xe0, 0x00, 0x08, 0x01, b'u', 0x00, 0x09, //
            0x04, 0x0d, 0x40, 0x15, 0x00, 0x18, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, //
            0x06, 0x01, 0x02,
        ];

        let descriptor = ESDescriptor::decode(&mut FieldReader::new(&bytes)).unwrap();

        assert_eq!(descriptor.depends_on_es_id(), Some(8));
        assert_eq!(descriptor.url(), Some(b"u".as_slice()));
        assert_eq!(descriptor.ocr_es_id(), Some(9));
        assert_eq!(descriptor.stream_priority(), 0);
        assert_eq!(encoded(&descriptor), bytes);
    }

    #[test]
    fn a_descriptor_no_field_claims_is_kept_and_written_back() {
        let mut bytes = aac_descriptor_bytes();
        *bytes.get_mut(1).unwrap() += 3;
        bytes.extend_from_slice(&[0x09, 0x01, 0xaa]);

        let descriptor = ESDescriptor::decode(&mut FieldReader::new(&bytes)).unwrap();

        assert_eq!(
            descriptor
                .other_descriptors()
                .first()
                .map(RawDescriptor::tag),
            Some(DescriptorTag::new(0x09))
        );
        assert_eq!(encoded(&descriptor), bytes);
    }

    #[test]
    fn a_descriptor_of_another_class_is_rejected_where_the_es_descriptor_goes() {
        assert_eq!(
            ESDescriptor::decode(&mut FieldReader::new(&[0x04, 0x00])),
            Err(Error::descriptor_tag_mismatch(
                DescriptorTag::ES,
                DescriptorTag::DECODER_CONFIG
            ))
        );
    }

    #[test]
    fn a_descriptor_without_a_decoder_config_is_rejected() {
        assert_eq!(
            ESDescriptor::decode(&mut FieldReader::new(&[
                0x03, 0x06, 0x00, 0x00, 0x00, 0x06, 0x01, 0x02
            ])),
            Err(Error::missing_descriptor(DescriptorTag::DECODER_CONFIG))
        );
    }

    #[test]
    fn a_decoder_config_held_twice_is_rejected() {
        let mut bytes = aac_descriptor_bytes();
        let decoder_config = bytes.get(5..24).unwrap().to_vec();
        *bytes.get_mut(1).unwrap() += 19;
        bytes.extend_from_slice(&decoder_config);

        assert_eq!(
            ESDescriptor::decode(&mut FieldReader::new(&bytes)),
            Err(Error::duplicate_descriptor(DescriptorTag::DECODER_CONFIG))
        );
    }

    #[test]
    fn a_stream_type_past_six_bits_does_not_build() {
        assert_eq!(
            DecoderConfigDescriptor::new(0x40, 0x40, 0, 0, 0, None),
            None
        );
        assert_eq!(
            DecoderConfigDescriptor::new(0x40, 0x05, 1 << 24, 0, 0, None),
            None
        );
    }
}
