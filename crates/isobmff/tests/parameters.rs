//! What the samples of a movie and its tracks do not give a muxer, read from a `moov` and written into one, checked against the conformance files

#[cfg(test)]
#[path = "support/muxer.rs"]
#[expect(
    dead_code,
    reason = "the samples and the packets are for the tests of the muxers"
)]
mod support;

#[cfg(test)]
mod tests {
    use isobmff::muxer::{MovieParameters, TrackParameters};
    use isobmff_boxes::{EditBox, EditListBox, EditListEntry, Matrix, SampleTableBox, TrackBox};
    use isobmff_core::{BoxDefinition, BoxEncode, FourCc, I8F8};
    use isobmff_test_support::conformance_files;
    use isobmff_test_support::fixtures::track;

    use crate::support::read;

    /// The parameters of track 1, of media in 1000 units a second, whose edits are `(edit_duration, media_time)`
    fn parameters(edits: &[(u64, i64)]) -> TrackParameters {
        let stsd = track(1, 1, &[], &[], &[]).mdia.minf.stbl.stsd;
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
        TrackParameters {
            edts: Some(edts),
            ..TrackParameters::new(1, 1000, FourCc::new(*b"vide"), stsd)
        }
    }

    /// The sample table of three samples lasting 1 each
    fn table() -> SampleTableBox {
        track(1, 1, &[4, 4, 4], &[(1, 3)], &[100]).mdia.minf.stbl
    }

    #[test]
    fn a_track_and_a_movie_written_from_their_parameters_read_back_as_them() {
        let track = TrackParameters {
            creation_time: 1 << 32,
            modification_time: 7,
            media_creation_time: 8,
            media_modification_time: 1 << 32,
            layer: -1,
            alternate_group: 2,
            volume: I8F8::ONE,
            matrix: Matrix::from_raw([0x0002_0000, 0, 0, 0, 0x0002_0000, 0, 0, 0, 0x4000_0000]),
            ..parameters(&[(500, -1), (3, 0)])
        };
        let movie = MovieParameters {
            creation_time: 1 << 32,
            modification_time: 1 << 32,
            ..MovieParameters::new(600)
        };

        let moov = movie.to_movie_box(vec![track.to_track_box(table(), 600)]);
        let trak = moov.trak.first().unwrap();
        assert_eq!(
            (
                MovieParameters::from(&moov),
                TrackParameters::from(trak),
                [moov.mvhd.version, trak.tkhd.version, trak.mdia.mdhd.version],
                (moov.mvhd.duration, moov.mvhd.next_track_id),
            ),
            (movie, track, [1, 1, 1], (503, 2))
        );
    }

    #[test]
    fn a_fragmented_movie_declares_no_sample_extends_each_track_and_keeps_an_edit_to_the_end() {
        let tracks = [
            parameters(&[(0, 1)]),
            TrackParameters::new(
                2,
                48_000,
                FourCc::new(*b"soun"),
                track(2, 1, &[], &[], &[]).mdia.minf.stbl.stsd,
            ),
        ];
        let moov = MovieParameters::new(1000).to_fragmented_movie_box(&tracks);
        let mvex = moov.mvex.as_ref().unwrap();
        assert_eq!(
            (
                mvex.trex
                    .iter()
                    .map(|trex| trex.track_id)
                    .collect::<Vec<_>>(),
                moov.trak
                    .iter()
                    .map(|trak| trak
                        .mdia
                        .minf
                        .stbl
                        .stts
                        .as_ref()
                        .map(|stts| stts.entries.len()))
                    .collect::<Vec<_>>(),
                moov.trak
                    .iter()
                    .map(TrackParameters::from)
                    .collect::<Vec<_>>(),
            ),
            (vec![1, 2], vec![Some(0), Some(0)], tracks.to_vec())
        );
    }

    // Why not the encoder of the fixtures: a box of a conformance file that
    // does not write is reported with the others, not a panic.
    /// `value` written, or `None` where it does not write
    fn encoded(value: &(impl BoxEncode + BoxDefinition)) -> Option<Vec<u8>> {
        let mut bytes = vec![0; usize::try_from(value.encoded_len()).ok()?];
        value.encode(&mut bytes).ok()?;
        Some(bytes)
    }

    /// `trak` written again from its parameters and the tables its samples declare, what the parameters do not hold taken from `trak`
    fn track_written_again(trak: &TrackBox, movie_timescale: u32) -> TrackBox {
        let mut derived = trak.mdia.minf.stbl.clone();
        derived.stsd = Default::default();
        derived.sbgp.clear();
        derived.csgp.clear();
        derived.sgpd.clear();
        derived.subs.clear();
        derived.ssrt.clear();
        derived.stsh = None;
        derived.other_boxes = Default::default();
        let mut written = TrackParameters::from(trak).to_track_box(derived, movie_timescale);
        (written.tkhd.version, written.tkhd.duration) = (trak.tkhd.version, trak.tkhd.duration);
        let mdhd = &mut written.mdia.mdhd;
        (mdhd.version, mdhd.duration) = (trak.mdia.mdhd.version, trak.mdia.mdhd.duration);
        written.mdia.minf.dinf.dref = trak.mdia.minf.dinf.dref.clone();
        if let (Some(source), Some(elst)) = (
            trak.edts.as_ref().and_then(|edts| edts.elst.as_ref()),
            written.edts.as_mut().and_then(|edts| edts.elst.as_mut()),
        ) {
            elst.version = source.version;
            if let (Some(source), Some(last)) = (source.entries.last(), elst.entries.last_mut()) {
                last.edit_duration = source.edit_duration;
            }
        }
        written
    }

    #[test]
    #[ignore = "needs ISOBMFF_CONFORMANCE_DIR"]
    fn every_moov_and_trak_of_the_conformance_files_is_written_again_from_its_parameters_as_it_was()
    {
        let (mut differences, mut tracks, mut movies) = (Vec::new(), 0, 0);
        for file in conformance_files() {
            let Some(source) = read(&file.bytes) else {
                continue;
            };
            let moov = &source.movie;
            for trak in &moov.trak {
                // Why skip these: the parameters hold no data reference, the
                // media data of a track a muxer writes being in the same file.
                let elsewhere = trak
                    .mdia
                    .minf
                    .dinf
                    .dref
                    .data_entries
                    .iter()
                    .any(|entry| !entry.is_in_same_file());
                if elsewhere {
                    continue;
                }
                tracks += 1;
                let written = track_written_again(trak, moov.mvhd.timescale);
                if encoded(&written) != encoded(trak) {
                    differences.push(format!(
                        "{} track {}: {:?}",
                        file.name,
                        trak.tkhd.track_id,
                        first_difference(&written, trak)
                    ));
                }
            }
            movies += 1;
            let mut written = MovieParameters::from(moov).to_movie_box(Vec::new());
            written.trak.clone_from(&moov.trak);
            written.mvex.clone_from(&moov.mvex);
            let (mvhd, source) = (&mut written.mvhd, &moov.mvhd);
            (mvhd.version, mvhd.duration, mvhd.next_track_id) =
                (source.version, source.duration, source.next_track_id);
            if encoded(&written) != encoded(moov) {
                differences.push(format!("{}: the moov differs", file.name));
            }
        }
        println!("{movies} moov and {tracks} trak written again");
        assert_eq!(differences, Vec::<String>::new());
        assert!(tracks > 0);
    }

    /// The first child of `written` and `source` that differs, of those that hold what the parameters do
    fn first_difference(written: &TrackBox, source: &TrackBox) -> Option<&'static str> {
        let (written_media, source_media) = (&written.mdia, &source.mdia);
        [
            ("tkhd", written.tkhd != source.tkhd),
            ("edts", written.edts != source.edts),
            ("mdhd", written_media.mdhd != source_media.mdhd),
            ("hdlr", written_media.hdlr != source_media.hdlr),
            ("minf", written_media.minf != source_media.minf),
            ("mdia", written_media != source_media),
            ("trak", written != source),
        ]
        .into_iter()
        .find(|(_, differs)| *differs)
        .map(|(name, _)| name)
    }
}
