//! Writers of movie files from the samples of their tracks
//!
//! A caller describes the movie and its tracks with [`MovieDescription`] and
//! [`TrackDescription`], and hands a muxer each sample with its bytes. The
//! muxer returns the bytes of the file. [`MovieFileMuxer`] writes a movie
//! file of one `mdat` and the `moov` that declares it,
//! [`FragmentedMovieFileMuxer`] a movie whose samples lie in movie fragments,
//! and [`SegmentMuxer`] such a movie in segments.

mod config;
mod description;
mod error;
mod fragmented_movie_file;
mod movie;
mod movie_file;

pub use config::MuxerConfig;
pub use description::{MovieDescription, TrackDescription};
pub use error::Error;
pub use fragmented_movie_file::{FragmentedMovieFileMuxer, SegmentMuxer};
pub use movie_file::{MediaDataHeader, MovieFileMuxer};
