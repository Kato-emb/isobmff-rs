//! Catalog of ISO/IEC 14496-12 boxes, each decoded into a value that owns its bytes
//!
//! A box in this crate implements the traits `isobmff-core` defines, so it
//! reads with [`BoxDecode`](isobmff_core::BoxDecode) and writes with
//! [`BoxEncode`](isobmff_core::BoxEncode) like any other box.
//!
//! A box is built by `new`, which states every field the spec gives no
//! template value; `with_<field>` states a field other than its template value,
//! and `new_<what>`, such as [`TrackBox::new_video`], states the fields one
//! kind of box varies and fills the rest.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: every box owns what it was read
//! from, and a container owns the children it holds.

#![no_std]

extern crate alloc;

mod btrt;
mod chunk_offset;
mod ctts;
mod data_entry;
mod data_types;
mod dinf;
mod dref;
mod ftyp;
mod hdlr;
mod hmhd;
mod mdat;
mod mdhd;
mod mdia;
mod mfhd;
mod mfra;
mod mfro;
mod minf;
mod moof;
mod moov;
mod mvex;
mod mvhd;
mod nmhd;
mod padb;
mod sample_entry;
mod sample_size;
mod sdtp;
mod sidx;
mod smhd;
mod srat;
mod stbl;
mod stdp;
mod sthd;
mod stsc;
mod stsd;
mod stss;
mod stts;
mod styp;
mod tfdt;
mod tfhd;
mod tfra;
mod tkhd;
mod traf;
mod trak;
mod trex;
mod trun;
mod vmhd;

pub use btrt::BitRateBox;
pub use chunk_offset::{
    ChunkLargeOffsetBox, ChunkLargeOffsetEntry, ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets,
};
pub use ctts::{CompositionOffsetBox, CompositionOffsetEntry};
pub use data_entry::{DataEntry, DataEntryUrlBox, DataEntryUrnBox};
pub use data_types::{CompositionTimeOffset, SampleFlags};
pub use dinf::DataInformationBox;
pub use dref::DataReferenceBox;
pub use ftyp::FileTypeBox;
pub use hdlr::HandlerBox;
pub use hmhd::HintMediaHeaderBox;
pub use mdat::MediaDataBox;
pub use mdhd::MediaHeaderBox;
pub use mdia::MediaBox;
pub use mfhd::MovieFragmentHeaderBox;
pub use mfra::MovieFragmentRandomAccessBox;
pub use mfro::MovieFragmentRandomAccessOffsetBox;
pub use minf::{MediaInformationBox, MediaInformationHeader};
pub use moof::MovieFragmentBox;
pub use moov::MovieBox;
pub use mvex::MovieExtendsBox;
pub use mvhd::MovieHeaderBox;
pub use nmhd::NullMediaHeaderBox;
pub use padb::{PaddingBitsBox, PaddingBitsEntry};
pub use sample_entry::{AudioSampleEntry, SampleEntry, VisualSampleEntry};
pub use sample_size::{
    CompactSampleSizeBox, CompactSampleSizeEntry, FieldSize, SampleSizeBox, SampleSizeEntries,
    SampleSizeEntry, SampleSizes,
};
pub use sdtp::{SampleDependencyTypeBox, SampleDependencyTypeEntry};
pub use sidx::{ReferenceType, SegmentIndexBox, SegmentIndexReference};
pub use smhd::SoundMediaHeaderBox;
pub use srat::SamplingRateBox;
pub use stbl::SampleTableBox;
pub use stdp::{DegradationPriorityBox, DegradationPriorityEntry};
pub use sthd::SubtitleMediaHeaderBox;
pub use stsc::{SampleToChunkBox, SampleToChunkEntry};
pub use stsd::SampleDescriptionBox;
pub use stss::{SyncSampleBox, SyncSampleEntry};
pub use stts::{TimeToSampleBox, TimeToSampleEntry};
pub use styp::SegmentTypeBox;
pub use tfdt::TrackFragmentBaseMediaDecodeTimeBox;
pub use tfhd::{TrackFragmentHeaderBox, TrackFragmentHeaderFlags};
pub use tfra::{TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry};
pub use tkhd::TrackHeaderBox;
pub use traf::TrackFragmentBox;
pub use trak::TrackBox;
pub use trex::TrackExtendsBox;
pub use trun::{StatedTrackRunSample, TrackRunBox, TrackRunBuilder, TrackRunSample};
pub use vmhd::VideoMediaHeaderBox;
