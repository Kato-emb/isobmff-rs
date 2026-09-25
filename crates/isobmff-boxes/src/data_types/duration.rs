//! The `duration` of a movie, track or media header, ISO/IEC 14496-12 §8.2.2.3, §8.3.2.3 and §8.4.2.3

use isobmff_core::{Error, FieldReader, FieldWidth, FieldWriter};

/// All 1s of a field of 32 bits
const ALL_ONES_COMPACT: u64 = u32::MAX as u64;

/// Returns the all 1s of a field of the given width
fn all_ones(width: FieldWidth) -> u64 {
    if width == FieldWidth::Compact {
        ALL_ONES_COMPACT
    } else {
        u64::MAX
    }
}

/// Returns whether the duration can be written in 32 bits without reading back as one that cannot be determined
pub(crate) const fn fits_in_32_bits(duration: Option<u64>) -> bool {
    match duration {
        Some(duration) => duration < ALL_ONES_COMPACT,
        None => true,
    }
}

/// Reads a header's `duration`, `None` for the all 1s the spec sets when it cannot be determined
///
/// # Errors
///
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
///   ends inside the field.
pub(crate) fn read_duration(
    reader: &mut FieldReader<'_>,
    width: FieldWidth,
) -> Result<Option<u64>, Error> {
    let duration = reader.read_unsigned(width)?;

    Ok((duration != all_ones(width)).then_some(duration))
}

/// Writes a header's `duration`, all 1s for one that cannot be determined
///
/// # Errors
///
/// * The failures of [`FieldWriter::write_unsigned`].
pub(crate) fn write_duration(
    writer: &mut FieldWriter<'_>,
    width: FieldWidth,
    duration: Option<u64>,
) -> Result<(), Error> {
    writer.write_unsigned(width, duration.unwrap_or(all_ones(width)))
}

#[cfg(test)]
mod tests {
    use isobmff_core::{FieldReader, FieldWidth, FieldWriter};

    use super::{read_duration, write_duration};

    #[test]
    fn a_duration_that_cannot_be_determined_is_all_1s_at_either_width() {
        let mut buffer = [0; 12];
        let mut writer = FieldWriter::new(&mut buffer);
        write_duration(&mut writer, FieldWidth::Compact, None).unwrap();
        write_duration(&mut writer, FieldWidth::Extended, None).unwrap();
        writer.finish().unwrap();

        assert_eq!(buffer, [0xff; 12]);

        let mut reader = FieldReader::new(&buffer);
        assert_eq!(read_duration(&mut reader, FieldWidth::Compact), Ok(None));
        assert_eq!(read_duration(&mut reader, FieldWidth::Extended), Ok(None));
    }
}
