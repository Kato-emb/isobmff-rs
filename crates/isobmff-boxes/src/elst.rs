//! [`EditListBox`] and the [`EditListEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxHeader,
};

use crate::full_box::version_width;

/// Flag value of `RepeatEdits`
const REPEAT_EDITS: u8 = 0x01;

/// One edit of an [`EditListBox`], which maps part of the presentation timeline to the media timeline
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct EditListEntry {
    /// Duration of the edit, in the timescale of the `MovieHeaderBox`
    pub edit_duration: u64,
    /// Starting time of the edit within the media, in the media timescale and in composition time
    ///
    /// -1 makes the edit an empty edit, which maps the edit to no media.
    pub media_time: i64,
    /// Integer part of the rate to play the media of the edit at, a 16.16 fixed-point number with `media_rate_fraction`
    ///
    /// A rate of 0 makes the edit a dwell: the media at `media_time` is
    /// presented for `edit_duration`.
    pub media_rate_integer: i16,
    /// Fractional part of the rate to play the media of the edit at
    pub media_rate_fraction: i16,
}

/// [`EditListBox`] (`elst`), ISO/IEC 14496-12 §8.6.6
///
/// Explicit timeline map of a track: each edit maps part of the presentation
/// timeline to part of the media timeline, to empty time, or to a dwell on a
/// single time-point of the media.
///
/// The box is written in the `version` it holds: version 0 carries
/// `edit_duration` and `media_time` in 32 bits, version 1 in 64. A box read
/// from a file keeps the version it was read in. Of the `flags`,
/// `RepeatEdits` is held, and the others, which are reserved, are read past
/// and written as zero.
///
/// # Untrusted input
///
/// The box reads the `entry_count` edits it states, after holding the count
/// against the payload, so what it allocates is bounded by the length of the
/// payload, which the caller sets.
#[doc(alias = "elst")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct EditListBox {
    /// Version of the box, 0 or 1, which sets the width of `edit_duration` and `media_time`
    pub version: u8,
    /// Whether the edit list is repeated, `RepeatEdits` of the `flags`
    pub repeat_edits: bool,
    /// Edits, in the order they came
    pub entries: Vec<EditListEntry>,
}

impl BoxDefinition for EditListBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"elst");
}

/// Returns how many bytes one edit takes in `version`
fn entry_len(version: u8) -> u64 {
    if version == 1 {
        8 + 8 + 2 + 2
    } else {
        4 + 4 + 2 + 2
    }
}

impl BoxDecode for EditListBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let header = FullBoxHeader::from_bytes(reader.read_bytes::<4>()?);
        let width = version_width(header.version())?;
        let [.., flags] = header.flags().bits().to_be_bytes();
        let entry_count = reader.read_u32()?;
        reader.require(u64::from(entry_count).saturating_mul(entry_len(header.version())))?;
        let mut entries = Vec::with_capacity(usize::try_from(entry_count).unwrap_or_default());
        for _ in 0..entry_count {
            entries.push(EditListEntry {
                edit_duration: reader.read_unsigned(width)?,
                media_time: reader.read_signed(width)?,
                media_rate_integer: reader.read_i16()?,
                media_rate_fraction: reader.read_i16()?,
            });
        }

        Ok(Self {
            version: header.version(),
            repeat_edits: flags & REPEAT_EDITS != 0,
            entries,
        })
    }
}

impl BoxEncode for EditListBox {
    fn payload_len(&self) -> u64 {
        u64::try_from(self.entries.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(entry_len(self.version))
            .saturating_add(4 + 4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let width = version_width(self.version)?;
        let flags = if self.repeat_edits { REPEAT_EDITS } else { 0 };
        writer.write_bytes(&[self.version, 0, 0, flags])?;
        writer.write_unsigned(
            FieldWidth::Compact,
            u64::try_from(self.entries.len()).unwrap_or(u64::MAX),
        )?;
        for entry in &self.entries {
            writer.write_unsigned(width, entry.edit_duration)?;
            writer.write_signed(width, entry.media_time)?;
            writer.write_i16(entry.media_rate_integer)?;
            writer.write_i16(entry.media_rate_fraction)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error, FieldWidth};

    use super::{EditListBox, EditListEntry};
    use crate::encoded;

    /// `elst` of version 0: an empty edit of 1000, then 30000 of the media from 2000 at rate 1.0
    const ELST_V0: &[u8] = b"\0\0\0\x28elst\0\0\0\0\0\0\0\x02\
        \0\0\x03\xe8\xff\xff\xff\xff\0\x01\0\0\
        \0\0\x75\x30\0\0\x07\xd0\0\x01\0\0";

    /// `elst` of version 1, repeated: an empty edit of 1000, then 2^33 of the media from 2000 at rate 1.5
    const ELST_V1: &[u8] = b"\0\0\0\x38elst\x01\0\0\x01\0\0\0\x02\
        \0\0\0\0\0\0\x03\xe8\xff\xff\xff\xff\xff\xff\xff\xff\0\x01\0\0\
        \0\0\0\x02\0\0\0\0\0\0\0\0\0\0\x07\xd0\0\x01\x80\0";

    fn edit(edit_duration: u64, media_time: i64, media_rate_fraction: i16) -> EditListEntry {
        EditListEntry {
            edit_duration,
            media_time,
            media_rate_integer: 1,
            media_rate_fraction,
        }
    }

    fn edit_list_v0() -> EditListBox {
        EditListBox {
            version: 0,
            repeat_edits: false,
            entries: vec![edit(1000, -1, 0), edit(30000, 2000, 0)],
        }
    }

    fn edit_list_v1() -> EditListBox {
        EditListBox {
            version: 1,
            repeat_edits: true,
            entries: vec![edit(1000, -1, 0), edit(1 << 33, 2000, -0x8000)],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out_in_either_version() {
        assert_eq!(
            (EditListBox::decode(ELST_V0), EditListBox::decode(ELST_V1)),
            (
                Ok((edit_list_v0(), b"".as_slice())),
                Ok((edit_list_v1(), b"".as_slice()))
            )
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out_in_the_version_it_holds() {
        assert_eq!(
            (encoded(&edit_list_v0()), encoded(&edit_list_v1())),
            (ELST_V0.to_vec(), ELST_V1.to_vec())
        );
    }

    #[test]
    fn reserved_flags_are_read_past_and_written_as_zero() {
        let (read, _) = EditListBox::decode(b"\0\0\0\x10elst\0\0\x01\x02\0\0\0\0").unwrap();

        assert_eq!(encoded(&read), b"\0\0\0\x10elst\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn version_0_refuses_a_duration_or_a_media_time_wider_than_32_bits() {
        let mut wide_duration = edit_list_v0();
        wide_duration.entries = vec![edit(1 << 32, 2000, 0)];
        let mut wide_media_time = edit_list_v0();
        wide_media_time.entries = vec![edit(30000, -(1 << 31) - 1, 0)];

        assert_eq!(
            (
                wide_duration.encode(&mut [0; 28]),
                wide_media_time.encode(&mut [0; 28])
            ),
            (
                Err(Error::out_of_range(1 << 32, FieldWidth::Compact)),
                Err(Error::out_of_range(
                    (-(1_i64 << 31) - 1).cast_unsigned(),
                    FieldWidth::Compact
                ))
            )
        );
    }

    #[test]
    fn a_count_the_payload_cannot_hold_is_refused_before_an_edit_is_read() {
        assert_eq!(
            EditListBox::decode_payload(b"\0\0\0\0\xff\xff\xff\xff\0\0\x03\xe8"),
            Err(Error::truncated_payload(8 + u64::from(u32::MAX) * 12, 12))
        );
    }

    #[test]
    fn a_payload_ending_inside_an_edit_is_refused() {
        assert_eq!(
            EditListBox::decode_payload(ELST_V0.get(8..38).unwrap()),
            Err(Error::truncated_payload(32, 30))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused_when_read_and_when_written() {
        let mut version_2 = edit_list_v0();
        version_2.version = 2;

        assert_eq!(
            (
                EditListBox::decode_payload(b"\x02\0\0\0\0\0\0\0"),
                version_2.encode(&mut [0; 40])
            ),
            (
                Err(Error::unsupported_version(2)),
                Err(Error::unsupported_version(2))
            )
        );
    }
}
