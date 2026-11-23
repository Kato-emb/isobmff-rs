//! How long the samples of a sample table last, checked against FFmpeg on the conformance files

#[cfg(test)]
mod tests {
    use isobmff::boxes::MovieBox;
    use isobmff::structure_data::timeline::{composition_end, media_duration};
    use isobmff::{BoxDecode, BoxDefinition, Boxes};
    use isobmff_test_support::{conformance_files, ffprobe, ffprobe_packets};

    #[test]
    #[ignore = "reads the conformance files ISOBMFF_CONFORMANCE_DIR names, and runs ffprobe"]
    fn the_media_lasts_and_is_composed_until_when_the_packets_ffprobe_reads_do() {
        let (mut tracks, mut differences) = (0, Vec::<String>::new());
        for file in conformance_files() {
            let Some(moov) = Boxes::new(&file.bytes)
                .map_while(Result::ok)
                .find(|raw_box| raw_box.header().box_type() == MovieBox::BOX_TYPE)
                .and_then(|moov| MovieBox::decode_payload(moov.payload()).ok())
            else {
                continue;
            };
            if !ffprobe(&["-v", "error"], &file.path).status.success() {
                continue;
            }
            let packets = ffprobe_packets(&["-ignore_editlist", "1"], &file.path);
            for trak in &moov.trak {
                let (track_id, stbl) = (trak.tkhd.track_id, &trak.mdia.minf.stbl);
                let declared = stbl.stts.as_ref().map_or(0, |stts| {
                    stts.entries
                        .iter()
                        .map(|entry| usize::try_from(entry.sample_count).unwrap())
                        .sum::<usize>()
                });
                let theirs = packets
                    .iter()
                    .filter(|packet| packet.track_id == track_id)
                    .take(declared)
                    .map(|packet| Some((packet.dts?, packet.pts?, packet.duration?)))
                    .collect::<Option<Vec<_>>>();
                // Why compare only a track whose every sample the time-to-sample
                // table declares ffprobe reads: the duration of a sample
                // ffprobe does not read counts in the media duration too.
                let Some(theirs) = theirs.filter(|theirs| declared > 0 && theirs.len() == declared)
                else {
                    continue;
                };
                let (Some(&(first_dts, ..)), Some(&(last_dts, _, last_duration))) =
                    (theirs.first(), theirs.last())
                else {
                    continue;
                };
                let composed = theirs
                    .iter()
                    .map(|&(_, pts, duration)| pts + duration)
                    .max()
                    .unwrap_or(0)
                    .max(0);
                let expected = (
                    u64::try_from(last_dts + last_duration - first_dts).unwrap(),
                    u64::try_from(composed).unwrap(),
                );
                let ours = (media_duration(stbl), composition_end(stbl));
                if ours != expected {
                    differences.push(format!(
                        "{} track {track_id}: {ours:?}, from ffprobe {expected:?}",
                        file.name
                    ));
                }
                // Why hold the time base against the timescale: durations
                // counted in another unit would disagree without saying why.
                let timescale = u64::from(trak.mdia.mdhd.timescale);
                if let Some(packet) = packets.iter().find(|packet| packet.track_id == track_id) {
                    let (numerator, denominator) = packet.time_base;
                    if timescale.saturating_mul(numerator) != denominator {
                        differences.push(format!(
                            "{} track {track_id}: timescale {timescale}, ffprobe time base {numerator}/{denominator}",
                            file.name
                        ));
                    }
                }
                tracks += 1;
            }
        }
        println!("compared {tracks} tracks");
        assert_eq!(differences, Vec::<String>::new());
        assert!(tracks > 0, "no track was compared");
    }
}
