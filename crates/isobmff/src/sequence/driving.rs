//! The reader of the framing layer, driven over a file, for the unit tests of this crate

use alloc::vec::Vec;
use core::ops::Range;

use crate::sequence::{BoxEvent, BoxReader, Error};

/// Every event a reader reports for `file`, handed over `cut_length` bytes at a time
///
/// # Errors
///
/// The failure the reader reports for the file, the events made before it
/// dropped along with it
pub(crate) fn events_of(
    file: &[u8],
    cut_length: usize,
) -> Result<Vec<(Range<u64>, BoxEvent)>, Error> {
    let mut reader = BoxReader::new();
    let mut events = Vec::new();

    for arriving in file.chunks(cut_length) {
        reader.handle_input(arriving)?;
        while let Some(event) = reader.poll_event() {
            events.push(event);
        }
    }
    reader.finish()?;
    while let Some(event) = reader.poll_event() {
        events.push(event);
    }

    Ok(events)
}
