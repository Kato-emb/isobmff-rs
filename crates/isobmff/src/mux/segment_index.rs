//! The subsegments an [`IndexedSegmentMuxer`](super::IndexedSegmentMuxer) writes, and the `sidx` that indexes those of a media segment

use alloc::vec;
use alloc::vec::Vec;

use isobmff_boxes::{Free, FreeSpaceBox, MovieBox, SegmentIndexBox, SegmentIndexReference};
use isobmff_core::BoxEncode;

use super::Error;
use super::output::{append_encoded, to_bytes};
use crate::{SampleProperties, timeline};

/// The widest `referenced_size`, of 31 bits
const REFERENCED_SIZE_MAX: u32 = 0x7fff_ffff;

/// A subsegment written: one movie fragment, its `moof` and its `mdat`
#[derive(Debug, Clone, Copy)]
pub(super) struct Subsegment {
    referenced_size: u32,
    /// Earliest presentation time of the reference track in the subsegment
    pub(super) earliest_presentation_time: u64,
    /// Where the presentation of the reference track in it ends
    pub(super) end: u64,
    /// Type of the stream access point it starts with, Annex I, or 0 where
    /// it starts with none
    sap_type: u8,
}

impl Subsegment {
    /// Returns the subsegment of `samples`, those of the track `track_id` of `movie` in decoding order, whose `moof` and `mdat` take `length` bytes
    ///
    /// The subsegment starts with a stream access point, ISO/IEC 14496-12
    /// Annex I, where its first sample is a sync sample, of type 1 where that
    /// sample is presented first and 2 where another is, §8.14.3.1.
    ///
    /// # Errors
    /// * [`Error::ReferencedSize`]: `length` is wider than a
    ///   `referenced_size`.
    /// * [`Error::Timeline`]: the edit list of the track does not fit the
    ///   samples.
    /// * [`Error::NoReferenceSample`]: the edit list presents none of them.
    pub(super) fn new(
        movie: &MovieBox,
        track_id: u32,
        samples: &[SampleProperties],
        length: u64,
    ) -> Result<Self, Error> {
        let referenced_size = u32::try_from(length)
            .ok()
            .filter(|size| *size <= REFERENCED_SIZE_MAX)
            .ok_or(Error::ReferencedSize { size: length })?;
        let presented = timeline::presentation(movie, track_id, samples.iter().copied())
            .map_err(|error| Error::Timeline { track_id, error })?;
        let (mut earliest, mut end, mut first) = (None, 0, None);
        for sample in presented {
            let time = sample.presentation_time;
            earliest = Some(earliest.map_or(time, |earliest: u64| earliest.min(time)));
            end = end.max(time.saturating_add(sample.duration));
            if sample.index == 0 {
                first = Some(first.map_or(time, |first: u64| first.min(time)));
            }
        }
        let earliest_presentation_time = earliest.ok_or(Error::NoReferenceSample { track_id })?;
        let sync = samples
            .first()
            .is_some_and(|sample| !sample.sample_flags.sample_is_non_sync_sample);
        let sap_type = match first {
            Some(first) if sync && first == earliest_presentation_time => 1,
            Some(_) if sync => 2,
            Some(_) | None => 0,
        };
        Ok(Self {
            referenced_size,
            earliest_presentation_time,
            end,
            sap_type,
        })
    }
}

/// Returns the length of a `sidx` of version 1 of `reference_count` references
pub(super) fn segment_index_len(reference_count: u16) -> u64 {
    let mut sidx = SegmentIndexBox::default();
    sidx.version = 1;
    sidx.references = vec![SegmentIndexReference::default(); usize::from(reference_count)];
    sidx.encoded_len()
}

/// Returns the `free` of `length` bytes, at least its header
pub(super) fn free_space(length: u64) -> FreeSpaceBox<Free> {
    let mut free = FreeSpaceBox::<Free>::default();
    let header = free.encoded_len();
    free.data = vec![0; usize::try_from(length.saturating_sub(header)).unwrap_or(0)];
    free
}

/// Returns the bytes of the `sidx` of version 1 that indexes `subsegments`, those of a media segment of the track `reference_id` of `movie`, followed by a `free` that fills the room reserved for `subsegment_count`
///
/// The last subsegment lasts up to where the presentation of the reference
/// track ends in it.
///
/// # Errors
/// * [`Error::SubsegmentDuration`]: a subsegment lasts longer than a
///   `subsegment_duration` counts.
/// * [`Error::Encode`]: the `sidx` does not write.
pub(super) fn segment_index<'a>(
    movie: &MovieBox,
    reference_id: u32,
    subsegment_count: u16,
    subsegments: impl Iterator<Item = &'a Subsegment>,
) -> Result<Vec<u8>, Error> {
    let mut subsegments = subsegments.peekable();
    let mut sidx = SegmentIndexBox::default();
    sidx.version = 1;
    sidx.reference_id = reference_id;
    sidx.timescale = movie
        .trak
        .iter()
        .find(|track| track.tkhd.track_id == reference_id)
        .map_or(0, |track| track.mdia.mdhd.timescale);
    sidx.earliest_presentation_time = subsegments
        .peek()
        .map_or(0, |subsegment| subsegment.earliest_presentation_time);
    while let Some(subsegment) = subsegments.next() {
        let next = subsegments
            .peek()
            .map_or(subsegment.end, |next| next.earliest_presentation_time);
        let duration = next.saturating_sub(subsegment.earliest_presentation_time);
        let mut reference = SegmentIndexReference::default();
        reference.referenced_size = subsegment.referenced_size;
        reference.subsegment_duration =
            u32::try_from(duration).map_err(|_| Error::SubsegmentDuration { duration })?;
        reference.starts_with_sap = subsegment.sap_type != 0;
        reference.sap_type = subsegment.sap_type;
        sidx.references.push(reference);
    }
    let reference_count = u16::try_from(sidx.references.len()).unwrap_or(u16::MAX);
    // Why a `free` after a shorter `sidx`, rather than refuse the segment:
    // each reference not written leaves 12 bytes of the room reserved, so a
    // segment of fewer subsegments leaves room for the 8-byte header of a
    // `free`, which `first_offset` steps over.
    let free_length =
        segment_index_len(subsegment_count).saturating_sub(segment_index_len(reference_count));
    sidx.first_offset = free_length;
    let mut bytes = to_bytes(&sidx)?;
    if free_length > 0 {
        append_encoded(&free_space(free_length), &mut bytes)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::{
        EditBox, EditListBox, EditListEntry, MovieBox, SampleFlags, SegmentIndexBox,
        SegmentIndexReference, TrackBox,
    };
    use isobmff_core::BoxDecode;

    use super::{Error, Subsegment, segment_index, segment_index_len};
    use crate::SampleProperties;

    fn movie(edits: &[(u64, i64)]) -> MovieBox {
        let mut trak = TrackBox::default();
        trak.tkhd.track_id = 1;
        trak.mdia.mdhd.timescale = 1_000;
        if !edits.is_empty() {
            let mut elst = EditListBox::default();
            elst.entries = edits
                .iter()
                .map(|&(edit_duration, media_time)| {
                    let mut entry = EditListEntry::default();
                    entry.edit_duration = edit_duration;
                    entry.media_time = media_time;
                    entry.media_rate_integer = 1;
                    entry
                })
                .collect();
            let mut edts = EditBox::default();
            edts.elst = Some(elst);
            trak.edts = Some(edts);
        }
        let mut moov = MovieBox::default();
        moov.mvhd.timescale = 1_000;
        moov.trak = vec![trak];
        moov
    }

    fn sample(decoding_time: u64, offset: i64, sync: bool) -> SampleProperties {
        let mut sample_flags = SampleFlags::default();
        sample_flags.sample_is_non_sync_sample = !sync;
        SampleProperties {
            track_id: 1,
            decoding_time,
            sample_duration: 100,
            sample_composition_time_offset: offset,
            sample_flags,
            sample_description_index: 1,
        }
    }

    fn reference(size: u32, duration: u32, sap: Option<u8>) -> SegmentIndexReference {
        let mut reference = SegmentIndexReference::default();
        reference.referenced_size = size;
        reference.subsegment_duration = duration;
        reference.starts_with_sap = sap.is_some();
        reference.sap_type = sap.unwrap_or(0);
        reference
    }

    #[test]
    fn a_subsegment_that_opens_with_a_sync_sample_presented_first_starts_with_a_stream_access_point_of_type_1()
     {
        let samples = [sample(0, 0, true), sample(100, 0, false)];

        let subsegment = Subsegment::new(&movie(&[]), 1, &samples, 500).unwrap();

        assert_eq!(
            (
                subsegment.earliest_presentation_time,
                subsegment.end,
                subsegment.sap_type
            ),
            (0, 200, 1)
        );
    }

    #[test]
    fn a_subsegment_whose_sync_sample_is_presented_after_another_starts_with_one_of_type_2() {
        let samples = [sample(0, 200, true), sample(100, 0, false)];

        let subsegment = Subsegment::new(&movie(&[]), 1, &samples, 500).unwrap();

        assert_eq!(
            (subsegment.earliest_presentation_time, subsegment.sap_type),
            (100, 2)
        );
    }

    #[test]
    fn a_subsegment_that_opens_with_no_sync_sample_gives_no_stream_access_point() {
        let samples = [sample(0, 0, false), sample(100, 0, true)];

        let subsegment = Subsegment::new(&movie(&[]), 1, &samples, 500).unwrap();

        assert_eq!(subsegment.sap_type, 0);
    }

    #[test]
    fn the_times_of_a_subsegment_are_those_its_edit_list_presents() {
        let samples = [sample(0, 0, true), sample(100, 0, false)];

        let subsegment =
            Subsegment::new(&movie(&[(500, -1), (1_000, 50)]), 1, &samples, 500).unwrap();

        assert_eq!(
            (subsegment.earliest_presentation_time, subsegment.end),
            (500, 650)
        );
    }

    #[test]
    fn a_subsegment_the_edit_list_presents_none_of_is_refused() {
        let samples = [sample(0, 0, true)];

        assert_eq!(
            Subsegment::new(&movie(&[(1_000, 5_000)]), 1, &samples, 500).unwrap_err(),
            Error::NoReferenceSample { track_id: 1 }
        );
    }

    #[test]
    fn a_subsegment_wider_than_a_referenced_size_is_refused() {
        let samples = [sample(0, 0, true)];

        assert_eq!(
            Subsegment::new(&movie(&[]), 1, &samples, 1 << 31).unwrap_err(),
            Error::ReferencedSize { size: 1 << 31 }
        );
    }

    #[test]
    fn each_subsegment_lasts_up_to_the_next_and_the_last_up_to_its_end() {
        let movie = movie(&[]);
        let first = Subsegment::new(&movie, 1, &[sample(0, 0, true)], 300).unwrap();
        let second = Subsegment::new(
            &movie,
            1,
            &[sample(100, 0, true), sample(200, 0, false)],
            200,
        )
        .unwrap();

        let bytes = segment_index(&movie, 1, 2, [first, second].iter()).unwrap();

        let mut expected = SegmentIndexBox::default();
        expected.version = 1;
        expected.reference_id = 1;
        expected.timescale = 1_000;
        expected.references = vec![reference(300, 100, Some(1)), reference(200, 200, Some(1))];
        assert_eq!(
            SegmentIndexBox::decode(&bytes).map(|(sidx, rest)| (sidx, rest.len())),
            Ok((expected, 0))
        );
    }

    #[test]
    fn a_segment_of_fewer_subsegments_than_reserved_fills_the_rest_with_free_space() {
        let movie = movie(&[]);
        let only = Subsegment::new(&movie, 1, &[sample(0, 0, true)], 300).unwrap();

        let bytes = segment_index(&movie, 1, 3, [only].iter()).unwrap();

        let (sidx, rest) = SegmentIndexBox::decode(&bytes).unwrap();
        assert_eq!(
            (
                u64::try_from(bytes.len()).unwrap(),
                sidx.first_offset,
                rest.get(4..8)
            ),
            (segment_index_len(3), 24, Some(b"free".as_slice()))
        );
    }
}
