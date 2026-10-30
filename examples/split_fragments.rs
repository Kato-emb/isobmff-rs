//! Splits a fragmented movie file into an initialization segment and one media segment per movie fragment
//!
//! ```text
//! cargo run --example split_fragments -- movie.mp4 segments/
//! ```
//!
//! The initialization segment, `init.mp4`, holds the `ftyp` and the `moov`
//! (ISO/IEC 14496-12 A.10), and the `mdat` of the samples the `moov` itself
//! holds, if any, at the file offset its chunk offsets point to. Each media
//! segment, `segment-<n>.m4s`, opens with a `styp` carrying the brands of the
//! `ftyp` (§8.14.2) and holds one `moof` with the `mdat` that follows it
//! (A.11). Every other box, such as a `sidx` or an `mfra`, is skipped without
//! reading its payload. Concatenating `init.mp4` with the segments in order
//! gives a file that plays as the original does.
//!
//! The reader's events go to a writer for each segment. The `ftyp`, `moov`
//! and `moof` are gathered whole; a `mdat` is streamed, its payload going to
//! the segment a piece at a time as it is read, never held whole.

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use isobmff::framing::{BoxEvent, BoxReader, BoxWriter};
use isobmff::{BoxHeader, BoxType};

/// Longest `ftyp`, `moov` or `moof` read whole
const GATHER_LIMIT: u64 = 16 * 1024 * 1024;

/// Segments written so far, and the box being copied into the last of them
struct Splitter<'a> {
    out_dir: &'a Path,
    output: BufWriter<File>,
    writer: BoxWriter,
    brands: Option<Vec<u8>>,
    segment_count: u32,
    copying: bool,
}

impl Splitter<'_> {
    /// Copies the box `event` belongs to into the segment it goes to, or skips it
    fn handle(
        &mut self,
        reader: &mut BoxReader,
        event: BoxEvent<'_>,
    ) -> Result<(), Box<dyn Error>> {
        if let BoxEvent::Header(header) = event {
            let box_type = header.box_type();
            self.copying = [b"ftyp", b"moov", b"moof", b"mdat"]
                .iter()
                .any(|copied| box_type == BoxType::compact(**copied));
            if !self.copying {
                reader.skip()?;
                return Ok(());
            }
            if box_type == BoxType::compact(*b"moof") {
                self.finish()?;
                self.segment_count = self.segment_count.saturating_add(1);
                let name = format!("segment-{}.m4s", self.segment_count);
                self.output = BufWriter::new(File::create(self.out_dir.join(name))?);
                self.writer = BoxWriter::new();
                let brands = self.brands.as_deref().ok_or("a moof before the ftyp")?;
                let segment_type = BoxHeader::with_payload_len(
                    BoxType::compact(*b"styp"),
                    u64::try_from(brands.len())?,
                )
                .ok_or("brands too long for a box")?;
                for styp in [
                    BoxEvent::Header(segment_type),
                    BoxEvent::Payload(brands),
                    BoxEvent::End,
                ] {
                    self.output.write_all(self.writer.handle_event(styp)?)?;
                }
            }
            if box_type == BoxType::compact(*b"mdat") {
                reader.stream()?;
            } else {
                reader.gather(GATHER_LIMIT)?;
            }
            self.output.write_all(self.writer.handle_event(event)?)?;
            if self.segment_count == 0
                && box_type == BoxType::compact(*b"mdat")
                && self.writer.open_box().map(|written| written.offset())
                    != reader.open_box().map(|read| read.offset())
            {
                return Err(
                    "the media data of the movie would move from where its chunk offsets point"
                        .into(),
                );
            }
            return Ok(());
        }
        if !self.copying {
            return Ok(());
        }
        if let Some(raw_box) = reader.gathered_box() {
            if raw_box.header().box_type() == BoxType::compact(*b"ftyp") {
                self.brands = Some(raw_box.payload().to_vec());
            }
            let payload = self
                .writer
                .handle_event(BoxEvent::Payload(raw_box.payload()))?;
            self.output.write_all(payload)?;
        }
        self.output.write_all(self.writer.handle_event(event)?)?;
        Ok(())
    }

    /// Ends the segment being written
    fn finish(&mut self) -> Result<(), Box<dyn Error>> {
        self.writer.finish()?;
        self.output.flush()?;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let (Some(path), Some(out_dir)) = (arguments.next(), arguments.next()) else {
        return Err("usage: split_fragments <file> <out_dir>".into());
    };
    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir)?;

    let mut file = File::open(path)?;
    let file_length = file.metadata()?.len();
    let mut reader = BoxReader::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut splitter = Splitter {
        out_dir,
        output: BufWriter::new(File::create(out_dir.join("init.mp4"))?),
        writer: BoxWriter::new(),
        brands: None,
        segment_count: 0,
        copying: false,
    };

    let (mut buffered_at, mut buffered) = (0_u64, 0_usize);

    while let Some(wanted) = reader.wanted_input() {
        let buffered_end = buffered_at.saturating_add(u64::try_from(buffered)?);
        if !(buffered_at..buffered_end).contains(&wanted.offset()) {
            file.seek(SeekFrom::Start(wanted.offset()))?;
            let at_most = wanted
                .length()
                .and_then(|length| usize::try_from(length).ok())
                .map_or(buffer.len(), |length| length.min(buffer.len()));
            buffered = file.read(buffer.get_mut(..at_most).ok_or("buffer too short")?)?;
            buffered_at = wanted.offset();
            if buffered == 0 {
                break;
            }
        }
        let input = buffer.get(..buffered).ok_or("read past the buffer")?;

        while let Some(event) = reader.handle_input(buffered_at, input)? {
            splitter.handle(&mut reader, event)?;
        }
    }

    if let Some(event) = reader.finish(file_length)? {
        splitter.handle(&mut reader, event)?;
    }
    splitter.finish()?;
    println!(
        "wrote init.mp4 and {} media segments to {}",
        splitter.segment_count,
        out_dir.display()
    );

    Ok(())
}
