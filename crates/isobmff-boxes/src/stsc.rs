//! [`SampleToChunkBox`] and the [`SampleToChunkEntry`]s it holds

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

use crate::full_box::{read_version_zero, write_version_zero};
use crate::table::{TableEntry, counted_len, read_counted, write_counted};

/// Run of chunks that share their samples per chunk and sample entry, one entry of a [`SampleToChunkBox`]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleToChunkEntry {
    /// Index of the first chunk of the run, the first chunk of the track being 1
    pub first_chunk: u32,
    /// Number of samples in each chunk of the run
    pub samples_per_chunk: u32,
    /// Index of the sample entry that describes the samples of the run, from 1
    pub sample_description_index: u32,
}

impl TableEntry for SampleToChunkEntry {
    const ENCODED_LEN: u64 = 12;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            first_chunk: reader.read_u32()?,
            samples_per_chunk: reader.read_u32()?,
            sample_description_index: reader.read_u32()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.first_chunk)?;
        writer.write_u32(self.samples_per_chunk)?;
        writer.write_u32(self.sample_description_index)
    }
}

/// [`SampleToChunkBox`] (`stsc`), ISO/IEC 14496-12 §8.7.4
///
/// The chunk each sample lies in, and the sample entry that describes it, as
/// runs of chunks: a run lasts until the `first_chunk` of the next entry.
#[doc(alias = "stsc")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct SampleToChunkBox {
    /// Runs of chunks, in chunk order
    pub entries: Vec<SampleToChunkEntry>,
}

impl BoxDefinition for SampleToChunkBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"stsc");
}

impl BoxDecode for SampleToChunkBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        read_version_zero(reader)?;

        Ok(Self {
            entries: read_counted(reader)?,
        })
    }
}

impl BoxEncode for SampleToChunkBox {
    fn payload_len(&self) -> u64 {
        counted_len(&self.entries).saturating_add(4)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_version_zero(writer)?;
        write_counted(writer, &self.entries)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, Error};

    use super::{SampleToChunkBox, SampleToChunkEntry};
    use crate::encoded;

    /// `stsc` of chunks of 4 samples from chunk 1 and of 2 from chunk 3, all of sample entry 1
    const STSC: &[u8] = b"\0\0\0\x28stsc\0\0\0\0\0\0\0\x02\
        \0\0\0\x01\0\0\0\x04\0\0\0\x01\0\0\0\x03\0\0\0\x02\0\0\0\x01";

    fn sample_to_chunk() -> SampleToChunkBox {
        SampleToChunkBox {
            entries: vec![
                SampleToChunkEntry {
                    first_chunk: 1,
                    samples_per_chunk: 4,
                    sample_description_index: 1,
                },
                SampleToChunkEntry {
                    first_chunk: 3,
                    samples_per_chunk: 2,
                    sample_description_index: 1,
                },
            ],
        }
    }

    #[test]
    fn reads_from_the_bytes_its_syntax_lays_out() {
        assert_eq!(
            SampleToChunkBox::decode(STSC),
            Ok((sample_to_chunk(), b"".as_slice()))
        );
    }

    #[test]
    fn writes_the_bytes_its_syntax_lays_out() {
        assert_eq!(encoded(&sample_to_chunk()), STSC);
    }

    #[test]
    fn a_payload_ending_inside_an_entry_is_refused() {
        assert_eq!(
            SampleToChunkBox::decode_payload(b"\0\0\0\0\0\0\0\x01\0\0\0\x01\0\0\0\x04"),
            Err(Error::truncated_payload(20, 16))
        );
    }

    #[test]
    fn a_version_the_box_does_not_read_is_refused() {
        assert_eq!(
            SampleToChunkBox::decode_payload(b"\x01\0\0\0\0\0\0\0"),
            Err(Error::unsupported_version(1))
        );
    }
}
