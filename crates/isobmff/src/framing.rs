//! Boxes of a file framed out of bytes that arrive in pieces
//!
//! A file is a run of boxes laid end to end, each opening with a header that
//! gives its size and type. The types here find where each box starts and
//! ends as the bytes of the file arrive, and hand its payload on as the caller
//! asks: gathered whole, streamed in pieces, or skipped.

mod box_reader;

pub use box_reader::{BoxEvent, BoxEvents, BoxReader, OpenBox};
