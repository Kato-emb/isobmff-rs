//! [`SampleSizeBox`] (`stsz`), ISO/IEC 14496-12 §8.7.3.2

use alloc::vec::Vec;
use core::num::NonZeroU32;

use isobmff_core::{
    BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWidth, FieldWriter,
    FullBoxFields, FullBoxFlags,
};

/// Length of the fields that precede the entries
const FIXED_FIELDS_LEN: u64 = 12;

/// Length of one entry of the table
const ENTRY_LEN: u64 = 4;

/// One entry of the table a [`SampleSizeBox`] holds
///
/// The entry states the size of the sample it is indexed by, which is what lets
/// the media data be read without framing of its own.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SampleSizeEntry {
    entry_size: u32,
}

impl SampleSizeEntry {
    /// Creates the entry from the bytes its sample occupies
    #[must_use]
    pub const fn new(entry_size: u32) -> Self {
        Self { entry_size }
    }

    /// Returns how many bytes the sample occupies
    #[must_use]
    pub const fn entry_size(&self) -> u32 {
        self.entry_size
    }
}

/// The sizes of the samples of a track, stated one way or the other
///
/// ISO/IEC 14496-12 §8.7.3.2 states them as a size every sample shares, or as a
/// table of one size per sample. The wire marks the second by writing the shared
/// size as zero, which is why the shared size is a [`NonZeroU32`]: the two ways
/// are held apart here, and neither can be written as the other.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SampleSizes {
    /// Every sample of the track occupies the same number of bytes
    Uniform {
        /// Bytes each sample of the track occupies
        sample_size: NonZeroU32,
        /// Samples the track holds
        sample_count: u32,
    },
    /// Each sample occupies what its own entry states
    PerSample(Vec<SampleSizeEntry>),
}

/// Box that states how many bytes each sample of a track occupies
///
/// [`SampleSizeBox`] (`stsz`), ISO/IEC 14496-12 §8.7.3.2. The sizes come either
/// as one shared by every sample or as a table of one per sample —
/// [`SampleSizes`] is that choice — and either way the box counts the samples
/// of the track. A track whose samples all live in fragments states no sizes
/// here at all. A `stz2` states the same thing with narrower fields, and is a
/// box of its own.
///
/// The `sample_count` of a per-sample table is not held: it counts the entries,
/// so it is derived on the way out. On the way in a count that disagrees with
/// the entries fails the box.
///
/// # Examples
///
/// ```
/// use core::num::NonZeroU32;
///
/// use isobmff_boxes::{SampleSizeBox, SampleSizes};
/// use isobmff_core::BoxWrite;
///
/// // A track whose samples all occupy the same number of bytes
/// let uniform = SampleSizeBox::new(SampleSizes::Uniform {
///     sample_size: NonZeroU32::new(1_024).unwrap(),
///     sample_count: 8,
/// });
///
/// // A size every sample shares leaves no table on the wire
/// assert_eq!(uniform.encoded_len(), 20);
///
/// // A track whose samples are all described by fragments states none here
/// let fragmented = SampleSizeBox::new(SampleSizes::PerSample(Vec::new()));
///
/// assert_eq!(fragmented.encoded_len(), 20);
/// ```
#[doc(alias = "stsz")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleSizeBox {
    sample_sizes: SampleSizes,
}

impl SampleSizeBox {
    /// Creates the box from the sizes it states for the samples
    #[must_use]
    pub const fn new(sample_sizes: SampleSizes) -> Self {
        Self { sample_sizes }
    }

    /// Returns the sizes of the samples, as the box states them
    #[must_use]
    pub const fn sample_sizes(&self) -> &SampleSizes {
        &self.sample_sizes
    }
}

impl BoxDefinition for SampleSizeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsz");
}

impl BoxDecode for SampleSizeBox {
    /// # Errors
    ///
    /// * [`UnsupportedVersion`](isobmff_core::ErrorKind::UnsupportedVersion): the box
    ///   declares a version other than 0.
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the payload
    ///   ends inside a field of the box or inside one of its entries.
    /// * [`EntryCountMismatch`](isobmff_core::ErrorKind::EntryCountMismatch): the
    ///   `sample_count` field disagrees with the entries that follow it.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let version = FullBoxFields::from_bytes(reader.read_bytes::<4>()?).version();
        if version != 0 {
            return Err(Error::unsupported_version(version));
        }

        let sample_size = reader.read_u32()?;
        let sample_count = reader.read_u32()?;

        if let Some(sample_size) = NonZeroU32::new(sample_size) {
            return Ok(Self {
                sample_sizes: SampleSizes::Uniform {
                    sample_size,
                    sample_count,
                },
            });
        }

        let mut entries = Vec::new();
        while !reader.remainder().is_empty() {
            entries.push(SampleSizeEntry {
                entry_size: reader.read_u32()?,
            });
        }

        let declared = u64::from(sample_count);
        let actual = u64::try_from(entries.len()).unwrap_or(u64::MAX);
        if actual != declared {
            return Err(Error::entry_count_mismatch(declared, actual));
        }

        Ok(Self {
            sample_sizes: SampleSizes::PerSample(entries),
        })
    }
}

impl BoxEncode for SampleSizeBox {
    fn payload_len(&self) -> u64 {
        let entries = match &self.sample_sizes {
            SampleSizes::Uniform { .. } => 0,
            SampleSizes::PerSample(entries) => u64::try_from(entries.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(ENTRY_LEN),
        };

        FIXED_FIELDS_LEN.saturating_add(entries)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_bytes(&FullBoxFields::new(0, FullBoxFlags::ZERO).to_bytes())?;

        match &self.sample_sizes {
            SampleSizes::Uniform {
                sample_size,
                sample_count,
            } => {
                writer.write_u32(sample_size.get())?;
                writer.write_u32(*sample_count)?;
            }
            SampleSizes::PerSample(entries) => {
                writer.write_u32(0)?;
                let sample_count = u64::try_from(entries.len()).unwrap_or(u64::MAX);
                // Why not saturate silently: a sample count past `u32` cannot be
                // written at all, and the box has already declared a length built
                // from it, so this stands for a `Vec` no target can hold.
                writer.write_unsigned(FieldWidth::Compact, sample_count)?;

                for entry in entries {
                    writer.write_u32(entry.entry_size)?;
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
    use core::num::NonZeroU32;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{SampleSizeBox, SampleSizeEntry, SampleSizes};

    /// Sizes shared by a track of eight samples
    fn uniform_sizes() -> SampleSizes {
        SampleSizes::Uniform {
            sample_size: NonZeroU32::new(1_024).unwrap(),
            sample_count: 8,
        }
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(sample_size: &SampleSizeBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(sample_size.payload_len()).unwrap()];
        sample_size.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_stating_one_size_for_every_sample_reads_back_as_the_value_that_wrote_it() {
        let sample_size = SampleSizeBox::new(uniform_sizes());

        let payload = encoded_payload(&sample_size);

        assert_eq!(payload, b"\0\0\0\0\0\0\x04\0\0\0\0\x08");
        assert_eq!(
            SampleSizeBox::decode_payload(&payload).unwrap(),
            sample_size
        );
    }

    #[test]
    fn a_box_stating_a_size_per_sample_reads_back_as_the_value_that_wrote_it() {
        let sample_size = SampleSizeBox::new(SampleSizes::PerSample(vec![
            SampleSizeEntry::new(1_024),
            SampleSizeEntry::new(512),
        ]));

        let payload = encoded_payload(&sample_size);

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0\0\0\0\x02\0\0\x04\0\0\0\x02\0");
        assert_eq!(
            SampleSizeBox::decode_payload(&payload).unwrap(),
            sample_size
        );
    }

    #[test]
    fn a_box_stating_the_sizes_of_no_samples_declares_a_count_of_zero() {
        let payload = encoded_payload(&SampleSizeBox::new(SampleSizes::PerSample(Vec::new())));

        assert_eq!(payload, b"\0\0\0\0\0\0\0\0\0\0\0\0");
        assert_eq!(
            SampleSizeBox::decode_payload(&payload).unwrap(),
            SampleSizeBox::new(SampleSizes::PerSample(Vec::new()))
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_entries_is_rejected() {
        let mut payload = encoded_payload(&SampleSizeBox::new(SampleSizes::PerSample(vec![
            SampleSizeEntry::new(1_024),
        ])));
        payload
            .get_mut(8..12)
            .unwrap()
            .copy_from_slice(&4_u32.to_be_bytes());

        assert_eq!(
            SampleSizeBox::decode_payload(&payload),
            Err(Error::entry_count_mismatch(4, 1))
        );
    }

    #[test]
    fn a_table_following_a_size_every_sample_shares_is_rejected() {
        let payload = [
            encoded_payload(&SampleSizeBox::new(uniform_sizes())),
            vec![0; 4],
        ]
        .concat();

        assert_eq!(
            SampleSizeBox::decode_payload(&payload),
            Err(Error::trailing_payload(12, 16))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_rejected() {
        let payload = b"\x01\0\0\0\0\0\0\0\0\0\0\0";

        assert_eq!(
            SampleSizeBox::decode_payload(payload),
            Err(Error::unsupported_version(1))
        );
    }
}
