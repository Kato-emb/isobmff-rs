//! [`CompactSampleSizeBox`] (`stz2`), ISO/IEC 14496-12 §8.7.3.3

use alloc::vec::Vec;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

use crate::nibbles;

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 12;

/// Width of the entries of a [`CompactSampleSizeBox`]
///
/// ISO/IEC 14496-12 §8.7.3.3 has the `field_size` take the value 4, 8 or 16.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FieldSize {
    /// Entries of 4 bits, two to a byte
    Four,
    /// Entries of 8 bits
    Eight,
    /// Entries of 16 bits
    Sixteen,
}

impl FieldSize {
    /// Returns the largest size an entry of this width states
    const fn maximum(self) -> u16 {
        match self {
            Self::Four => 0x0f,
            Self::Eight => 0xff,
            Self::Sixteen => u16::MAX,
        }
    }
}

/// One entry of the table a [`CompactSampleSizeBox`] holds
///
/// The entry states the size of the sample it is indexed by, in the width the
/// box declares for all of them.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CompactSampleSizeEntry {
    entry_size: u16,
}

impl CompactSampleSizeEntry {
    /// Creates the entry from the bytes its sample occupies
    #[must_use]
    pub const fn new(entry_size: u16) -> Self {
        Self { entry_size }
    }

    /// Returns how many bytes the sample occupies
    #[must_use]
    pub const fn entry_size(&self) -> u16 {
        self.entry_size
    }
}

/// Box that states how many bytes each sample of a track occupies, in entries under 32 bits wide
///
/// [`CompactSampleSizeBox`] (`stz2`), ISO/IEC 14496-12 §8.7.3.3. The table holds
/// one entry per sample, every one in the width [`FieldSize`] declares; entries
/// of 4 bits are packed two to a byte, the earlier sample of each pair in the
/// high half, and the low half of the last byte of a table counting an odd
/// number of samples is read as nothing and written as zero.
///
/// The `sample_count` field is not held: it counts the entries, so it is
/// derived on the way out. On the way in a count that disagrees with the
/// entries fails the box. The reserved bits before the `field_size` are read
/// as nothing and written as zero.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{CompactSampleSizeBox, CompactSampleSizeEntry, FieldSize};
/// use isobmff_core::BoxEncode;
///
/// // Sizes that all fit 8 bits are stated one byte each
/// let compact = CompactSampleSizeBox::from_sizes([200, 16, 255]);
/// assert_eq!(compact.field_size(), FieldSize::Eight);
///
/// // The box header, the fixed fields, and one byte per sample
/// assert_eq!(compact.encoded_len(), 23);
///
/// // An entry wider than the width declared builds nothing
/// assert_eq!(
///     CompactSampleSizeBox::new(FieldSize::Four, vec![CompactSampleSizeEntry::new(16)]),
///     None
/// );
/// ```
#[doc(alias = "stz2")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CompactSampleSizeBox {
    field_size: FieldSize,
    entries: Vec<CompactSampleSizeEntry>,
}

impl CompactSampleSizeBox {
    /// Creates the box from the width of its entries and the entries it states the sizes with
    ///
    /// Returns `None` when an entry states a size the width does not hold.
    #[must_use]
    pub fn new(field_size: FieldSize, entries: Vec<CompactSampleSizeEntry>) -> Option<Self> {
        if entries
            .iter()
            .any(|entry| entry.entry_size > field_size.maximum())
        {
            return None;
        }

        Some(Self {
            field_size,
            entries,
        })
    }

    /// Creates the box from the size of every sample in turn, in the narrowest width they all fit
    #[must_use]
    pub fn from_sizes(sizes: impl IntoIterator<Item = u16>) -> Self {
        let entries: Vec<CompactSampleSizeEntry> =
            sizes.into_iter().map(CompactSampleSizeEntry::new).collect();
        let widest = entries
            .iter()
            .map(|entry| entry.entry_size)
            .max()
            .unwrap_or_default();

        let field_size = if widest <= FieldSize::Four.maximum() {
            FieldSize::Four
        } else if widest <= FieldSize::Eight.maximum() {
            FieldSize::Eight
        } else {
            FieldSize::Sixteen
        };

        Self {
            field_size,
            entries,
        }
    }

    /// Returns the width every entry is stated in
    #[must_use]
    pub const fn field_size(&self) -> FieldSize {
        self.field_size
    }

    /// Returns the entries, one per sample in decode order
    #[must_use]
    pub fn entries(&self) -> &[CompactSampleSizeEntry] {
        &self.entries
    }
}

impl BoxDefinition for CompactSampleSizeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stz2");
}

impl BoxDecode for CompactSampleSizeBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`UnsupportedFieldSize`](isobmff_core::ErrorKind::UnsupportedFieldSize): the
    ///   box declares a `field_size` other than 4, 8 or 16.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `sample_count` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let &[_, _, _, bits] = reader.read_bytes::<4>()?;
        let field_size = match bits {
            4 => FieldSize::Four,
            8 => FieldSize::Eight,
            16 => FieldSize::Sixteen,
            _ => return Err(Error::unsupported_field_size(bits)),
        };
        let declared = u64::from(reader.read_u32()?);

        let mut entries = Vec::new();
        match field_size {
            FieldSize::Four => entries.extend(
                nibbles::unpack(reader.take_remainder(), declared)
                    .into_iter()
                    .map(|entry_size| CompactSampleSizeEntry::new(u16::from(entry_size))),
            ),
            FieldSize::Eight => entries.extend(
                reader
                    .take_remainder()
                    .iter()
                    .map(|&entry_size| CompactSampleSizeEntry::new(u16::from(entry_size))),
            ),
            FieldSize::Sixteen => {
                while !reader.remainder().is_empty() {
                    entries.push(CompactSampleSizeEntry::new(reader.read_u16()?));
                }
            }
        }

        let actual = entries.len() as u64;
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self {
            field_size,
            entries,
        })
    }
}

impl BoxEncode for CompactSampleSizeBox {
    fn payload_len(&self) -> u64 {
        let count = self.entries.len() as u64;
        let entries = match self.field_size {
            FieldSize::Four => count.div_ceil(2),
            FieldSize::Eight => count,
            FieldSize::Sixteen => count.saturating_mul(2),
        };

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;
        let bits = match self.field_size {
            FieldSize::Four => 4,
            FieldSize::Eight => 8,
            FieldSize::Sixteen => 16,
        };
        writer.write_bytes(&[0, 0, 0, bits])?;
        let sample_count = self.entries.len() as u64;
        // Why not saturate silently: a sample count past `u32` cannot be written
        // at all, and the box has already declared a length built from it, so
        // this stands for a `Vec` no target can hold.
        writer.write_unsigned(FieldWidth::Compact, sample_count)?;

        // Why not narrowing with `try_from`: `new` and `from_sizes` keep every
        // entry within the width, so the bits left out are zero and the write
        // cannot fail on them.
        let low_byte = |entry: &CompactSampleSizeEntry| {
            let [_, low] = entry.entry_size.to_be_bytes();
            low
        };
        match self.field_size {
            FieldSize::Four => nibbles::pack(writer, self.entries.iter().map(low_byte))?,
            FieldSize::Eight => {
                for entry in &self.entries {
                    writer.write_bytes(&[low_byte(entry)])?;
                }
            }
            FieldSize::Sixteen => {
                for entry in &self.entries {
                    writer.write_u16(entry.entry_size)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{CompactSampleSizeBox, CompactSampleSizeEntry, FieldSize};

    /// Box of the entries stating `sizes`, in the width given
    fn compact(field_size: FieldSize, sizes: &[u16]) -> CompactSampleSizeBox {
        CompactSampleSizeBox::new(
            field_size,
            sizes
                .iter()
                .copied()
                .map(CompactSampleSizeEntry::new)
                .collect(),
        )
        .unwrap()
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(compact_sample_size: &CompactSampleSizeBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(compact_sample_size.payload_len()).unwrap()];
        compact_sample_size.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_of_each_width_reads_back_as_the_value_that_wrote_it() {
        let widths = [
            (
                compact(FieldSize::Four, &[7, 1, 15]),
                b"\0\0\0\0\0\0\0\x04\0\0\0\x03\x71\xf0".to_vec(),
            ),
            (
                compact(FieldSize::Eight, &[200, 3]),
                b"\0\0\0\0\0\0\0\x08\0\0\0\x02\xc8\x03".to_vec(),
            ),
            (
                compact(FieldSize::Sixteen, &[0x0102, 0xffff]),
                b"\0\0\0\0\0\0\0\x10\0\0\0\x02\x01\x02\xff\xff".to_vec(),
            ),
        ];

        for (compact_sample_size, written) in widths {
            let payload = encoded_payload(&compact_sample_size);

            assert_eq!(payload, written);
            assert_eq!(
                CompactSampleSizeBox::decode_payload(&payload).unwrap(),
                compact_sample_size
            );
        }
    }

    #[test]
    fn sizes_are_stated_in_the_narrowest_width_they_all_fit() {
        assert_eq!(
            CompactSampleSizeBox::from_sizes([15]).field_size(),
            FieldSize::Four
        );
        assert_eq!(
            CompactSampleSizeBox::from_sizes([15, 16]).field_size(),
            FieldSize::Eight
        );
        assert_eq!(
            CompactSampleSizeBox::from_sizes([256]).field_size(),
            FieldSize::Sixteen
        );
    }

    #[test]
    fn an_entry_wider_than_the_width_declared_builds_no_box() {
        assert_eq!(
            CompactSampleSizeBox::new(FieldSize::Four, vec![CompactSampleSizeEntry::new(16)]),
            None
        );
        assert_eq!(
            CompactSampleSizeBox::new(FieldSize::Eight, vec![CompactSampleSizeEntry::new(256)]),
            None
        );
    }

    #[test]
    fn a_width_the_box_does_not_read_is_rejected() {
        let payload = b"\0\0\0\0\0\0\0\x0c\0\0\0\0";

        assert_eq!(
            CompactSampleSizeBox::decode_payload(payload),
            Err(Error::unsupported_field_size(12))
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let four_bits = b"\0\0\0\0\0\0\0\x04\0\0\0\x03\x71";
        let eight_bits = b"\0\0\0\0\0\0\0\x08\0\0\0\x03\xc8\x03";

        assert_eq!(
            CompactSampleSizeBox::decode_payload(four_bits),
            Err(Error::entry_count_mismatch(3, 2))
        );
        assert_eq!(
            CompactSampleSizeBox::decode_payload(eight_bits),
            Err(Error::entry_count_mismatch(3, 2))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0\0\0\0\x08\0\0\0\0";

        assert_eq!(
            CompactSampleSizeBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
