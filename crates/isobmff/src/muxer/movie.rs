//! What the muxers share: the checks of a track, the durations of the `moov`, the data references and the `mdat` header they write, and the writing of a box into the bytes a muxer holds

use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU128;

use isobmff_boxes::{DataEntry, DataEntryUrlBox, DataReferenceBox, MovieBox};
use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader, BoxSize, BoxType, ExtendedSize};

use super::Error;
use super::TrackParameters;
use crate::structure_data::{composition_end, media_duration};

/// Sets the durations of `movie`, of each of its tracks and of their media to those the sample tables and the edit lists give, and each header to the version they need, as [`MovieParameters::to_movie_box`](super::MovieParameters::to_movie_box) says
pub(super) fn set_durations(movie: &mut MovieBox) {
    let movie_timescale = movie.mvhd.timescale;
    for trak in &mut movie.trak {
        let stbl = &trak.mdia.minf.stbl;
        let (media, composed) = (media_duration(stbl), composition_end(stbl));
        let media_timescale = trak.mdia.mdhd.timescale;
        let in_movie_timescale = |duration: u64| {
            // Why round up: the track then lasts at least as long as its
            // media, as FFmpeg's movenc rounds it.
            NonZeroU128::new(u128::from(media_timescale)).map_or(0, |timescale| {
                let scaled = u128::from(duration)
                    .saturating_mul(u128::from(movie_timescale))
                    .div_ceil(timescale.get());
                u64::try_from(scaled).unwrap_or(u64::MAX)
            })
        };
        if let Some(elst) = trak.edts.as_mut().and_then(|edts| edts.elst.as_mut()) {
            if let Some(last) = elst
                .entries
                .last_mut()
                .filter(|last| last.edit_duration == 0 && last.media_time >= 0)
            {
                last.edit_duration = in_movie_timescale(
                    composed.saturating_sub(u64::try_from(last.media_time).unwrap_or(0)),
                );
                if u32::try_from(last.edit_duration).is_err() {
                    elst.version = 1;
                }
            }
            trak.tkhd.duration = elst.entries.iter().fold(0_u64, |total, entry| {
                total.saturating_add(entry.edit_duration)
            });
        } else {
            trak.tkhd.duration = in_movie_timescale(media);
        }
        let tkhd = &mut trak.tkhd;
        tkhd.version = version_for([tkhd.duration, tkhd.creation_time, tkhd.modification_time]);
        let mdhd = &mut trak.mdia.mdhd;
        mdhd.duration = media;
        mdhd.version = version_for([mdhd.duration, mdhd.creation_time, mdhd.modification_time]);
    }
    let mvhd = &mut movie.mvhd;
    mvhd.duration = movie
        .trak
        .iter()
        .map(|trak| trak.tkhd.duration)
        .max()
        .unwrap_or(0);
    mvhd.version = version_for([mvhd.duration, mvhd.creation_time, mvhd.modification_time]);
}

/// The version of a box whose `values` take 32 bits in version 0 and 64 in version 1
fn version_for(values: [u64; 3]) -> u8 {
    u8::from(values.iter().any(|&value| u32::try_from(value).is_err()))
}

/// Checks that `track` may be added beside the tracks `added`: its `track_ID` is neither 0 nor taken, and its media data is in the same file
pub(super) fn check_track<'a>(
    mut added: impl Iterator<Item = &'a TrackParameters>,
    track: &TrackParameters,
) -> Result<(), Error> {
    let track_id = track.track_id;
    if track_id == 0 || added.any(|added| added.track_id == track_id) {
        return Err(Error::TrackId { track_id });
    }
    if let Some(data_reference_index) = track
        .stsd
        .sample_descriptions
        .iter()
        .map(|entry| entry.data_reference_index().unwrap_or(0))
        .find(|&index| index != 1)
    {
        return Err(Error::DataReferenceIndex {
            track_id,
            data_reference_index,
        });
    }
    Ok(())
}

/// The data references of every track a muxer writes: one entry, the media data in the same file
pub(super) fn data_reference() -> DataReferenceBox {
    let mut dref = DataReferenceBox::default();
    dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
    dref
}

/// Returns the header of an `mdat` of `payload_length` bytes, in the 16 bytes of a `largesize`, or `None` where the box would be longer than 64 bits count
///
/// The `media data` box, ISO/IEC 14496-12 §8.1.1, under the header of §4.2.2
/// with `size` 1: 16 bytes whatever the length, so a header written before
/// the payload is written over in place once the length is known.
pub(super) fn media_data_header(payload_length: u64) -> Option<[u8; 16]> {
    let size = ExtendedSize::new(payload_length.checked_add(16)?)?;
    let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size))?;
    let mut bytes = [0; 16];
    header.encode(&mut bytes).ok()?;
    Some(bytes)
}

/// Appends the whole box `value` forms to `out`, leaving `out` as it was where it does not write
pub(super) fn append_encoded(
    value: &(impl BoxEncode + BoxDefinition),
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let start = out.len();
    let length = usize::try_from(value.encoded_len()).map_err(|_| Error::Overflow)?;
    out.resize(start.saturating_add(length), 0);
    match value.encode(out.get_mut(start..).unwrap_or_default()) {
        Ok(_) => Ok(()),
        Err(source) => {
            out.truncate(start);
            Err(Error::Encode(source))
        }
    }
}
