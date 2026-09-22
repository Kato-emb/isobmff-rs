//! The demuxers and the muxers over `std::io`, one of each per structure
//!
//! A demuxer is created over a source that is `Read + Seek` and yields the
//! samples the file carries as `Iterator` items; a muxer is created over a
//! sink that is `Write` and takes the boxes and the samples the file is laid
//! down from. What each takes and refuses is its own contract.

mod driver;
