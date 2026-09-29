//! The drivers over `std::io`
//!
//! [`DemuxDriver`] drives a demux FSM over a source that is `Read + Seek` and
//! yields the samples the file carries as `Iterator` items; a muxer is
//! created over a sink that is `Write` and takes the boxes and the samples
//! the file is laid down from. What each takes and refuses is its own
//! contract.

mod driver;
mod fragmented_movie;
mod media_segment;
mod non_fragmented_movie;

pub use driver::DemuxDriver;
pub use fragmented_movie::FragmentedMuxer;
pub use media_segment::MediaSegmentMuxer;
pub use non_fragmented_movie::NonFragmentedMuxer;
