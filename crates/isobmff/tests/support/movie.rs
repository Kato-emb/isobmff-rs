//! The `moov` of a file and the samples it declares, and the packets ffprobe reads, which the tests of reading and writing sample tables share

use std::collections::BTreeMap;
use std::path::Path;

use isobmff::SampleLocation;
use isobmff::framing::{BoxReader, Event, FramerConfig};
use isobmff::structure_data::{Error, StructureDataReader};
use isobmff_boxes::MovieBox;
use isobmff_core::{BoxDecode, BoxDefinition};
use isobmff_test_support::ffprobe;
use isobmff_test_support::serde_json::{self, Value};

/// The most samples the tests resolve from one `moov`
pub(crate) const SAMPLE_COUNT_LIMIT: u64 = 1 << 20;

/// The `moov` of `file`, read with the framing reader
pub(crate) fn movie_of(file: &[u8]) -> Option<MovieBox> {
    let mut reader = BoxReader::new(FramerConfig::new(64 * 1024 * 1024));
    let mut movie = None;
    loop {
        while let Some(event) = reader.poll_event() {
            if let Event::Gathered {
                box_type, payload, ..
            } = event
                && box_type == MovieBox::BOX_TYPE
            {
                movie = Some(MovieBox::decode_payload(payload).unwrap());
            }
        }
        let Some(required) = reader.required_input() else {
            break;
        };
        match file.get(usize::try_from(required.position).unwrap()..) {
            Some(bytes) if !bytes.is_empty() => reader.handle_input(required.position, bytes),
            _ => reader.finish(),
        }
    }
    movie
}

/// The location of each sample the tracks of `movie` declare, in the order of their file offsets
pub(crate) fn locations(movie: &MovieBox) -> Result<Vec<SampleLocation>, Error> {
    StructureDataReader::new(SAMPLE_COUNT_LIMIT)
        .handle_movie(movie)
        .map(Iterator::collect)
}

/// `(offset, size, decoding_time)` of the samples of each track, by `track_ID`
pub(crate) type Samples = BTreeMap<u32, Vec<(u64, u64, u64)>>;

/// `(offset, size, decoding_time)` of the samples of each track, in decoding order, the decoding times counted from that of the first sample of the track
pub(crate) fn by_track(locations: &[SampleLocation]) -> Samples {
    let mut tracks = Samples::new();
    for location in locations {
        tracks
            .entry(location.properties.track_id)
            .or_default()
            .push((
                location.offset,
                u64::from(location.size),
                location.properties.decoding_time,
            ));
    }
    for samples in tracks.values_mut() {
        samples.sort_by_key(|&(_, _, decoding_time)| decoding_time);
        let first = samples
            .first()
            .map_or(0, |&(_, _, decoding_time)| decoding_time);
        for sample in samples.iter_mut() {
            sample.2 = sample.2.checked_sub(first).unwrap();
        }
    }
    tracks
}

/// `(pos, size, dts)` of the packets ffprobe reads from each track, ignoring edit lists, or `None` where ffprobe does not read the file
#[expect(
    clippy::indexing_slicing,
    reason = "indexing a JSON value yields null for a key it lacks, and never panics"
)]
pub(crate) fn ffprobe_by_track(path: &Path) -> Option<Samples> {
    let output = ffprobe(
        &[
            "-v",
            "error",
            "-ignore_editlist",
            "1",
            "-show_entries",
            "stream=index,id:packet=stream_index,pos,size,dts",
            "-of",
            "json",
        ],
        path,
    );
    if !output.status.success() {
        return None;
    }
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let ids = report["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stream| {
            let id = stream["id"].as_str().unwrap().trim_start_matches("0x");
            u32::from_str_radix(id, 16).unwrap()
        })
        .collect::<Vec<_>>();
    let mut tracks = BTreeMap::<u32, Vec<_>>::new();
    for packet in report["packets"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let number = |field: &str| {
            let value = &packet[field];
            value
                .as_i64()
                .or_else(|| value.as_str()?.parse().ok())
                .unwrap()
        };
        let index = usize::try_from(packet["stream_index"].as_u64().unwrap()).unwrap();
        tracks.entry(*ids.get(index).unwrap()).or_default().push((
            u64::try_from(number("pos")).unwrap(),
            u64::try_from(number("size")).unwrap(),
            number("dts"),
        ));
    }
    Some(
        tracks
            .into_iter()
            .map(|(track_id, packets)| {
                let first = packets.first().map_or(0, |&(_, _, dts)| dts);
                let counted = packets
                    .into_iter()
                    .map(|(pos, size, dts)| {
                        (
                            pos,
                            size,
                            u64::try_from(dts.checked_sub(first).unwrap()).unwrap(),
                        )
                    })
                    .collect();
                (track_id, counted)
            })
            .collect(),
    )
}
