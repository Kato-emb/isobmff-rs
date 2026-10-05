//! [`demux`], the tracks of a file and the samples its tables declare

use std::io::{self, Read};

use isobmff::boxes::{MovieBox, MovieFragmentBox};
use isobmff::core::{BoxDecode, BoxDefinition, BoxType};
use isobmff::sample::{SampleExtent, TrackDecodeTimes, movie_fragment, sample_table};
use isobmff::sequence::BoxEvent;
use isobmff::structure;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::Error;
use crate::dump::read_boxes;

/// The tracks of a file and the samples its tables declare
#[wasm_bindgen]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Demux {
    /// The tracks the movie declares, in the order it declares them
    #[wasm_bindgen(getter_with_clone)]
    pub tracks: Vec<TrackRecord>,
    /// The samples of the movie in the order its sample tables lay them in the file, then those of each fragment in the order it declares them
    #[wasm_bindgen(getter_with_clone)]
    pub samples: Vec<SampleRecord>,
}

/// One track of a movie, as its headers and its sample descriptions state it
#[wasm_bindgen(inspectable)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrackRecord {
    /// The `track_ID` of the `tkhd`
    pub track_id: u32,
    /// The `handler_type` of the `hdlr`, a fourcc such as `vide` or `soun`
    #[wasm_bindgen(getter_with_clone)]
    pub handler_type: String,
    /// The `timescale` of the `mdhd`, in ticks per second
    pub timescale: u32,
    /// The `duration` of the `mdhd` in its timescale, or none when it is indeterminate
    pub duration: Option<u64>,
    /// The type of each entry of the `stsd`, a fourcc such as `avc1` or `mp4a`
    #[wasm_bindgen(getter_with_clone)]
    pub sample_entries: Vec<String>,
}

/// One sample, what its tables state of it and where its bytes lie in the file
#[wasm_bindgen(inspectable)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SampleRecord {
    /// The track the sample belongs to
    pub track_id: u32,
    /// When the sample is decoded, in the timescale of its track
    pub decode_time: u64,
    /// How long the sample lasts, in the timescale of its track
    pub sample_duration: u32,
    /// How far after its decode time the sample is composed, in the timescale of its track
    pub sample_composition_time_offset: i64,
    /// Whether the sample is a sync sample
    pub sync: bool,
    /// The entry of the `stsd` that describes the sample, counted from 1
    pub sample_description_index: u32,
    /// The offset in the file of the first byte of the sample
    pub offset: u64,
    /// The bytes the sample spans
    pub size: u64,
}

/// Returns the tracks of the movie `source` carries and the samples its tables declare
///
/// The samples are resolved from the `moov` and each `moof`, and their bytes
/// are read past without being kept.
///
/// # Errors
///
/// The failure of `source`, the reason the box reader, a `moov` or a `moof`
/// refuses what it reads, or the reason the sample layer refuses the tables
/// they declare. A file carrying no `moov`, or none ahead of its first `moof`
/// as a media segment delivered apart from its movie does, is refused as
/// [`Unsupported`](io::ErrorKind::Unsupported).
pub(crate) fn demux<S: Read>(source: S) -> Result<Demux, Error> {
    let mut gathered: Option<(BoxType, u64, Vec<u8>)> = None;
    let mut movie: Option<MovieBox> = None;
    let mut decode_times = TrackDecodeTimes::unknown();
    let mut samples = Vec::new();
    read_boxes(source, |event, extent| {
        match event {
            BoxEvent::Header(header) => {
                let box_type = header.box_type();
                gathered = [MovieBox::BOX_TYPE, MovieFragmentBox::BOX_TYPE]
                    .contains(&box_type)
                    .then(|| (box_type, extent.start, Vec::new()));
            }
            BoxEvent::Payload(payload) => {
                if let Some((_, _, bytes)) = &mut gathered {
                    bytes.extend(payload);
                }
            }
            BoxEvent::End => match gathered.take() {
                Some((box_type, _, bytes)) if box_type == MovieBox::BOX_TYPE => {
                    let declared =
                        MovieBox::decode_payload(&bytes).map_err(structure::Error::from)?;
                    for extent in sample_table::sample_extents(
                        &declared,
                        structure::DemuxLimits::DEFAULT_RESOLVED_SAMPLES,
                    ) {
                        samples.push(record(&extent.map_err(structure::Error::from)?));
                    }
                    decode_times =
                        TrackDecodeTimes::new(&declared).map_err(structure::Error::from)?;
                    movie = Some(declared);
                }
                Some((_, moof_start, bytes)) => {
                    let declaring = movie.as_ref().ok_or_else(no_movie)?;
                    let fragment =
                        MovieFragmentBox::decode_payload(&bytes).map_err(structure::Error::from)?;
                    for extent in movie_fragment::sample_extents(
                        &fragment,
                        declaring,
                        moof_start,
                        &mut decode_times,
                        structure::DemuxLimits::DEFAULT_RESOLVED_SAMPLES,
                    )
                    .map_err(structure::Error::from)?
                    {
                        samples.push(record(&extent.map_err(structure::Error::from)?));
                    }
                }
                None => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    let movie = movie.ok_or_else(no_movie)?;
    let tracks = movie
        .trak()
        .iter()
        .map(|trak| {
            let mdia = trak.mdia();
            TrackRecord {
                track_id: trak.tkhd().track_id(),
                handler_type: mdia.hdlr().handler_type().to_string(),
                timescale: mdia.mdhd().timescale(),
                duration: mdia.mdhd().duration().get(),
                sample_entries: mdia
                    .minf()
                    .stbl()
                    .stsd()
                    .entries()
                    .iter()
                    .map(|entry| entry.box_type().to_string())
                    .collect(),
            }
        })
        .collect();

    Ok(Demux { tracks, samples })
}

/// Returns the failure of a file that carries no `moov`, or none ahead of its first `moof`
fn no_movie() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the file carries no movie (moov) ahead of its samples; a media segment on its own, without the movie it continues, is not supported",
    )
}

/// Returns the record of the sample `extent` resolves
fn record(extent: &SampleExtent) -> SampleRecord {
    let bytes = extent.extent();

    SampleRecord {
        track_id: extent.track_id(),
        decode_time: extent.decode_time(),
        sample_duration: extent.sample_duration(),
        sample_composition_time_offset: extent.sample_composition_time_offset(),
        sync: !extent.sample_flags().sample_is_non_sync_sample(),
        sample_description_index: extent.sample_description_index(),
        offset: bytes.start,
        size: bytes.end.saturating_sub(bytes.start),
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use isobmff::sample::Sample;
    use isobmff_test_support::{
        SAMPLE_CHUNKS, fragmented_file_samples, fragmented_file_with_samples,
        indexed_fragmented_file, non_fragmented_file, non_fragmented_file_samples,
        segment_file_with_samples,
    };

    use super::{Demux, SampleRecord, TrackRecord, demux};

    use crate::Error;

    fn the_track() -> Vec<TrackRecord> {
        vec![TrackRecord {
            track_id: 1,
            handler_type: "vide".to_owned(),
            timescale: 90_000,
            duration: Some(0),
            sample_entries: vec!["avc1".to_owned()],
        }]
    }

    fn placed(samples: &[Sample], offsets: &[u64]) -> Vec<SampleRecord> {
        samples
            .iter()
            .zip(offsets)
            .map(|(sample, &offset)| SampleRecord {
                track_id: sample.track_id(),
                decode_time: sample.decode_time(),
                sample_duration: sample.sample_duration(),
                sample_composition_time_offset: sample.sample_composition_time_offset(),
                sync: !sample.sample_flags().sample_is_non_sync_sample(),
                sample_description_index: sample.sample_description_index(),
                offset,
                size: u64::try_from(sample.data().len()).unwrap(),
            })
            .collect()
    }

    #[test]
    fn a_non_fragmented_file_lists_its_samples_where_its_chunks_lie() {
        let demuxed = demux(io::Cursor::new(non_fragmented_file(&SAMPLE_CHUNKS, true))).unwrap();

        assert_eq!(
            demuxed,
            Demux {
                tracks: the_track(),
                samples: placed(
                    &non_fragmented_file_samples(),
                    &[561, 569, 585, 601, 609, 617]
                ),
            }
        );
    }

    #[test]
    fn a_movie_lying_after_its_media_data_lists_the_same_samples() {
        let demuxed = demux(io::Cursor::new(non_fragmented_file(&SAMPLE_CHUNKS, false))).unwrap();

        assert_eq!(
            demuxed,
            Demux {
                tracks: the_track(),
                samples: placed(&non_fragmented_file_samples(), &[32, 40, 56, 72, 80, 88]),
            }
        );
    }

    #[test]
    fn a_fragmented_file_lists_the_samples_its_fragment_declares() {
        let demuxed = demux(io::Cursor::new(fragmented_file_with_samples())).unwrap();

        assert_eq!(
            demuxed,
            Demux {
                tracks: the_track(),
                samples: placed(&fragmented_file_samples(), &[629, 637, 645]),
            }
        );
    }

    #[test]
    fn each_fragment_lists_its_samples_past_the_indexes_around_it() {
        let file = indexed_fragmented_file();

        let demuxed = demux(io::Cursor::new(file.bytes)).unwrap();

        assert_eq!(
            demuxed,
            Demux {
                tracks: the_track(),
                samples: placed(&file.fragment_samples.concat(), &[685, 693, 701, 801, 809]),
            }
        );
    }

    #[test]
    fn a_media_segment_on_its_own_is_unsupported() {
        let refused = demux(io::Cursor::new(segment_file_with_samples())).unwrap_err();

        assert!(
            matches!(refused, Error::Io(failure) if failure.kind() == io::ErrorKind::Unsupported)
        );
    }
}
