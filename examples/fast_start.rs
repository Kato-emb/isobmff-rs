//! Writes a movie file again with its `moov` before its media data, so that a player can start before the whole file arrives
//!
//! ```text
//! cargo run --example fast_start -- movie.mp4 fast.mp4
//! ```
//!
//! The machine reads the boxes at file level, seeking past the media data
//! to the `moov`, and then names what to write: the `moov` laid out again,
//! and the ranges of the input to copy as they are. The input is read twice:
//! once to find the boxes, and once to copy them.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};

use isobmff::{FastStart, FastStartLimits, FastStartOutput};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let usage = "usage: fast_start <input> <output>";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let mut input = BufReader::new(File::open(input)?);
    let mut fast_start = FastStart::new(FastStartLimits {
        movie_bytes: 256 * 1024 * 1024,
        box_count: 1 << 20,
    });

    while let Some(wanted) = fast_start.wanted_input() {
        input.seek(SeekFrom::Start(wanted.offset()))?;
        match input.fill_buf()? {
            [] => fast_start.finish()?,
            bytes => fast_start.handle_input(wanted.offset(), bytes)?,
        }
    }

    let mut output = BufWriter::new(File::create(output)?);
    while let Some(laid_out) = fast_start.poll_output() {
        match laid_out {
            FastStartOutput::Bytes(bytes) => {
                println!("moov: {} bytes", bytes.len());
                output.write_all(&bytes)?;
            }
            FastStartOutput::Copy(range) => {
                input.seek(SeekFrom::Start(range.offset()))?;
                let copied = match range.length() {
                    Some(length) => io::copy(&mut (&mut input).take(length), &mut output)?,
                    None => io::copy(&mut input, &mut output)?,
                };
                println!("copied {copied} bytes from offset {}", range.offset());
            }
            _ => return Err("an output this example does not know".into()),
        }
    }
    output.flush()?;
    Ok(())
}
