//! The `moov` of a movie, built from its descriptions and the sample table of each track, and the writing of a box into the bytes a muxer holds

use alloc::vec;
use alloc::vec::Vec;
use core::num::NonZeroU128;

use isobmff_boxes::{
    DataEntry, DataEntryUrlBox, EditBox, HandlerBox, HintMediaHeaderBox, MediaInformationHeader,
    MovieBox, NullMediaHeaderBox, SampleTableBox, SoundMediaHeaderBox, SubtitleMediaHeaderBox,
    TrackBox, VideoMediaHeaderBox,
};
use isobmff_core::{BoxDefinition, BoxEncode, I8F8};

use super::{Error, MovieDescription, TrackDescription};

/// Returns the `moov` of `movie` whose tracks are `trak`, with the durations and the next `track_ID` they give
pub(super) fn movie_box(movie: &MovieDescription, trak: Vec<TrackBox>) -> MovieBox {
    let mut moov = MovieBox::default();
    moov.mvhd.timescale = movie.timescale;
    moov.mvhd.duration = trak
        .iter()
        .map(|trak| trak.tkhd.duration)
        .max()
        .unwrap_or(0);
    moov.mvhd.version = version_for(moov.mvhd.duration);
    moov.mvhd.next_track_id = trak
        .iter()
        .map(|trak| trak.tkhd.track_id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    moov.trak = trak;
    moov.other_boxes = movie.other_boxes.clone();
    moov
}

/// Returns the `trak` of the track `description` describes, whose samples `stbl` declares, its duration in `movie_timescale`
pub(super) fn track_box(
    description: &TrackDescription,
    stbl: SampleTableBox,
    movie_timescale: u32,
) -> TrackBox {
    let media_duration = stbl.stts.as_ref().map_or(0, |stts| {
        stts.entries.iter().fold(0_u64, |total, entry| {
            total.saturating_add(
                u64::from(entry.sample_count).saturating_mul(u64::from(entry.sample_delta)),
            )
        })
    });
    let duration = match &description.edit_list {
        Some(elst) => elst.entries.iter().fold(0_u64, |total, entry| {
            total.saturating_add(entry.edit_duration)
        }),
        // Why round up: the track then lasts at least as long as its media,
        // as FFmpeg's movenc rounds it.
        None => NonZeroU128::new(u128::from(description.timescale)).map_or(0, |timescale| {
            let scaled = u128::from(media_duration)
                .saturating_mul(u128::from(movie_timescale))
                .div_ceil(timescale.get());
            u64::try_from(scaled).unwrap_or(u64::MAX)
        }),
    };

    let mut trak = TrackBox::default();
    trak.tkhd.version = version_for(duration);
    trak.tkhd.track_enabled = true;
    trak.tkhd.track_in_movie = true;
    trak.tkhd.track_in_preview = true;
    trak.tkhd.track_id = description.track_id;
    trak.tkhd.duration = duration;
    trak.tkhd.volume = if description.handler_type.as_bytes() == b"soun" {
        I8F8::ONE
    } else {
        I8F8::ZERO
    };
    trak.tkhd.width = description.width;
    trak.tkhd.height = description.height;
    if description.edit_list.is_some() || !description.edit_other_boxes.as_slice().is_empty() {
        let mut edts = EditBox::default();
        edts.elst.clone_from(&description.edit_list);
        edts.other_boxes = description.edit_other_boxes.clone();
        trak.edts = Some(edts);
    }
    trak.mdia.mdhd.version = version_for(media_duration);
    trak.mdia.mdhd.timescale = description.timescale;
    trak.mdia.mdhd.duration = media_duration;
    let mut hdlr = HandlerBox::default();
    hdlr.handler_type = description.handler_type;
    trak.mdia.hdlr = hdlr;
    trak.mdia.minf.media_information_header = Some(match description.handler_type.as_bytes() {
        b"vide" => MediaInformationHeader::Video(VideoMediaHeaderBox::default()),
        b"soun" => MediaInformationHeader::Sound(SoundMediaHeaderBox::default()),
        b"hint" => MediaInformationHeader::Hint(HintMediaHeaderBox::default()),
        b"subt" => MediaInformationHeader::Subtitle(SubtitleMediaHeaderBox::default()),
        _ => MediaInformationHeader::Null(NullMediaHeaderBox::default()),
    });
    trak.mdia.minf.dinf.dref.data_entries = vec![DataEntry::Url(DataEntryUrlBox::default())];
    trak.mdia.minf.dinf.other_boxes = description.data_information_other_boxes.clone();
    trak.mdia.minf.stbl = stbl;
    trak.mdia.minf.stbl.other_boxes = description.sample_table_other_boxes.clone();
    trak.mdia.minf.other_boxes = description.media_information_other_boxes.clone();
    trak.mdia.other_boxes = description.media_other_boxes.clone();
    trak.other_boxes = description.other_boxes.clone();
    trak
}

/// Checks that `track` may be added beside the tracks `added`: its `track_ID` is neither 0 nor taken, and its media data is in the same file
pub(super) fn check_track<'a>(
    mut added: impl Iterator<Item = &'a TrackDescription>,
    track: &TrackDescription,
) -> Result<(), Error> {
    let track_id = track.track_id;
    if track_id == 0 || added.any(|added| added.track_id == track_id) {
        return Err(Error::TrackId { track_id });
    }
    if let Some(data_reference_index) = track
        .sample_entries
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

/// The version of a box that holds `value` in a field of 32 bits in version 0 and 64 in version 1
fn version_for(value: u64) -> u8 {
    u8::from(value > u64::from(u32::MAX))
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
