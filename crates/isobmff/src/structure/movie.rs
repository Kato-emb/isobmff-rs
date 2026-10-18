//! [`MovieDemuxFsm`], a movie file read through the layers [`structure`](crate::structure) holds, fragmented or not, ISO/IEC 14496-12 §8.2.1 and Annex A.8

mod demux;

pub use demux::MovieDemuxFsm;
