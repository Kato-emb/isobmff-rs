//! The samples a sample table declares, with their timing and where their bytes lie
//!
//! A sample table, ISO/IEC 14496-12 §8.5, holds the tables
//! [`timeline`](super::timeline) reads the timing of its samples from, and
//! those [`data_layout`](super::data_layout) reads where their bytes lie
//! from. [`samples`] holds them against each other, and reads the samples of
//! tables that agree.

use core::iter::FusedIterator;

use isobmff_boxes::{
    ChunkOffsetBox, DataReferenceBox, SampleDescriptionBox, SampleSizeBox, SampleTableBox,
    SampleToChunkBox, TimeToSampleBox,
};
use isobmff_core::BoxDefinition;

use super::Error;
use super::data_layout::{self, SampleFramings};
use super::timeline::{self, SampleTimings};
use crate::{SampleLocation, SampleProperties};

/// Returns the location of each sample the `sample table` `stbl` of track `track_id` declares, in decoding order, once its tables agree with each other as the specification reads them
///
/// The `sample table`, ISO/IEC 14496-12 §3.1.50 and §8.5
///
/// `dref` is the data reference table of the track. The checks go in this
/// order, and the first that fails is returned.
///
/// # Errors
///
/// * [`MandatoryBoxMissing`](Error::MandatoryBoxMissing): `stbl` has no
///   time-to-sample table, sample-to-chunk table, chunk offset table or
///   sample size table this crate reads.
/// * The failures of
///   [`check_sample_to_chunk`](data_layout::check_sample_to_chunk).
/// * [`SampleCountMismatch`](Error::SampleCountMismatch): the time-to-sample
///   table, the composition offset table, the sample dependency types or the
///   chunks cover another number of samples than the `sample_count` of the
///   sample size table, ISO/IEC 14496-12 §8.7.3.2.
/// * The failures of [`check_sync_samples`](timeline::check_sync_samples).
/// * The failures of
///   [`check_sample_framings`](data_layout::check_sample_framings).
pub(super) fn samples<'a>(
    stbl: &'a SampleTableBox,
    dref: &'a DataReferenceBox,
    track_id: u32,
) -> Result<Samples<'a>, Error> {
    let missing = |box_type| Error::MandatoryBoxMissing { box_type };
    let stts = stbl
        .stts
        .as_ref()
        .ok_or(missing(TimeToSampleBox::BOX_TYPE))?;
    let stsc = stbl
        .stsc
        .as_ref()
        .ok_or(missing(SampleToChunkBox::BOX_TYPE))?;
    let (chunk_offset, chunk_count) = stbl
        .chunk_offset
        .as_ref()
        .and_then(|chunk_offset| Some((chunk_offset, data_layout::chunk_count(chunk_offset)?)))
        .ok_or(missing(ChunkOffsetBox::BOX_TYPE))?;
    let (sample_size, sample_count) = stbl
        .sample_size
        .as_ref()
        .and_then(|sample_size| Some((sample_size, data_layout::sample_count(sample_size)?)))
        .ok_or(missing(SampleSizeBox::BOX_TYPE))?;
    data_layout::check_sample_to_chunk(stsc, chunk_count)?;
    let (ctts, stss, sdtp) = (stbl.ctts.as_ref(), stbl.stss.as_ref(), stbl.sdtp.as_ref());
    let chunk_sample_count = data_layout::chunk_sample_count(stsc, chunk_count);
    for (box_type, table_sample_count) in timeline::table_sample_counts(stts, ctts, sdtp)
        .chain([(SampleToChunkBox::BOX_TYPE, chunk_sample_count)])
    {
        if table_sample_count != u64::from(sample_count) {
            return Err(Error::SampleCountMismatch {
                box_type,
                table_sample_count,
                sample_count,
            });
        }
    }
    if let Some(stss) = stss {
        timeline::check_sync_samples(stss, sample_count)?;
    }
    data_layout::check_sample_framings(
        stsc,
        chunk_offset,
        sample_size,
        sample_count,
        &stbl.stsd,
        dref,
    )?;
    Ok(Samples {
        timings: timeline::sample_timings(stts, ctts, stss, sdtp, sample_count),
        framings: data_layout::sample_framings(stsc, chunk_offset, sample_size, sample_count),
        stsd: &stbl.stsd,
        dref,
        track_id,
    })
}

/// The location of each sample of a sample table, in decoding order, as [`samples`] returns them from tables it has checked
#[derive(Debug, Clone)]
pub(super) struct Samples<'a> {
    timings: SampleTimings<'a>,
    framings: SampleFramings<'a>,
    stsd: &'a SampleDescriptionBox,
    dref: &'a DataReferenceBox,
    track_id: u32,
}

impl Samples<'_> {
    /// Returns the number of samples left to return
    pub(super) const fn sample_count(&self) -> u32 {
        self.framings.sample_count()
    }
}

impl Iterator for Samples<'_> {
    type Item = SampleLocation;

    fn next(&mut self) -> Option<SampleLocation> {
        let framing = self.framings.next()?;
        let timing = self.timings.next()?;
        // Why end rather than report the failure: `samples` refused a table
        // where this fails, and an iterator cannot return `Err`.
        let data_origin = data_layout::sample_table_data_origin(
            self.stsd,
            self.dref,
            framing.sample_description_index,
        )
        .ok()?;
        Some(SampleLocation {
            properties: SampleProperties {
                track_id: self.track_id,
                decoding_time: timing.decoding_time,
                sample_duration: timing.sample_duration,
                sample_composition_time_offset: timing.sample_composition_time_offset,
                sample_flags: timing.sample_flags,
                sample_description_index: framing.sample_description_index,
            },
            data_origin,
            offset: framing.offset,
            size: framing.size,
        })
    }
}

impl FusedIterator for Samples<'_> {}
