//! The location of each sample a reader reports, compared with GPAC and FFmpeg on the conformance files and the fixtures

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use isobmff::boxes::{DataEntry, MovieBox, SampleSizeVariant, SampleSizes, TrackBox};
use isobmff::structure_data::data_layout::data_entry;
use isobmff::{MediaDataContainer, SampleLocation};
use isobmff_test_support::serde_json::Value;
use isobmff_test_support::{
    FfprobePacket, conformance_files, ffprobe, ffprobe_packets, gpac_list, gpac_tracks, movie_box,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// The fixtures of the `isobmff` crate, `tests/fixtures`, each with its name
fn fixtures() -> Vec<(String, PathBuf)> {
    let mut fixtures = fs::read_dir(FIXTURES)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "mp4"))
        .map(|path| (path.file_name().unwrap().to_str().unwrap().to_owned(), path))
        .collect::<Vec<_>>();
    fixtures.sort();
    fixtures
}

/// The fixtures whose tables disagree, and the refusal of the first track of each
pub(crate) const REFUSED_FIXTURES: [(&str, [u8; 4], u64); 2] = [
    ("mismatch_tables.mp4", *b"stts", 10),
    ("mismatch_chunks.mp4", *b"stsc", 24),
];

/// The number of samples GPAC reports the sample size table of each track of `report` declares, by `track_ID`
fn gpac_sample_counts(report: &Value) -> BTreeMap<u32, usize> {
    gpac_tracks(report)
        .into_iter()
        .map(|(track_id, trak)| {
            let stbl = trak.pointer("/MediaBox/MediaInformationBox/SampleTableBox");
            let sample_count = ["SampleSizeBox", "CompactSampleSizeBox"]
                .into_iter()
                .find_map(|size_box| stbl?.get(size_box)?.get("@SampleCount")?.as_str());
            (
                track_id,
                sample_count.map_or(0, |count| count.parse().unwrap()),
            )
        })
        .collect()
}

/// The container each data reference entry GPAC reports of each track of `report` specifies, in the order of the entries, by `track_ID`; `None` for an entry of a type other than `url ` and `urn `
fn gpac_containers(report: &Value) -> BTreeMap<u32, Vec<Option<MediaDataContainer>>> {
    gpac_tracks(report)
        .into_iter()
        .map(|(track_id, trak)| {
            let containers = trak
                .pointer(
                    "/MediaBox/MediaInformationBox/DataInformationBox/DataReferenceBox/URLDataEntryBox",
                )
                .map(gpac_list)
                .unwrap_or_default()
                .into_iter()
                .zip(1..)
                .map(|(entry, data_reference_index)| {
                    match (
                        entry.get("@Type").and_then(Value::as_str),
                        entry.get("@Flags").and_then(Value::as_str),
                    ) {
                        (Some("url "), Some("1")) => Some(MediaDataContainer::SameFile),
                        (Some("url " | "urn "), _) => Some(MediaDataContainer::Resource {
                            data_reference_index,
                        }),
                        _ => None,
                    }
                })
                .collect();
            (track_id, containers)
        })
        .collect()
}

/// The tracks and samples compared, and the tracks of them in the conformance files
#[derive(Debug, Default)]
struct Tally {
    tracks: usize,
    samples: usize,
    conformance_tracks: usize,
}

impl Tally {
    fn add(&mut self, is_conformance: bool, samples: usize) {
        self.tracks = self.tracks.saturating_add(1);
        self.samples = self.samples.saturating_add(samples);
        self.conformance_tracks = self
            .conformance_tracks
            .saturating_add(usize::from(is_conformance));
    }
}

/// The data reference entry the sample entry of each entry of the sample-to-chunk table of `trak` names, for the entries whose sample entry names one
pub(crate) fn chunk_data_entries(trak: &TrackBox) -> impl Iterator<Item = &DataEntry> {
    let (stbl, dref) = (&trak.mdia.minf.stbl, &trak.mdia.minf.dinf.dref);
    let entries = stbl.stsc.iter().flat_map(|stsc| &stsc.entries);
    entries
        .into_iter()
        .filter_map(|entry| data_entry(&stbl.stsd, dref, entry.sample_description_index).ok())
}

/// Compares the locations `read` reports of the samples of each track of the `moov` of each conformance file and fixture with the samples GPAC and ffprobe read, and prints what it compared and left out
///
/// `leave_out` takes the `moov` of a file, and returns why the reader does not locate its samples, for a file left out. `read` takes the bytes of a file and its `moov`, and returns the locations by `track_ID`, each track in decoding order, or why it does not.
pub(crate) fn compare_with_gpac_and_ffprobe(
    leave_out: impl Fn(&MovieBox) -> Option<&'static str>,
    mut read: impl FnMut(&[u8], &MovieBox) -> Result<BTreeMap<u32, Vec<SampleLocation>>, String>,
) {
    let mut differences = Vec::<String>::new();
    let (mut timed, mut located) = (Tally::default(), Tally::default());
    let mut contained = Tally::default();
    let mut times_only = Vec::<(String, u32, usize, usize)>::new();
    let mut left_out = Vec::<(String, Option<u32>, &str)>::new();
    let conformance = conformance_files()
        .into_iter()
        .map(|file| (file.name, file.path, file.bytes, Some(file.report)));
    let fixtures = fixtures().into_iter().map(|(name, path)| {
        let bytes = fs::read(&path).unwrap();
        (name, path, bytes, None)
    });
    for (name, path, bytes, report) in conformance.chain(fixtures) {
        // Why not compare the fixtures whose tables disagree: the
        // specification gives no value to some of their samples, and
        // `the_fixtures_whose_tables_disagree_are_refused` holds them
        // refused.
        if REFUSED_FIXTURES
            .iter()
            .any(|&(refused, ..)| refused == name)
        {
            left_out.push((name, None, "a file whose tables disagree"));
            continue;
        }
        let Some(moov) = movie_box(&bytes, &name) else {
            continue;
        };
        if let Some(reason) = leave_out(&moov) {
            left_out.push((name, None, reason));
            continue;
        }
        let ours = match read(&bytes, &moov) {
            Ok(ours) => ours,
            Err(difference) => {
                differences.push(format!("{name}: {difference}"));
                continue;
            }
        };
        let gpac = report.as_ref().map(gpac_sample_counts);
        let gpac_containers = report.as_ref().map(gpac_containers);
        let packets = ffprobe(&["-v", "error"], &path).status.success().then(|| {
            let mut packets = BTreeMap::<u32, Vec<FfprobePacket>>::new();
            for packet in ffprobe_packets(&["-ignore_editlist", "1"], &path) {
                packets.entry(packet.track_id).or_default().push(packet);
            }
            packets
        });
        for trak in &moov.trak {
            let track_id = trak.tkhd.track_id;
            let ours = ours.get(&track_id).map(Vec::as_slice).unwrap_or_default();
            let stbl = &trak.mdia.minf.stbl;
            let declared = match &stbl.sample_size {
                Some(SampleSizeVariant::SampleSize(stsz)) => match &stsz.sample_sizes {
                    SampleSizes::Constant { sample_count, .. } => {
                        usize::try_from(*sample_count).unwrap()
                    }
                    SampleSizes::Table { entry_sizes } => entry_sizes.len(),
                    _ => 0,
                },
                Some(SampleSizeVariant::CompactSampleSize(stz2)) => stz2.entry_sizes.len(),
                Some(_) | None => 0,
            };
            if ours.len() != declared {
                differences.push(format!(
                    "{name} track {track_id}: {} samples, the sample size table {declared}",
                    ours.len()
                ));
            }
            if let Some(containers) = gpac_containers
                .as_ref()
                .and_then(|containers| containers.get(&track_id))
                && !ours.is_empty()
            {
                let by_sample_entry = stbl
                    .stsd
                    .sample_descriptions
                    .iter()
                    .map(|sample_entry| {
                        let index = usize::from(sample_entry.data_reference_index()?);
                        *containers.get(index.checked_sub(1)?)?
                    })
                    .collect::<Vec<_>>();
                let theirs = ours
                    .iter()
                    .map(|location| {
                        let index = usize::try_from(location.properties.sample_description_index);
                        *by_sample_entry.get(index.ok()?.checked_sub(1)?)?
                    })
                    .collect::<Vec<_>>();
                let ours = ours
                    .iter()
                    .map(|location| Some(location.container))
                    .collect::<Vec<_>>();
                if ours != theirs {
                    differences.push(format!(
                        "{name} track {track_id}: containers {ours:?}, GPAC's data reference entries {theirs:?}"
                    ));
                }
                contained.add(report.is_some(), ours.len());
            }
            if let Some(gpac) = &gpac
                && gpac.get(&track_id) != Some(&ours.len())
            {
                differences.push(format!(
                    "{name} track {track_id}: {} samples, GPAC {:?}",
                    ours.len(),
                    gpac.get(&track_id)
                ));
            }
            if declared == 0 {
                left_out.push((name.clone(), Some(track_id), "of no sample"));
                continue;
            }
            let is_elsewhere =
                chunk_data_entries(trak).any(|data_entry| !data_entry.is_in_same_file());
            let Some(packets) = &packets else {
                left_out.push((
                    name.clone(),
                    Some(track_id),
                    "of a file ffprobe does not read",
                ));
                continue;
            };
            let theirs = packets
                .get(&track_id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            // Why let ffprobe read fewer packets of a track with samples
            // outside this file: it drops those whose bytes it cannot
            // read. Why let it read more in a fragmented file alone: those
            // are the samples of its movie fragments.
            if is_elsewhere && theirs.len() < ours.len() {
                times_only.push((name.clone(), track_id, ours.len(), theirs.len()));
            } else if theirs.len() < ours.len()
                || (moov.mvex.is_none() && theirs.len() != ours.len())
            {
                differences.push(format!(
                    "{name} track {track_id}: {} samples, ffprobe {} packets",
                    ours.len(),
                    theirs.len()
                ));
            }
            let shift = ours.first().zip(theirs.first()).and_then(|(ours, theirs)| {
                i128::from(ours.properties.decoding_time).checked_sub(i128::from(theirs.dts?))
            });
            let differing = ours.iter().zip(theirs).position(|(ours, theirs)| {
                let decoding_time = i128::from(ours.properties.decoding_time);
                let composition_time = decoding_time
                    .checked_add(i128::from(ours.properties.sample_composition_time_offset))
                    .unwrap();
                // Why not hold the positions of samples outside this
                // file: ffprobe reads them as if they were in this one.
                (!is_elsewhere && (ours.offset, u64::from(ours.size)) != (theirs.pos, theirs.size))
                    || theirs
                        .dts
                        .map(|dts| decoding_time.checked_sub(i128::from(dts)).unwrap())
                        != shift
                    || theirs.pts.map(i128::from) != Some(composition_time)
            });
            if let Some(sample) = differing {
                differences.push(format!(
                    "{name} track {track_id}: sample {sample} {:?}, ffprobe {:?}",
                    ours.get(sample),
                    theirs.get(sample)
                ));
            }
            // Why hold the time base against the timescale: a time
            // counted in another unit could agree up to the shift alone.
            let timescale = u64::from(trak.mdia.mdhd.timescale);
            if let Some(&FfprobePacket {
                time_base: (numerator, denominator),
                ..
            }) = theirs.first()
                && timescale.saturating_mul(numerator) != denominator
            {
                differences.push(format!(
                    "{name} track {track_id}: timescale {timescale}, ffprobe time base {numerator}/{denominator}"
                ));
            }
            let compared = ours.len().min(theirs.len());
            timed.add(report.is_some(), compared);
            if !is_elsewhere {
                located.add(report.is_some(), compared);
            }
        }
    }
    let mut tally = BTreeMap::<&str, usize>::new();
    for (name, track_id, bucket) in &left_out {
        println!("left out {name} track {track_id:?}: {bucket}");
        let count = tally.entry(bucket).or_default();
        *count = count.saturating_add(1);
    }
    for (name, track_id, samples, packets) in &times_only {
        println!(
            "times alone of {name} track {track_id}: {samples} samples, ffprobe {packets} packets"
        );
    }
    println!(
        "compared the times of {timed:?}, the positions of {located:?}, the containers of {contained:?}; left out {tally:?}"
    );
    assert_eq!(differences, Vec::<String>::new());
    assert!(
        located.tracks > 0 && located.samples > 0 && contained.samples > 0,
        "no track was compared"
    );
}
