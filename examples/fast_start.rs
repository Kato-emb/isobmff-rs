//! Writes a movie file again with its `moov` before its media data, so that a player can start before the whole file arrives
//!
//! ```text
//! cargo run --example fast_start -- movie.mp4 fast.mp4
//! ```
//!
//! The machine reads the boxes at file level, seeking past the media data
//! to the `moov`. Once the input ends, it wants each box again in the order
//! of the file laid out again, and outputs its bytes as they arrive, with the
//! `moov` laid out again in its place. One loop reads what the machine wants
//! and writes what it outputs, at the end of the output.

use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};

use isobmff::{FastStart, FastStartLimits};

/// Bytes the input is read in, at most, at a time
const PIECE_BYTES: usize = 64 * 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let usage = "usage: fast_start <input> <output>";
    let (input, output) = (
        arguments.next().ok_or(usage)?,
        arguments.next().ok_or(usage)?,
    );
    let mut input = BufReader::with_capacity(PIECE_BYTES, File::open(input)?);
    let mut output = BufWriter::new(File::create(output)?);
    let mut written = 0_u64;
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
        while let Some(laid_out) = fast_start.poll_output() {
            if laid_out.offset() != written {
                return Err(format!(
                    "output for file offset {}, not the end {written} of the output",
                    laid_out.offset()
                )
                .into());
            }
            output.write_all(laid_out.bytes())?;
            written = written.saturating_add(u64::try_from(laid_out.bytes().len())?);
        }
    }
    output.flush()?;
    println!("wrote {written} bytes");
    Ok(())
}
