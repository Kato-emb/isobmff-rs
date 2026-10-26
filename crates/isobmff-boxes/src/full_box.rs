//! The `version` and `flags` a full box opens with, as the boxes of this crate read them

use isobmff_core::{Error, FieldReader, FieldWidth, FieldWriter, FullBoxFlags, FullBoxHeader};

/// Reads the `version` and `flags` of a full box that defines version 0 alone, and returns the flags
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside the fields.
/// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
///   version is not 0.
pub(crate) fn read_version_zero(reader: &mut FieldReader<'_>) -> Result<FullBoxFlags, Error> {
    let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
    if header.version() != 0 {
        return Err(Error::unsupported_version(header.version()));
    }

    Ok(header.flags())
}

/// Writes the `version` and `flags` of a full box of version 0 whose flags are all zero
///
/// # Errors
///
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
///   buffer ends inside the fields.
pub(crate) fn write_version_zero(writer: &mut FieldWriter<'_>) -> Result<(), Error> {
    writer.write_bytes(&FullBoxHeader::new(0, FullBoxFlags::ZERO).to_bytes())
}

/// Reads a composition offset of 32 bits, unsigned under `version` 0 and signed under any other
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside the field.
pub(crate) fn read_composition_offset(
    reader: &mut FieldReader<'_>,
    version: u8,
) -> Result<i64, Error> {
    if version == 0 {
        reader.read_u32().map(i64::from)
    } else {
        reader.read_i32().map(i64::from)
    }
}

/// Writes a composition offset of 32 bits, unsigned under `version` 0 and signed under any other
///
/// # Errors
///
/// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange): `offset` is
///   outside what the field carries under `version`.
/// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
///   buffer ends inside the field.
pub(crate) fn write_composition_offset(
    writer: &mut FieldWriter<'_>,
    version: u8,
    offset: i64,
) -> Result<(), Error> {
    if version == 0 {
        let unsigned = u64::try_from(offset)
            .map_err(|_| Error::out_of_range(offset.cast_unsigned(), FieldWidth::Compact))?;
        writer.write_unsigned(FieldWidth::Compact, unsigned)
    } else {
        writer.write_signed(FieldWidth::Compact, offset)
    }
}

/// Returns the width of the fields that version 1 of a box widens to 64 bits
///
/// # Errors
///
/// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the
///   version is neither 0 nor 1.
pub(crate) fn version_width(version: u8) -> Result<FieldWidth, Error> {
    match version {
        0 => Ok(FieldWidth::Compact),
        1 => Ok(FieldWidth::Extended),
        _ => Err(Error::unsupported_version(version)),
    }
}
