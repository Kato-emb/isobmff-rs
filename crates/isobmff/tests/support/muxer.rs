//! What the tests of the muxers share: a file read back with the demuxer, the samples and tracks they write, and what ffprobe reads of a packet

use isobmff::SampleProperties;
use isobmff::demuxer::Event;
use isobmff::muxer::TrackDescription;
use isobmff::{Demuxer, DemuxerConfig};
use isobmff_boxes::{FileTypeBox, MovieBox, SampleFlags};
use isobmff_core::FourCc;
use isobmff_test_support::FfprobePacket;
use isobmff_test_support::fixtures::track;

/// What a demuxer reads of a file: its `ftyp`, its `moov`, and every sample of its tracks with its bytes, in file order
pub(crate) struct Read {
    pub(crate) file_type: FileTypeBox,
    pub(crate) movie: MovieBox,
    pub(crate) samples: Vec<(SampleProperties, Vec<u8>)>,
    /// Index in `samples` of the first sample read after each `moof`
    pub(crate) fragment_starts: std::collections::BTreeSet<usize>,
    pub(crate) errors: Vec<isobmff::demuxer::Error>,
}

pub(crate) fn read(file: &[u8]) -> Option<Read> {
    let mut demuxer = Demuxer::new(DemuxerConfig::default());
    let (mut file_type, mut movie) = (None, None);
    let (mut samples, mut errors) = (Vec::new(), Vec::new());
    let mut fragment_starts = std::collections::BTreeSet::new();
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                Event::FileType(ftyp) => file_type = Some(ftyp),
                Event::Movie(moov) => {
                    for trak in &moov.trak {
                        demuxer.subscribe_track(trak.tkhd.track_id);
                    }
                    movie = Some(*moov);
                }
                Event::Sample(sample) => {
                    samples.push((sample.properties, sample.bytes.to_vec()));
                }
                Event::MovieFragment(_) => {
                    fragment_starts.insert(samples.len());
                }
                Event::Error(error) => errors.push(error),
                Event::SampleLocation(_) | Event::Box { .. } | _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        match file.get(usize::try_from(required.position).unwrap()..) {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
            _ => demuxer.finish(),
        }
    }
    Some(Read {
        file_type: file_type?,
        movie: movie?,
        samples,
        fragment_starts,
        errors,
    })
}

/// The file type of an MP4 file, ISO/IEC 14496-14
pub(crate) fn mp4_file_type() -> FileTypeBox {
    let mut ftyp = FileTypeBox::default();
    ftyp.major_brand = FourCc::new(*b"isom");
    ftyp.compatible_brands = vec![FourCc::new(*b"isom")];
    ftyp
}

pub(crate) fn one_sample_entry_track(track_id: u32) -> TrackDescription {
    let stsd = track(track_id, 1, &[], &[], &[]).mdia.minf.stbl.stsd;
    TrackDescription::new(track_id, 1000, FourCc::new(*b"vide"), stsd)
}

/// A sync sample of track 1 at `decoding_time`, lasting 10
pub(crate) fn sample(decoding_time: u64) -> SampleProperties {
    SampleProperties {
        track_id: 1,
        decoding_time,
        sample_duration: 10,
        sample_composition_time_offset: 0,
        sample_flags: SampleFlags::default(),
        sample_description_index: 1,
    }
}

/// What ffprobe reads of a packet, apart from where it lies
pub(crate) fn packet_read(
    packet: &FfprobePacket,
) -> (u32, u64, Option<i64>, Option<i64>, bool, bool) {
    (
        packet.track_id,
        packet.size,
        packet.dts,
        packet.pts,
        packet.discard,
        packet.key,
    )
}
