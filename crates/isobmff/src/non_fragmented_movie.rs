//! [`NonFragmentedDemuxer`] and [`NonFragmentedMuxer`], a non-fragmented movie file read off a source that seeks and written to a sink, ISO/IEC 14496-12 §8.2.1 and §8.7

mod demuxer;
mod muxer;

pub use demuxer::NonFragmentedDemuxer;
pub use muxer::NonFragmentedMuxer;
