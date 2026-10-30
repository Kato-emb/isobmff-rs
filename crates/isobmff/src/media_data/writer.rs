//! [`MediaDataWriter`], which writes the media data of samples into an `mdat`

use alloc::vec::Vec;

use isobmff_boxes::MediaDataBox;
use isobmff_core::{BoxDefinition, BoxHeader};

use crate::error::MuxError;
use crate::framing::{BoxEvent, BoxWriter, to_u64};
use crate::sample::Sample;

/// Writer of the media data of samples written together: one `mdat` that holds them in their order, ISO/IEC 14496-12 §8.1.1
#[derive(Debug, Default)]
pub(crate) struct MediaDataWriter;

impl MediaDataWriter {
    /// Returns the header of the `mdat` of `samples`
    ///
    /// # Errors
    ///
    /// * [`DataOffsetOutOfRange`](MuxError::DataOffsetOutOfRange): the media
    ///   data of the samples together is longer than a box holds.
    pub(crate) fn header(samples: &[Sample]) -> Result<BoxHeader, MuxError> {
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

    /// Writes the `mdat` `header` opens, holding the media data of `samples`, through `boxes`, and returns its bytes
    ///
    /// # Errors
    ///
    /// * The failures of [`BoxWriter::handle_event`].
    pub(crate) fn write(
        &self,
        boxes: &mut BoxWriter,
        header: BoxHeader,
        samples: &[Sample],
    ) -> Result<Vec<u8>, MuxError> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(boxes.handle_event(BoxEvent::Header(header))?);
        for sample in samples {
            bytes.extend_from_slice(boxes.handle_event(BoxEvent::Payload(sample.data()))?);
        }
        bytes.extend_from_slice(boxes.handle_event(BoxEvent::End)?);

        Ok(bytes)
    }
}
