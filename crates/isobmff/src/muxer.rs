//! Writers of movie files from the samples of their tracks
//!
//! A caller describes the movie and its tracks with [`MovieDescription`] and
//! [`TrackDescription`], and hands a muxer each sample with its bytes. The
//! muxer returns the bytes of the file. [`MovieFileMuxer`] writes a movie
//! file of one `mdat` and the `moov` that declares it.

mod config;
mod description;
mod error;
mod movie;
mod movie_file;

pub use config::MuxerConfig;
pub use description::{MovieDescription, TrackDescription};
pub use error::Error;
pub use movie_file::{MediaDataHeader, MovieFileMuxer};
