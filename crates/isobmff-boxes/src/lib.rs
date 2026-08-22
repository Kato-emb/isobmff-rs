//! Catalog of ISO/IEC 14496-12 boxes, each decoded into a value that owns its bytes
//!
//! A box in this crate implements the traits `isobmff-core` defines, so it
//! reads with [`BoxDecode`](isobmff_core::BoxDecode) and writes with
//! [`BoxEncode`](isobmff_core::BoxEncode) like any other box.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: every box owns what it was read
//! from, and a container owns the children it holds.

#![no_std]

extern crate alloc;

mod ftyp;
mod hdlr;
mod mdat;
mod mdhd;
mod mdia;
mod mfhd;
mod minf;
mod moof;
mod moov;
mod mvex;
mod mvhd;
mod stbl;
mod stco;
mod stsc;
mod stsd;
mod stsz;
mod stts;
mod styp;
mod tfdt;
mod tfhd;
mod tkhd;
mod traf;
mod trak;
mod trex;
mod trun;

pub use ftyp::FileTypeBox;
pub use hdlr::HandlerBox;
pub use mdat::MediaDataBox;
pub use mdhd::MediaHeaderBox;
pub use mdia::MediaBox;
pub use mfhd::MovieFragmentHeaderBox;
pub use minf::MediaInformationBox;
pub use moof::MovieFragmentBox;
pub use moov::MovieBox;
pub use mvex::MovieExtendsBox;
pub use mvhd::MovieHeaderBox;
pub use stbl::SampleTableBox;
pub use stco::{ChunkOffsetBox, ChunkOffsetEntry};
pub use stsc::{SampleToChunkBox, SampleToChunkEntry};
pub use stsd::SampleDescriptionBox;
pub use stsz::{SampleSizeBox, SampleSizeEntry, SampleSizes};
pub use stts::{TimeToSampleBox, TimeToSampleEntry};
pub use styp::SegmentTypeBox;
pub use tfdt::TrackFragmentBaseMediaDecodeTimeBox;
pub use tfhd::TrackFragmentHeaderBox;
pub use tkhd::TrackHeaderBox;
pub use traf::TrackFragmentBox;
pub use trak::TrackBox;
pub use trex::TrackExtendsBox;
pub use trun::{TrackRunBox, TrackRunSample};
