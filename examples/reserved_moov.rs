//! Writes the samples of a movie file again as a movie file whose `moov` goes in space reserved before its media data, as Android's `MPEG4Writer` lays a recording out
//!
//! ```text
//! cargo run --example reserved_moov -- movie.mp4 reserved.mp4 --reserve 4096
//! cargo run --example reserved_moov -- movie.mp4 reserved.mp4 --reserve 4096 --large-mdat
//! ```
//!
//! The file is the `ftyp`, a `free` box of the bytes `--reserve` gives,
//! a 16-byte `free` the header of the `mdat` takes the place of, and the
//! media data of every sample after it, written as the samples come. As the
//! file finishes, the `moov` goes into the reserved space, a `free` box over
//! the rest of it ([`fill`]); where it does not fit, or leaves fewer than the
//! 8 bytes of a `free` header, it goes at the end of the file and the reserved
//! space stays a `free` box. Then the header of the `mdat` goes over its
//! placeholder ([`media_data_header`]): 8 bytes after a `free` of 8, or the
//! 16 of a `largesize`, which `--large-mdat` takes whatever the length.
//!
//! How much to reserve is the caller's to choose, from the bit rate and the
//! duration it expects. The layers do the rest: [`reserve`] makes the `free` boxes,
//! [`located_in_file`] gives the location of each sample where the example
//! puts its media data, [`SampleTableWriter`] declares the samples there, and
//! [`BoxWriter::write_box`] writes the boxes.
//! Each output goes where its offset says, the destination seeking back for
//! the `moov` and the header of the `mdat`.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};

use isobmff::framing::{BoxWriter, fill, media_data_header, reserve};
use isobmff::structure_data::{SampleTableWriter, located_in_file};
use isobmff::{
    BoxHeader, BoxSize, BoxType, ByteRange, DemuxEvent, Demuxer, ExtendedSize, MuxError, Output,
    Sample,
};

/// Length of the placeholder of the header of the `mdat`, as `media_data_header` takes it
const PLACEHOLDER_BYTES: u64 = 16;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let usage = "usage: reserved_moov <source> <destination> --reserve <bytes> [--large-mdat]";
    let (source, destination) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let reserved_bytes: u64 = match (arguments.next().as_deref(), arguments.next()) {
        (Some("--reserve"), Some(bytes)) => bytes.parse()?,
        _ => return Err(usage.into()),
    };
    let large_mdat = match arguments.next().as_deref() {
        None => false,
        Some("--large-mdat") => true,
        Some(_) => return Err(usage.into()),
    };

    let mut source = BufReader::new(File::open(source)?);
    let mut destination = Destination {
        file: File::create(destination)?,
        written: 0,
    };
    let mut demuxer = Demuxer::new();
    let mut file: Option<ReservedFile> = None;
    loop {
        let event = match demuxer.wanted_input() {
            Some(wanted) => {
                source.seek(SeekFrom::Start(wanted.offset()))?;
                match source.fill_buf()? {
                    [] => demuxer.finish(),
                    input => demuxer.handle_input(wanted.offset(), input),
                }
            }
            // The demuxer wants no input past a box that runs to the end of the file, which ends only once the file is finished.
            None => match demuxer.finish() {
                Ok(None) => break,
                finished => finished,
            },
        };
        let samples: Vec<Sample> = std::iter::from_fn(|| demuxer.poll_sample()).collect();
        if let Some(DemuxEvent::Movie { .. }) = event? {
            let movie = demuxer.movie().ok_or("moov without its value")?.clone();
            let file_type = demuxer.file_type().cloned().unwrap_or_default();
            let ftyp = BoxWriter::new().write_box(&file_type)?;
            destination.write(ftyp.offset(), ftyp.bytes())?;
            let reserved_offset = destination.written;
            destination.write(reserved_offset, &reserve(reserved_bytes)?)?;
            let placeholder = destination.written;
            destination.write(placeholder, &reserve(PLACEHOLDER_BYTES)?)?;
            file = Some(ReservedFile {
                tables: SampleTableWriter::new(movie)?,
                reserved_offset,
                reserved_bytes,
                placeholder,
                end: destination.written,
            });
        }
        if let Some(file) = &mut file {
            file.write_samples(&samples, &mut destination)?;
        }
    }
    file.ok_or("no moov")?
        .finish(large_mdat, &mut destination)?;
    println!("wrote {} bytes", destination.written);
    Ok(())
}

/// What the example keeps of the file it writes
struct ReservedFile {
    tables: SampleTableWriter,
    /// File offset of the `free` box that reserves space for the `moov`, and its length
    reserved_offset: u64,
    reserved_bytes: u64,
    /// File offset of the 16-byte `free` the header of the `mdat` goes over
    placeholder: u64,
    /// File offset the media data of the next sample goes at
    end: u64,
}

impl ReservedFile {
    /// Writes the media data of `samples` after that of the samples before them, and declares them in the sample tables
    fn write_samples(
        &mut self,
        samples: &[Sample],
        destination: &mut Destination,
    ) -> Result<(), Box<dyn Error>> {
        let locations = located_in_file(self.tables.movie(), samples, self.end)?;
        self.tables.handle_locations(&locations)?;
        for (sample, location) in samples.iter().zip(&locations) {
            destination.write(location.byte_range().start, sample.data())?;
        }
        self.end = locations
            .last()
            .map_or(self.end, |last| last.byte_range().end);
        Ok(())
    }

    /// Writes the `moov` into the reserved space, or at the end where it does not fit, then the header of the `mdat` over its placeholder
    fn finish(
        mut self,
        large_mdat: bool,
        destination: &mut Destination,
    ) -> Result<(), Box<dyn Error>> {
        let payload_len = self
            .end
            .saturating_sub(self.placeholder)
            .saturating_sub(PLACEHOLDER_BYTES);
        let moov = BoxWriter::new()
            .write_box(self.tables.finish())?
            .into_bytes();
        let moov_len = moov.len();
        let reserved = ByteRange::new(self.reserved_offset, Some(self.reserved_bytes));
        match fill(reserved, moov.clone()) {
            Ok(outputs) => {
                println!(
                    "moov of {moov_len} bytes in the {} reserved",
                    self.reserved_bytes
                );
                for output in &outputs {
                    destination.write(output.offset(), output.bytes())?;
                }
            }
            Err(MuxError::ReservedSpaceMismatch { .. }) => {
                println!(
                    "moov of {moov_len} bytes does not fill the {} reserved: at the end",
                    self.reserved_bytes
                );
                destination.write(self.end, &moov)?;
            }
            Err(error) => return Err(error.into()),
        }
        let header = if large_mdat {
            let size =
                ExtendedSize::new(payload_len.saturating_add(PLACEHOLDER_BYTES)).ok_or("mdat")?;
            let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size))
                .ok_or("mdat")?;
            let mut bytes = vec![0; 16];
            header.encode(&mut bytes)?;
            Output::new(self.placeholder, bytes)
        } else {
            media_data_header(self.placeholder, payload_len).ok_or("mdat too long")?
        };
        destination.write(header.offset(), header.bytes())?;
        Ok(())
    }
}

/// File the outputs are written to, and where it ends
struct Destination {
    file: File,
    written: u64,
}

impl Destination {
    /// Writes `bytes` at file offset `offset`: at the end of the file, or over what is written, seeking there and back
    fn write(&mut self, offset: u64, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        let end = offset.saturating_add(u64::try_from(bytes.len())?);
        if offset == self.written {
            self.file.write_all(bytes)?;
            self.written = end;
        } else if end <= self.written {
            self.file.seek(SeekFrom::Start(offset))?;
            self.file.write_all(bytes)?;
            self.file.seek(SeekFrom::Start(self.written))?;
        } else {
            return Err(format!(
                "output for file offset {offset}, neither the end {} of the file nor within it",
                self.written
            )
            .into());
        }
        Ok(())
    }
}
