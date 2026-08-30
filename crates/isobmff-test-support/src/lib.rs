//! Synthetic files and the machinery to drive them, shared by the tests of this workspace
//!
//! A fixture here returns the value it built rather than an `Option` of it.

// Why not std: the workspace builds every crate it holds for a bare-metal target,
// this one included, so a fixture reaching for std would fail that check.
#![no_std]
// Why not relaxing these in the manifest: a crate stating any `[lints]` table of
// its own inherits none of the workspace's, and Cargo refuses to have both, so
// relaxing three would mean restating the other 41 — and every lint the workspace
// adds afterwards would pass this crate by, unnoticed. `clippy.toml` names this
// attribute as how a crate relaxes a lint for itself.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "a fixture that will not build is a bug in the fixture, and every caller is a test that would only unwrap the report, so the panic states no contract"
)]

extern crate alloc;

mod boxes;
mod driving;

pub use boxes::{
    MEDIA_DATA, file_running_to_its_end, file_type, fragmented_file, fragmented_movie,
    media_data_header, movie_fragment, segment_file, track, unfragmented_movie, written,
};
pub use driving::{bytes_of, events_of, payloads_fused, polled};
