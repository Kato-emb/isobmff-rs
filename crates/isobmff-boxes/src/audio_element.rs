//! [`AudioElementBox`] and the boxes that describe an audio element of a preselection and how it may be changed

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, FullBoxFlags, FullBoxHeader, OtherBoxes, Utf8String,
};

use crate::elng::ExtendedLanguageBox;
use crate::full_box::read_version_zero;
use crate::infe::{read_string, write_string};

/// Bit of `flags` that says the azimuth fields are there
const AZIMUTH_PRESENT: u32 = 1;
/// Bit of `flags` that says the elevation fields are there
const ELEVATION_PRESENT: u32 = 2;
/// Bit of `flags` that says the distance fields are there
const DISTANCE_PRESENT: u32 = 4;

/// [`AudioElementPosInteractivityPolar`] (`aepp`), ISO/IEC 14496-12 §12.2.11
///
/// Whether, and within which limits, an audio element may be placed
/// interactively in the audio scene, in polar coordinates. A coordinate whose
/// fields are [`None`] may not be changed.
///
/// The `flags` of the box are whether the fields of each coordinate are
/// there. The three fields of a coordinate are there together: a coordinate
/// of some fields but not all is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue), for the first
/// field of the coordinate, when the box is written.
#[doc(alias = "aepp")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct AudioElementPosInteractivityPolar {
    /// Least azimuth, in degrees as ISO/IEC 23091-3 defines `LoudspeakerAzimuth`
    pub min_azimuth: Option<i16>,
    /// Greatest azimuth, in degrees
    pub max_azimuth: Option<i16>,
    /// Azimuth by default, in degrees
    pub default_azimuth: Option<i16>,
    /// Least elevation, in degrees as ISO/IEC 23091-3 defines `LoudspeakerElevation`
    pub min_elevation: Option<i8>,
    /// Greatest elevation, in degrees
    pub max_elevation: Option<i8>,
    /// Elevation by default, in degrees
    pub default_elevation: Option<i8>,
    /// Least distance from the listener, in millimetres
    pub min_distance: Option<u32>,
    /// Greatest distance from the listener, in millimetres
    pub max_distance: Option<u32>,
    /// Distance from the listener by default, in millimetres
    pub default_distance: Option<u32>,
}

/// Returns the three fields of a coordinate, [`None`] where none is there
///
/// # Errors
///
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) naming `field`:
///   some of the fields are there but not all.
fn coordinate<Value: Copy>(
    fields: [Option<Value>; 3],
    field: &'static str,
) -> Result<Option<[Value; 3]>, Error> {
    match fields {
        [Some(min), Some(max), Some(default)] => Ok(Some([min, max, default])),
        [None, None, None] => Ok(None),
        _ => Err(Error::invalid_value(field)),
    }
}

impl BoxDefinition for AudioElementPosInteractivityPolar {
    const BOX_TYPE: BoxType = BoxType::compact(*b"aepp");
}

impl BoxDecode for AudioElementPosInteractivityPolar {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let flags = read_version_zero(reader)?.bits();
        let mut read = Self::default();
        if flags & AZIMUTH_PRESENT != 0 {
            read.min_azimuth = Some(reader.read_i16()?);
            read.max_azimuth = Some(reader.read_i16()?);
            read.default_azimuth = Some(reader.read_i16()?);
        }
        if flags & ELEVATION_PRESENT != 0 {
            let [min, max, default] =
                (*reader.read_bytes::<3>()?).map(|byte| i8::from_be_bytes([byte]));
            read.min_elevation = Some(min);
            read.max_elevation = Some(max);
            read.default_elevation = Some(default);
        }
        if flags & DISTANCE_PRESENT != 0 {
            read.min_distance = Some(reader.read_u32()?);
            read.max_distance = Some(reader.read_u32()?);
            read.default_distance = Some(reader.read_u32()?);
        }

        Ok(read)
    }
}

impl BoxEncode for AudioElementPosInteractivityPolar {
    fn payload_len(&self) -> u64 {
        [
            (self.min_azimuth.is_some(), 2 * 3),
            (self.min_elevation.is_some(), 3),
            (self.min_distance.is_some(), 4 * 3),
        ]
        .into_iter()
        .filter(|(present, _)| *present)
        .fold(4, |len, (_, fields_len)| len.saturating_add(fields_len))
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let azimuth = coordinate(
            [self.min_azimuth, self.max_azimuth, self.default_azimuth],
            "minAzimuth",
        )?;
        let elevation = coordinate(
            [
                self.min_elevation,
                self.max_elevation,
                self.default_elevation,
            ],
            "minElevation",
        )?;
        let distance = coordinate(
            [self.min_distance, self.max_distance, self.default_distance],
            "minDistance",
        )?;
        let flags = [
            (azimuth.is_some(), AZIMUTH_PRESENT),
            (elevation.is_some(), ELEVATION_PRESENT),
            (distance.is_some(), DISTANCE_PRESENT),
        ]
        .into_iter()
        .filter(|(present, _)| *present)
        .fold(0, |flags, (_, bit)| flags | bit);

        writer.write_bytes(
            &FullBoxHeader::new(0, FullBoxFlags::new(flags).unwrap_or(FullBoxFlags::ZERO))
                .to_bytes(),
        )?;
        if let Some(azimuth) = azimuth {
            azimuth
                .iter()
                .try_for_each(|value| writer.write_i16(*value))?;
        }
        if let Some(elevation) = elevation {
            writer.write_bytes(&elevation.map(|value| {
                let [byte] = value.to_be_bytes();
                byte
            }))?;
        }
        if let Some(distance) = distance {
            distance
                .iter()
                .try_for_each(|value| writer.write_u32(*value))?;
        }

        Ok(())
    }
}

/// [`AudioElementProminenceInteractivity`] (`aepr`), ISO/IEC 14496-12 §12.2.12
///
/// The limits within which the prominence of an audio element may be changed
/// interactively. Without the box, it may not be.
#[doc(alias = "aepr")]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AudioElementProminenceInteractivity {
    /// Flags of the box, kept as read
    pub flags: FullBoxFlags,
    /// Least prominence, in dB
    pub min_prominence: f32,
    /// Greatest prominence, in dB
    pub max_prominence: f32,
    /// Prominence by default, in dB
    pub default_prominence: f32,
}

impl Default for AudioElementProminenceInteractivity {
    fn default() -> Self {
        Self {
            flags: FullBoxFlags::ZERO,
            min_prominence: 0.0,
            max_prominence: 0.0,
            default_prominence: 0.0,
        }
    }
}

impl BoxDefinition for AudioElementProminenceInteractivity {
    const BOX_TYPE: BoxType = BoxType::compact(*b"aepr");
}

impl BoxDecode for AudioElementProminenceInteractivity {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            flags: read_version_zero(reader)?,
            min_prominence: f32::from_bits(reader.read_u32()?),
            max_prominence: f32::from_bits(reader.read_u32()?),
            default_prominence: f32::from_bits(reader.read_u32()?),
        })
    }
}

impl BoxEncode for AudioElementProminenceInteractivity {
    fn payload_len(&self) -> u64 {
        4 + 4 * 3
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(0, self.flags).to_bytes())?;
        [
            self.min_prominence,
            self.max_prominence,
            self.default_prominence,
        ]
        .iter()
        .try_for_each(|value| writer.write_u32(value.to_bits()))
    }
}

/// [`AudioElementDescriptionBox`] (`aedb`), ISO/IEC 14496-12 §12.2.10
///
/// What an audio element is, and how it may be changed interactively: by
/// switching it, by placing it, and by its prominence.
///
/// The box writes `aepp` and then `aepr` after its fields, whatever order they
/// were read in. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them, and so is
/// one that does not read, which then reads back [`None`]. The six bits of
/// `reserved` are read past and written as zero.
#[doc(alias = "aedb")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct AudioElementDescriptionBox {
    /// Flags of the box, kept as read
    pub flags: FullBoxFlags,
    /// Identifier of the audio element, unique within the preselection
    pub audio_element_tag: Utf8String,
    /// Whether the audio element can be switched on and off on its own
    pub is_toggleable: bool,
    /// Whether the audio element is switched on by default
    pub is_default_enabled: bool,
    /// Limits within which the audio element may be placed, where it may be
    pub aepp: Option<AudioElementPosInteractivityPolar>,
    /// Limits within which the prominence of the audio element may be changed, where it may be
    pub aepr: Option<AudioElementProminenceInteractivity>,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl Default for AudioElementDescriptionBox {
    fn default() -> Self {
        Self {
            flags: FullBoxFlags::ZERO,
            audio_element_tag: Utf8String::default(),
            is_toggleable: false,
            is_default_enabled: false,
            aepp: None,
            aepr: None,
            other_boxes: OtherBoxes::default(),
        }
    }
}

impl BoxDefinition for AudioElementDescriptionBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"aedb");
}

impl BoxDecode for AudioElementDescriptionBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let flags = read_version_zero(reader)?;
        let audio_element_tag = read_string(reader)?;
        let [switches] = *reader.read_bytes::<1>()?;
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            flags,
            audio_element_tag,
            is_toggleable: switches & 0x80 != 0,
            is_default_enabled: switches & 0x40 != 0,
            aepp: children.take_zero_or_one_if_decoded()?,
            aepr: children.take_zero_or_one_if_decoded()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for AudioElementDescriptionBox {
    fn payload_len(&self) -> u64 {
        [
            4 + 1,
            self.audio_element_tag.encoded_len(),
            self.aepp.as_ref().map_or(0, BoxEncode::encoded_len),
            self.aepr.as_ref().map_or(0, BoxEncode::encoded_len),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxHeader::new(0, self.flags).to_bytes())?;
        write_string(writer, &self.audio_element_tag)?;
        writer.write_bytes(&[
            u8::from(self.is_toggleable) << 7 | u8::from(self.is_default_enabled) << 6
        ])?;
        let mut rest = writer.take_remainder();
        if let Some(aepp) = &self.aepp {
            rest = aepp.encode(rest)?;
        }
        if let Some(aepr) = &self.aepr {
            rest = aepr.encode(rest)?;
        }
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

/// [`AudioElementBox`] (`aelm`), ISO/IEC 14496-12 §12.2.9
///
/// The semantic information about one audio element of a preselection or of
/// an audio element selection.
///
/// The box writes `elng` and then `aedb`, whatever order they were read in:
/// the order of ISO/IEC 14496-12 Table 2. Children of other types are kept in
/// [`other_boxes`](Self::other_boxes), and written back after them, and so is
/// an `elng` that does not read, which then reads back [`None`]. A box of no
/// `aedb` does not read.
#[doc(alias = "aelm")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Debug)]
pub struct AudioElementBox {
    /// Language of the audio element
    pub elng: Option<ExtendedLanguageBox>,
    /// What the audio element is and how it may be changed
    pub aedb: AudioElementDescriptionBox,
    /// Children of a type the box holds no field for
    pub other_boxes: OtherBoxes,
}

impl BoxDefinition for AudioElementBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"aelm");
}

impl BoxDecode for AudioElementBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut children = ChildBoxes::decode(reader.take_remainder())?;

        Ok(Self {
            elng: children.take_zero_or_one_if_decoded()?,
            aedb: children.take_exactly_one()?,
            other_boxes: OtherBoxes::from(children),
        })
    }
}

impl BoxEncode for AudioElementBox {
    fn payload_len(&self) -> u64 {
        [
            self.elng.as_ref().map_or(0, BoxEncode::encoded_len),
            self.aedb.encoded_len(),
        ]
        .into_iter()
        .chain(self.other_boxes.as_slice().iter().map(AnyBox::encoded_len))
        .fold(0, u64::saturating_add)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = writer.take_remainder();
        if let Some(elng) = &self.elng {
            rest = elng.encode(rest)?;
        }
        rest = self.aedb.encode(rest)?;
        for kept in self.other_boxes.as_slice() {
            rest = kept.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use isobmff_core::{
        BoxDecode, BoxDefinition, BoxEncode, ChildBoxes, Error, FullBoxFlags, OtherBoxes,
        Utf8String,
    };

    use super::{
        AudioElementBox, AudioElementDescriptionBox, AudioElementPosInteractivityPolar,
        AudioElementProminenceInteractivity,
    };
    use crate::elng::ExtendedLanguageBox;
    use crate::encoded;

    /// `aepp` of every coordinate: azimuth -30 to 30 by 0, elevation -10 to 10 by 0, distance 500 to 2000 by 1000
    const AEPP: &[u8] = b"\0\0\0\x21aepp\0\0\0\x07\xff\xe2\0\x1e\0\0\xf6\x0a\0\
        \0\0\x01\xf4\0\0\x07\xd0\0\0\x03\xe8";

    /// `aepp` of the elevation alone: -10 to 10 by 0
    const AEPP_ELEVATION: &[u8] = b"\0\0\0\x0faepp\0\0\0\x02\xf6\x0a\0";

    /// `aepr` of -6 dB to 6 dB by 0 dB
    const AEPR: &[u8] = b"\0\0\0\x18aepr\0\0\0\0\xc0\xc0\0\0\x40\xc0\0\0\0\0\0\0";

    /// `elng` of `de`
    const ELNG: &[u8] = b"\0\0\0\x0felng\0\0\0\0de\0";

    /// `kind` holding nothing, a box `aelm` holds no field for
    const KIND: &[u8] = b"\0\0\0\x08kind";

    fn string(text: &str) -> Utf8String {
        Utf8String::new(String::from(text)).unwrap()
    }

    fn positioning() -> AudioElementPosInteractivityPolar {
        AudioElementPosInteractivityPolar {
            min_azimuth: Some(-30),
            max_azimuth: Some(30),
            default_azimuth: Some(0),
            min_elevation: Some(-10),
            max_elevation: Some(10),
            default_elevation: Some(0),
            min_distance: Some(500),
            max_distance: Some(2000),
            default_distance: Some(1000),
        }
    }

    fn elevation_alone() -> AudioElementPosInteractivityPolar {
        AudioElementPosInteractivityPolar {
            min_elevation: Some(-10),
            max_elevation: Some(10),
            default_elevation: Some(0),
            ..AudioElementPosInteractivityPolar::default()
        }
    }

    fn prominence() -> AudioElementProminenceInteractivity {
        AudioElementProminenceInteractivity {
            flags: FullBoxFlags::ZERO,
            min_prominence: -6.0,
            max_prominence: 6.0,
            default_prominence: 0.0,
        }
    }

    fn description() -> AudioElementDescriptionBox {
        AudioElementDescriptionBox {
            flags: FullBoxFlags::ZERO,
            audio_element_tag: string("dialog"),
            is_toggleable: true,
            is_default_enabled: false,
            aepp: Some(positioning()),
            aepr: Some(prominence()),
            other_boxes: OtherBoxes::default(),
        }
    }

    /// Fields of [`description`]: tag `dialog`, toggleable, not enabled by default
    const AEDB_FIELDS: &[u8] = b"\0\0\0\0dialog\0\x80";

    #[test]
    fn the_interactivity_boxes_read_from_the_bytes_their_syntax_lays_out() {
        assert_eq!(
            (
                AudioElementPosInteractivityPolar::decode(AEPP).map(|(read, _)| read),
                AudioElementPosInteractivityPolar::decode(AEPP_ELEVATION).map(|(read, _)| read),
                AudioElementProminenceInteractivity::decode(AEPR).map(|(read, _)| read)
            ),
            (Ok(positioning()), Ok(elevation_alone()), Ok(prominence()))
        );
    }

    #[test]
    fn the_interactivity_boxes_write_the_bytes_their_syntax_lays_out() {
        assert_eq!(
            (
                encoded(&positioning()),
                encoded(&elevation_alone()),
                encoded(&prominence())
            ),
            (AEPP.to_vec(), AEPP_ELEVATION.to_vec(), AEPR.to_vec())
        );
    }

    #[test]
    fn a_coordinate_of_some_fields_but_not_all_is_refused_when_written() {
        let mut partial = positioning();
        partial.max_distance = None;

        assert_eq!(
            partial.encode(&mut [0; 64]),
            Err(Error::invalid_value("minDistance"))
        );
    }

    #[test]
    fn an_audio_element_reads_its_description_and_writes_it_after_elng() {
        let aedb = [b"\0\0\0\x4daedb".as_slice(), AEDB_FIELDS, AEPR, AEPP].concat();
        let bytes = [b"\0\0\0\x6caelm".as_slice(), KIND, &aedb, ELNG].concat();
        let expected = AudioElementBox {
            elng: Some(ExtendedLanguageBox {
                extended_language: string("de"),
            }),
            aedb: description(),
            other_boxes: OtherBoxes::from(ChildBoxes::decode(KIND).unwrap()),
        };
        let written = [
            b"\0\0\0\x6caelm".as_slice(),
            ELNG,
            b"\0\0\0\x4daedb",
            AEDB_FIELDS,
            AEPP,
            AEPR,
            KIND,
        ]
        .concat();

        assert_eq!(
            AudioElementBox::decode(&bytes).map(|(read, _)| (encoded(&read), read)),
            Ok((written, expected))
        );
    }

    #[test]
    fn an_audio_element_of_no_description_is_refused() {
        assert_eq!(
            AudioElementBox::decode_payload(ELNG),
            Err(Error::missing_mandatory_box(
                AudioElementDescriptionBox::BOX_TYPE
            ))
        );
    }

    #[test]
    fn a_payload_ending_inside_a_field_is_refused() {
        assert_eq!(
            (
                AudioElementPosInteractivityPolar::decode_payload(b"\0\0\0\x01\0\0"),
                AudioElementProminenceInteractivity::decode_payload(b"\0\0\0\0\0\0"),
                AudioElementDescriptionBox::decode_payload(b"\0\0\0\0a\0")
            ),
            (
                Err(Error::truncated_payload(8, 6)),
                Err(Error::truncated_payload(8, 6)),
                Err(Error::truncated_payload(7, 6))
            )
        );
    }

    #[test]
    fn a_version_the_boxes_do_not_read_is_refused() {
        assert_eq!(
            (
                AudioElementPosInteractivityPolar::decode_payload(b"\x01\0\0\0"),
                AudioElementProminenceInteractivity::decode_payload(b"\x01\0\0\0"),
                AudioElementDescriptionBox::decode_payload(b"\x01\0\0\0")
            ),
            (
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1)),
                Err(Error::unsupported_version(1))
            )
        );
    }
}
