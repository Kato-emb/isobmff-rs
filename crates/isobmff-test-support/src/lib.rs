//! Synthetic files and the machinery to drive them, shared by the tests of this workspace
//!
//! A fixture here returns the value it built rather than an `Option` of it.

// Why not std: the workspace builds every crate it holds for a bare-metal target,
// this one included, so a fixture reaching for std would fail that check.
#![no_std]

extern crate alloc;

mod boxes;
mod driving;

pub use boxes::{
    MEDIA_DATA, file_passed_on, file_type, fragmented_file, fragmented_movie, media_data_header,
    movie_fragment, segment_file, track, unfragmented_movie, written,
};
pub use driving::{bytes_of, events_of, payloads_fused, polled};
