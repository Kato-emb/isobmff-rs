//! [`FragmentedDemuxer`] and [`FragmentedMuxer`], a fragmented movie file read off a source that seeks and written to a sink, ISO/IEC 14496-12 Annex A.8

mod demuxer;
mod muxer;

pub use demuxer::FragmentedDemuxer;
pub use muxer::FragmentedMuxer;
