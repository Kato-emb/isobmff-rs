//! Writes the AVC video of a non-fragmented MP4 file out as an Annex B byte stream
//!
//! The first track whose first sample entry is `avc1` is taken. The sequence and then the picture
//! parameter sets of that entry's decoder configuration record come first, then the NAL units of
//! every sample in turn, each behind the four-byte start code `00 00 00 01`. A fragmented file,
//! and a sample described by another sample entry than the first, are refused.
//!
//! Usage: `cargo run -p isobmff-examples --example extract_avc -- <in.mp4> <out.h264>`

use core::error::Error;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use isobmff::avc::Avc1SampleEntry;
use isobmff::structure::MovieDemuxFsm;
use isobmff::{BoxDecode, BoxDefinition};

const START_CODE: [u8; 4] = [0, 0, 0, 1];

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: extract_avc <in.mp4> <out.h264>";
    let mut arguments = env::args().skip(1);
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;

    let mut file = File::open(input)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut demux_fsm = MovieDemuxFsm::new();
    let mut handed = Ok(());
    while demux_fsm.movie().is_none() {
        handed?;
        let wanted = demux_fsm
            .wanted_input()
            .ok_or("the file carries no movie")?;
        file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = file.read(&mut buffer)?;
        handed = if read == 0 {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
        };
    }
    let movie = demux_fsm.movie().ok_or("the file carries no movie")?;
    if movie.mvex().is_some() {
        return Err("the file is fragmented".into());
    }
    let (track_id, payload) = movie
        .trak()
        .iter()
        .find_map(|track| {
            let entry = track.mdia().minf().stbl().stsd().entries().first()?;
            if entry.box_type() != Avc1SampleEntry::BOX_TYPE {
                return None;
            }
            Some((track.tkhd().track_id(), entry.raw_payload()?))
        })
        .ok_or("the file carries no track described by an avc1 entry")?;
    let entry = Avc1SampleEntry::decode_payload(payload)?;
    let record = entry.config().avc_config();
    let length_size = record.length_size_minus_one();

    let mut stream = BufWriter::new(File::create(output)?);
    for parameter_set in record
        .sequence_parameter_sets()
        .iter()
        .chain(record.picture_parameter_sets())
    {
        stream.write_all(&START_CODE)?;
        stream.write_all(parameter_set)?;
    }
    let mut count: u64 = 0;
    loop {
        while let Some(sample) = demux_fsm.poll_sample() {
            if sample.properties().track_id != track_id {
                continue;
            }
            if sample.properties().sample_description_index != 1 {
                return Err("a sample is described by another sample entry than the first".into());
            }
            for nal_unit in length_size.nal_units(sample.data()) {
                stream.write_all(&START_CODE)?;
                stream.write_all(nal_unit?)?;
            }
            count = count.saturating_add(1);
        }
        handed?;
        let Some(wanted) = demux_fsm.wanted_input() else {
            break;
        };
        file.seek(SeekFrom::Start(wanted.offset()))?;
        let read = file.read(&mut buffer)?;
        handed = if read == 0 {
            demux_fsm.finish()
        } else {
            demux_fsm.handle_input(wanted.offset(), buffer.get(..read).unwrap_or_default())
        };
    }
    stream.flush()?;
    println!("samples={count}");

    Ok(())
}
