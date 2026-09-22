//! [`SampleDescriptions`], the `stsd` entries of a track and the resource each has its samples lie in

use isobmff_boxes::{DataEntry, DataReferenceBox, SampleDescriptionBox, SampleEntry, TrackBox};
use isobmff_core::BoxDefinition as _;

use crate::error::Error;

/// The sample descriptions of one track, and the resource each has its samples lie in
///
/// A sample is described by one entry of the `stsd` of its track (ISO/IEC
/// 14496-12 §8.5.2), counted from one, and every entry carries the
/// `data_reference_index` naming the entry of the `dref` (§8.7.2) the bytes of
/// those samples lie in. Both declaration forms name their samples' entry — a
/// sample table by the run of chunks (§8.7.4), a movie fragment by the `tfhd`
/// or the `trex` (§8.8.7) — and resolve it here to the data reference, which
/// has to be the file itself: an entry sending the reader to an external file
/// is refused.
pub(crate) struct SampleDescriptions<'track> {
    track_id: u32,
    stsd: &'track SampleDescriptionBox,
    dref: &'track DataReferenceBox,
}

impl<'track> SampleDescriptions<'track> {
    /// Reads the descriptions of `trak`
    pub(crate) fn new(trak: &'track TrackBox) -> Self {
        let minf = trak.mdia().minf();

        Self {
            track_id: trak.tkhd().track_id(),
            stsd: minf.stbl().stsd(),
            dref: minf.dinf().dref(),
        }
    }

    /// Returns the `data_reference_index` of entry `sample_description_index`, which names the file itself
    ///
    /// # Errors
    ///
    /// * [`UnknownSampleDescriptionIndex`](crate::ErrorKind::UnknownSampleDescriptionIndex):
    ///   the track has no such `stsd` entry.
    /// * The failures of [`SampleEntry::try_from`], carried on
    ///   [`Box`](crate::ErrorKind::Box): the entry does not read as a
    ///   sample entry, with `stsd` added to the containers.
    /// * [`UnknownDataReferenceIndex`](crate::ErrorKind::UnknownDataReferenceIndex):
    ///   the entry names a `dref` entry the track has none of.
    /// * [`ExternalDataReference`](crate::ErrorKind::ExternalDataReference):
    ///   the `dref` entry names a resource other than the file itself.
    pub(crate) fn data_reference_index(&self, sample_description_index: u32) -> Result<u16, Error> {
        let entry = usize::try_from(sample_description_index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| self.stsd.entries().get(index))
            .ok_or(Error::unknown_sample_description_index(
                self.track_id,
                sample_description_index,
            ))?;
        let data_reference_index = SampleEntry::try_from(entry)
            .map_err(|error| error.in_container(SampleDescriptionBox::BOX_TYPE))?
            .data_reference_index();
        let data_entry = usize::from(data_reference_index)
            .checked_sub(1)
            .and_then(|index| self.dref.entries().get(index))
            .ok_or(Error::unknown_data_reference_index(
                self.track_id,
                data_reference_index,
            ))?;

        if matches!(data_entry, DataEntry::Url(url) if url.location().is_none()) {
            Ok(data_reference_index)
        } else {
            Err(Error::external_data_reference(
                self.track_id,
                data_reference_index,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;

    use isobmff_boxes::{DataEntry, DataEntryUrnBox, DataReferenceBox};
    use isobmff_core::{AnyBox, BoxType, NullTerminatedString};
    use isobmff_test_support::{
        external_data_reference, track, track_described_by, track_reading_from,
    };

    use super::SampleDescriptions;
    use crate::error::Error;

    #[test]
    fn an_entry_naming_the_file_itself_resolves_to_its_data_reference_index() {
        let trak = track(1);

        assert_eq!(
            SampleDescriptions::new(&trak).data_reference_index(1),
            Ok(1)
        );
    }

    #[test]
    fn an_entry_the_track_has_none_of_is_refused() {
        let trak = track(1);

        assert_eq!(
            SampleDescriptions::new(&trak).data_reference_index(2),
            Err(Error::unknown_sample_description_index(1, 2))
        );
        assert_eq!(
            SampleDescriptions::new(&trak).data_reference_index(0),
            Err(Error::unknown_sample_description_index(1, 0))
        );
    }

    #[test]
    fn an_entry_ending_before_its_data_reference_index_fails_as_that_box() {
        let cut_short = AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0; 4]);
        let trak = track_described_by(1, cut_short);

        assert_eq!(
            SampleDescriptions::new(&trak).data_reference_index(1),
            Err(Error::from(
                isobmff_core::Error::truncated_payload(8, 4)
                    .in_container(BoxType::compact(*b"avc1"))
                    .in_container(BoxType::compact(*b"stsd"))
            ))
        );
    }

    #[test]
    fn a_data_reference_the_track_has_none_of_is_refused() {
        let trak = track_reading_from(1, DataReferenceBox::new(vec![]));

        assert_eq!(
            SampleDescriptions::new(&trak).data_reference_index(1),
            Err(Error::unknown_data_reference_index(1, 1))
        );
    }

    #[test]
    fn a_data_reference_to_an_external_file_is_refused() {
        let by_url = track_reading_from(1, external_data_reference());
        let by_urn = track_reading_from(
            1,
            DataReferenceBox::new(vec![DataEntry::Urn(DataEntryUrnBox::new(
                NullTerminatedString::new(String::from("media.bin")).unwrap(),
                None,
            ))]),
        );

        assert_eq!(
            SampleDescriptions::new(&by_url).data_reference_index(1),
            Err(Error::external_data_reference(1, 1))
        );
        assert_eq!(
            SampleDescriptions::new(&by_urn).data_reference_index(1),
            Err(Error::external_data_reference(1, 1))
        );
    }
}
