//! [`MediaDataWriter`], which writes the media data of samples into an `mdat`

use alloc::vec::Vec;

use isobmff_boxes::{MediaDataBox, MovieBox};
use isobmff_core::{BoxDefinition, BoxHeader};

use crate::error::MuxError;
use crate::framing::{BoxEvent, BoxWriter};
use crate::length::to_u64;
use crate::sample::{Sample, SampleLocation};
use crate::structure_data::laid_out;

/// Writer of the media data of samples written together: one `mdat` that holds them in their order, ISO/IEC 14496-12 §8.1.1
///
/// The writer does no I/O. It returns the bytes of the `mdat`, and the
/// [`SampleLocation`] of each sample it holds, which a
/// [`SampleTableWriter`](crate::structure_data::SampleTableWriter) declares
/// in the sample tables of a movie.
///
/// The [`media_data`](crate::media_data) module documentation writes a file
/// with it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default)]
pub struct MediaDataWriter;

impl MediaDataWriter {
    /// Creates the writer
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the header of the `mdat` of `samples`
    ///
    /// # Errors
    ///
    /// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of the samples together is longer than a box holds.
    pub fn header(&self, samples: &[Sample]) -> Result<BoxHeader, MuxError> {
        let payload_len = samples
            .iter()
            .map(|sample| to_u64(sample.data().len()))
            .fold(0, u64::saturating_add);
        BoxHeader::with_payload_len(MediaDataBox::BOX_TYPE, payload_len).ok_or(
            MuxError::DataOffsetOutOfRange {
                track_id: samples
                    .first()
                    .map_or(0, |sample| sample.properties().track_id),
            },
        )
    }

    /// Writes the `mdat` `header` opens, holding the media data of `samples` of the tracks of `movie` in their order, through `boxes`, and returns its bytes and the location of each sample
    ///
    /// The byte range of each location is the file offsets of its media data
    /// in the `mdat`, counted from where `boxes` writes it, and its key the
    /// one a [`StructureDataReader`](crate::structure_data::StructureDataReader)
    /// reading `movie` gives the data reference entry the sample entry of the
    /// sample names (§8.7.2): the locations are those a reader resolves out
    /// of the file. A refused call writes nothing.
    ///
    /// # Errors
    ///
    /// * [`UnknownTrackId`](MuxError::UnknownTrackId): a sample is of a track
    ///   the movie has none of.
    /// * [`UnknownSampleDescriptionIndex`](MuxError::UnknownSampleDescriptionIndex),
    ///   [`UnknownDataReferenceIndex`](MuxError::UnknownDataReferenceIndex),
    ///   [`ExternalDataReference`](MuxError::ExternalDataReference),
    ///   [`UnsupportedDataReference`](MuxError::UnsupportedDataReference): the
    ///   track has no sample entry the sample names, or its entry names a data
    ///   reference entry that is not the file itself, which the `mdat` is in.
    /// * The failures of [`BoxWriter::handle_event`], such as
    ///   [`PayloadLengthMismatch`](MuxError::PayloadLengthMismatch) where
    ///   `header` is not the header of the `mdat` of `samples`.
    /// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of a sample lies past 64-bit file offsets.
    pub fn write(
        &self,
        movie: &MovieBox,
        boxes: &mut BoxWriter,
        header: BoxHeader,
        samples: &[Sample],
    ) -> Result<(Vec<u8>, Vec<SampleLocation>), MuxError> {
        let mut bytes = Vec::with_capacity(
            usize::try_from(header.size().total_bytes().unwrap_or_default()).unwrap_or_default(),
        );
        let locations = laid_out(
            movie,
            samples,
            boxes.next_offset().saturating_add(header.encoded_len()),
        )?;
        bytes.extend_from_slice(boxes.handle_event(BoxEvent::Header(header))?);
        for sample in samples {
            bytes.extend_from_slice(boxes.handle_event(BoxEvent::Payload(sample.data()))?);
        }
        bytes.extend_from_slice(boxes.handle_event(BoxEvent::End)?);

        Ok((bytes, locations))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::MediaDataWriter;
    use crate::error::MuxError;
    use crate::framing::BoxWriter;
    use crate::sample::SampleLocation;
    use crate::structure_data::fixtures::{UNLIMITED, in_another_file, movie, sample, track};
    use crate::structure_data::{SampleTableWriter, StructureDataReader};

    #[test]
    fn the_locations_of_the_samples_written_are_those_a_reader_resolves_out_of_the_file() {
        let movie = movie([1, 2].map(|track_id| track(track_id, 0, &[], &[], &[])));
        let samples = [
            sample(1, 0, 10, 0, true),
            sample(2, 0, 10, 0, true),
            sample(1, 10, 10, 0, true),
        ];
        let mut boxes = BoxWriter::new();
        let media_data = MediaDataWriter::new();
        let header = media_data.header(&samples).unwrap();

        let (_, locations) = media_data
            .write(&movie, &mut boxes, header, &samples)
            .unwrap();

        let mut tables = SampleTableWriter::new(movie).unwrap();
        tables.handle_locations(&locations).unwrap();
        let read: Vec<SampleLocation> = StructureDataReader::new(UNLIMITED)
            .handle_movie(tables.finish().clone())
            .unwrap()
            .collect();
        assert_eq!(read, locations);
    }

    #[test]
    fn samples_whose_media_data_is_not_in_the_file_itself_are_refused_and_nothing_is_written() {
        let movie = movie([
            track(1, 0, &[], &[], &[]),
            in_another_file(track(2, 0, &[], &[], &[])),
        ]);
        let samples = [sample(1, 0, 10, 0, true), sample(2, 0, 10, 0, true)];
        let mut boxes = BoxWriter::new();
        let media_data = MediaDataWriter::new();
        let header = media_data.header(&samples).unwrap();

        assert_eq!(
            (
                media_data
                    .write(&movie, &mut boxes, header, &samples)
                    .map(|_| ()),
                boxes.next_offset(),
            ),
            (
                Err(MuxError::ExternalDataReference {
                    track_id: 2,
                    data_reference_index: 1
                }),
                0
            )
        );
    }
}
