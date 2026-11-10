//! Programs that read and write ISO base media files with `isobmff`
//!
//! [`demux`] reads a file with the demuxer, and [`remux`] writes what it read
//! again with the muxers. [`hybrid`] writes a movie from the layers of
//! `isobmff` alone, without the demuxer or a muxer: the file reads as
//! fragmented while it is being written, and once it ends a `moov` after it
//! declares every sample, so it reads as a movie file. Each has a program of
//! its own in `src/bin`.

pub mod demux;
pub mod hybrid;
pub mod remux;
