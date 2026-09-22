//! [`MediaSegmentDemuxer`] and [`MediaSegmentMuxer`], a media segment read off a source that seeks and written to a sink, ISO/IEC 14496-12 §8.16

mod demuxer;
mod muxer;

pub use demuxer::MediaSegmentDemuxer;
pub use muxer::MediaSegmentMuxer;
