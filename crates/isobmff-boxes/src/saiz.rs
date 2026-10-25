//! [`SampleAuxiliaryInformationSizesBox`], the [`SampleInfoSizes`] it states, and the [`AuxInfoType`] it and its offsets box name

use alloc::vec::Vec;
use core::num::NonZeroU32;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter, FourCc,
    FullBoxFlags, FullBoxHeader,
};

use crate::table::{read_entries, table_len, write_entry_count};

/// Type and stream of sample auxiliary information, which a [`SampleAuxiliaryInformationSizesBox`] and a [`SampleAuxiliaryInformationOffsetsBox`](crate::SampleAuxiliaryInformationOffsetsBox) state when flag `0x1` is set
///
/// [`Default`] is the type of every byte zero, of stream 0.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AuxInfoType {
    /// Type that determines the format of the sample auxiliary information
    pub aux_info_type: FourCc,
    /// Stream of the sample auxiliary information of that type, whose meaning
    /// the type defines
    pub aux_info_type_parameter: u32,
}

impl Default for AuxInfoType {
    fn default() -> Self {
        Self {
            aux_info_type: FourCc::new([0; 4]),
            aux_info_type_parameter: 0,
        }
    }
}

impl AuxInfoType {
    /// Reads the type that flag `0x1` of `header` says is there
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside a field.
    pub(crate) fn read_if_flagged(
        header: FullBoxHeader,
        reader: &mut FieldReader<'_>,
    ) -> Result<Option<Self>, Error> {
        if header.flags().bits() & 0x1 == 0 {
            return Ok(None);
        }

        Ok(Some(Self {
            aux_info_type: FourCc::new(*reader.read_bytes::<4>()?),
            aux_info_type_parameter: reader.read_u32()?,
        }))
    }

    /// Returns how many bytes the version and flags of a box that states `aux_info_type`, and the type if there is one, take
    pub(crate) fn header_len(aux_info_type: Option<Self>) -> u64 {
        if aux_info_type.is_some() { 4 + 8 } else { 4 }
    }

    /// Writes the version and flags of a box that states `aux_info_type`, and then the type if there is one
    ///
    /// # Errors
    ///
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside a field.
    pub(crate) fn write_with_header(
        aux_info_type: Option<Self>,
        version: u8,
        writer: &mut FieldWriter<'_>,
    ) -> Result<(), Error> {
        let Some(aux_info_type) = aux_info_type else {
            return writer.write_bytes(&FullBoxHeader::new(version, FullBoxFlags::ZERO).to_bytes());
        };

        let flags = FullBoxFlags::new(0x1).unwrap_or(FullBoxFlags::ZERO);
        writer.write_bytes(&FullBoxHeader::new(version, flags).to_bytes())?;
        writer.write_bytes(aux_info_type.aux_info_type.as_bytes())?;
        writer.write_u32(aux_info_type.aux_info_type_parameter)
    }
}

/// Sizes of the sample auxiliary information of the samples, as a [`SampleAuxiliaryInformationSizesBox`] states them
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SampleInfoSizes {
    /// The information of every sample is `default_sample_info_size` bytes,
    /// and no table follows
    Constant {
        /// Size of the information of every sample, in bytes
        default_sample_info_size: NonZeroU32,
        /// Number of samples, from the first, that have information
        sample_count: u32,
    },
    /// The information of the samples has different sizes, which a table states
    Table {
        /// `sample_info_size` of every sample, in bytes, 0 for a sample of no
        /// information
        sample_info_sizes: Vec<u32>,
    },
}

impl Default for SampleInfoSizes {
    fn default() -> Self {
        Self::Table {
            sample_info_sizes: Vec::new(),
        }
    }
}

/// [`SampleAuxiliaryInformationSizesBox`] (`saiz`), ISO/IEC 14496-12 §8.7.8
///
/// The size of the sample auxiliary information of each sample, of one type
/// and stream, whose position a `SampleAuxiliaryInformationOffsetsBox` of the
/// same type and stream gives. Where the box states no type, the type is that
/// of the protection scheme of the track, or else its sample entry type.
///
/// The box is written in the `version` it holds, whose sizes are 8, 16 or 32
/// bits wide. A size wider than that is refused when the box is written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `default_sample_info_size` or `sample_info_size`.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{AuxInfoType, SampleAuxiliaryInformationSizesBox, SampleInfoSizes};
/// use isobmff_core::{BoxDecode, BoxEncode, FourCc};
///
/// // A box is built from its default, field by field
/// let mut aux_info_type = AuxInfoType::default();
/// aux_info_type.aux_info_type = FourCc::new(*b"cenc");
/// aux_info_type.aux_info_type_parameter = 1;
/// let mut sizes = SampleAuxiliaryInformationSizesBox::default();
/// sizes.aux_info_type = Some(aux_info_type);
/// sizes.sample_info_sizes = SampleInfoSizes::Table {
///     sample_info_sizes: vec![16, 0, 24],
/// };
///
/// // It writes the bytes its syntax lays out, and reads back from them
/// let mut buffer = [0; 28];
/// sizes.encode(&mut buffer).unwrap();
/// assert_eq!(
///     &buffer,
///     b"\0\0\0\x1csaiz\0\0\0\x01cenc\0\0\0\x01\0\0\0\0\x03\x10\0\x18"
/// );
/// assert_eq!(
///     SampleAuxiliaryInformationSizesBox::decode(&buffer),
///     Ok((sizes, b"".as_slice()))
/// );
/// ```
#[doc(alias = "saiz")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleAuxiliaryInformationSizesBox {
    /// Version of the box: 0, 1 or 2, for 8, 16 or 32-bit sizes
    pub version: u8,
    /// Type and stream of the information, which flag `0x1` says is there
    pub aux_info_type: Option<AuxInfoType>,
    /// Sizes of the information of the samples
    pub sample_info_sizes: SampleInfoSizes,
}

impl BoxDefinition for SampleAuxiliaryInformationSizesBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"saiz");
}

/// Returns how many bytes a size takes under `version`, or `None` for a version the box does not read
fn size_len(version: u8) -> Option<u64> {
    match version {
        0 => Some(1),
        1 => Some(2),
        2 => Some(4),
        _ => None,
    }
}

/// Reads a size as wide as `version` lays it out
fn read_size(reader: &mut FieldReader<'_>, version: u8) -> Result<u32, Error> {
    match version {
        0 => Ok(u32::from(u8::from_be_bytes(*reader.read_bytes::<1>()?))),
        1 => Ok(u32::from(reader.read_u16()?)),
        _ => reader.read_u32(),
    }
}

/// Writes a size as wide as `version` lays it out, naming `field` where it is too wide
fn write_size(
    writer: &mut FieldWriter<'_>,
    version: u8,
    size: u32,
    field: &'static str,
) -> Result<(), Error> {
    match version {
        0 => writer.write_bytes(&[u8::try_from(size).map_err(|_| Error::invalid_value(field))?]),
        1 => writer.write_u16(u16::try_from(size).map_err(|_| Error::invalid_value(field))?),
        _ => writer.write_u32(size),
    }
}

impl BoxDecode for SampleAuxiliaryInformationSizesBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let version = header.version();
        let size_len = size_len(version).ok_or(Error::unsupported_version(version))?;
        let aux_info_type = AuxInfoType::read_if_flagged(header, reader)?;
        let default_sample_info_size = read_size(reader, version)?;
        let sample_count = reader.read_u32()?;
        let sample_info_sizes = match NonZeroU32::new(default_sample_info_size) {
            Some(default_sample_info_size) => SampleInfoSizes::Constant {
                default_sample_info_size,
                sample_count,
            },
            None => SampleInfoSizes::Table {
                sample_info_sizes: read_entries(reader, sample_count, size_len, |reader| {
                    read_size(reader, version)
                })?,
            },
        };

        Ok(Self {
            version,
            aux_info_type,
            sample_info_sizes,
        })
    }
}

impl BoxEncode for SampleAuxiliaryInformationSizesBox {
    fn payload_len(&self) -> u64 {
        let size_len = size_len(self.version).unwrap_or_default();
        let table_len = match &self.sample_info_sizes {
            SampleInfoSizes::Constant { .. } => 0,
            SampleInfoSizes::Table { sample_info_sizes } => {
                table_len(sample_info_sizes.len(), size_len)
            }
        };

        table_len
            .saturating_add(AuxInfoType::header_len(self.aux_info_type))
            .saturating_add(size_len)
            .saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        if size_len(self.version).is_none() {
            return Err(Error::unsupported_version(self.version));
        }

        AuxInfoType::write_with_header(self.aux_info_type, self.version, writer)?;
        match &self.sample_info_sizes {
            SampleInfoSizes::Constant {
                default_sample_info_size,
                sample_count,
            } => {
                write_size(
                    writer,
                    self.version,
                    default_sample_info_size.get(),
                    "default_sample_info_size",
                )?;
                writer.write_u32(*sample_count)
            }
            SampleInfoSizes::Table { sample_info_sizes } => {
                write_size(writer, self.version, 0, "default_sample_info_size")?;
                write_entry_count(writer, sample_info_sizes.len())?;
                sample_info_sizes.iter().try_for_each(|size| {
                    write_size(writer, self.version, *size, "sample_info_size")
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FourCc};

    use super::{AuxInfoType, SampleAuxiliaryInformationSizesBox, SampleInfoSizes};
    use crate::encoded;

    /// `saiz` of version 0 and no type: 100 samples of 16 bytes of information each
    const SAIZ_CONSTANT: &[u8] = b"\0\0\0\x11saiz\0\0\0\0\x10\0\0\0\x64";

    /// `saiz` of version 0 and type `cenc`, parameter 1: a table of three
    /// samples, of 16, 0 and 24 bytes
    const SAIZ_0: &[u8] = b"\0\0\0\x1csaiz\0\0\0\x01cenc\0\0\0\x01\0\0\0\0\x03\x10\0\x18";

    /// The same table in version 1
    const SAIZ_1: &[u8] = b"\0\0\0\x20saiz\x01\0\0\x01cenc\0\0\0\x01\0\0\0\0\0\x03\0\x10\0\0\0\x18";

    /// The same table in version 2
    const SAIZ_2: &[u8] = b"\0\0\0\x28saiz\x02\0\0\x01cenc\0\0\0\x01\0\0\0\0\0\0\0\x03\
        \0\0\0\x10\0\0\0\0\0\0\0\x18";

    fn table(version: u8) -> SampleAuxiliaryInformationSizesBox {
        SampleAuxiliaryInformationSizesBox {
            version,
            aux_info_type: Some(AuxInfoType {
                aux_info_type: FourCc::new(*b"cenc"),
                aux_info_type_parameter: 1,
            }),
            sample_info_sizes: SampleInfoSizes::Table {
                sample_info_sizes: vec![16, 0, 24],
            },
        }
    }

    fn constant() -> SampleAuxiliaryInformationSizesBox {
        SampleAuxiliaryInformationSizesBox {
            version: 0,
            aux_info_type: None,
            sample_info_sizes: SampleInfoSizes::Constant {
                default_sample_info_size: NonZeroU32::new(16).unwrap(),
                sample_count: 100,
            },
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_every_version() {
        assert_eq!(
            [
                SampleAuxiliaryInformationSizesBox::decode(SAIZ_CONSTANT),
                SampleAuxiliaryInformationSizesBox::decode(SAIZ_0),
                SampleAuxiliaryInformationSizesBox::decode(SAIZ_1),
                SampleAuxiliaryInformationSizesBox::decode(SAIZ_2)
            ],
            [
                Ok((constant(), b"".as_slice())),
                Ok((table(0), b"".as_slice())),
                Ok((table(1), b"".as_slice())),
                Ok((table(2), b"".as_slice()))
            ]
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_every_version() {
        assert_eq!(
            [
                encoded(&constant()),
                encoded(&table(0)),
                encoded(&table(1)),
                encoded(&table(2))
            ],
            [
                SAIZ_CONSTANT.to_vec(),
                SAIZ_0.to_vec(),
                SAIZ_1.to_vec(),
                SAIZ_2.to_vec()
            ]
        );
    }

    #[test]
    fn a_size_wider_than_its_version_carries_is_refused_when_written() {
        let mut wide = table(0);
        wide.sample_info_sizes = SampleInfoSizes::Table {
            sample_info_sizes: vec![0x100],
        };

        assert_eq!(
            wide.encode(&mut [0; 26]),
            Err(Error::invalid_value("sample_info_size"))
        );
    }

    #[test]
    fn a_sample_count_of_more_sizes_than_the_payload_holds_is_refused() {
        assert_eq!(
            SampleAuxiliaryInformationSizesBox::decode_payload(b"\x01\0\0\0\0\0\0\0\0\x02\0\x10"),
            Err(Error::truncated_payload(14, 12))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SampleAuxiliaryInformationSizesBox::decode_payload(b"\x03\0\0\0"),
            Err(Error::unsupported_version(3))
        );
    }
}
