//! Catalog of ISO/IEC 14496-12 boxes, each decoded into a value that owns its bytes
//!
//! A box in this crate implements the traits `isobmff-core` defines, so it
//! reads with [`BoxDecode`](isobmff_core::BoxDecode) and writes with
//! [`BoxEncode`](isobmff_core::BoxEncode) like any other box.
//!
//! A box is built by `new`, which states every field the spec gives no
//! template value; `with_<field>` states a field at a value other than its
//! template, and `new_<what>`, such as [`TrackBox::new_video`] or
//! [`SampleDescriptionBox::new_v1`], states the fields one variant of the box
//! varies and fills the rest.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: every box owns what it was read
//! from, and a container owns the children it holds.

#![no_std]

extern crate alloc;

use isobmff_core::{BoxDefinition, BoxType};

mod btrt;
mod chunk_offset;
mod ctts;
mod data_entry;
mod data_types;
mod dinf;
mod dref;
mod edts;
mod elst;
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
pub use data_types::{CompositionTimeOffset, HeaderDuration, SampleFlags};
pub use dinf::DataInformationBox;
pub use dref::DataReferenceBox;
pub use edts::EditBox;
pub use elst::{EditListBox, EditListEntry, MediaRate};
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
pub use sdtp::{
    IsLeading, SampleDependencyTypeBox, SampleDependencyTypeEntry, SampleDependsOn,
    SampleHasRedundancy, SampleIsDependedOn,
};
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
pub use trun::{TrackRunBox, TrackRunSample};
pub use vmhd::VideoMediaHeaderBox;

/// Types of the container boxes ISO/IEC 14496-12 defines, whose payload is
/// the boxes they hold and nothing else
///
/// ISO/IEC 14496-12 §3.1.3 defines a container box as one whose sole purpose is
/// to contain and group a set of related boxes. A box is listed when its
/// syntax declares a `Box`, not a `FullBox`, and its payload is boxes alone,
/// so a box such as `meta` (§8.11.1), whose payload starts with the fields of
/// a `FullBox`, is not. The types are listed in the order of the sections
/// that define them.
pub const CONTAINER_BOXES: &[BoxType] = &[
    MovieBox::BOX_TYPE,
    TrackBox::BOX_TYPE,
    BoxType::compact(*b"tref"),
    BoxType::compact(*b"trgr"),
    MediaBox::BOX_TYPE,
    MediaInformationBox::BOX_TYPE,
    SampleTableBox::BOX_TYPE,
    EditBox::BOX_TYPE,
    DataInformationBox::BOX_TYPE,
    MovieExtendsBox::BOX_TYPE,
    MovieFragmentBox::BOX_TYPE,
    TrackFragmentBox::BOX_TYPE,
    MovieFragmentRandomAccessBox::BOX_TYPE,
    BoxType::compact(*b"udta"),
    BoxType::compact(*b"meco"),
    BoxType::compact(*b"sinf"),
    BoxType::compact(*b"schi"),
    BoxType::compact(*b"paen"),
    BoxType::compact(*b"strk"),
    BoxType::compact(*b"strd"),
    BoxType::compact(*b"rinf"),
    BoxType::compact(*b"cinf"),
    BoxType::compact(*b"hnti"),
    BoxType::compact(*b"hinf"),
    BoxType::compact(*b"fdsa"),
    BoxType::compact(*b"ludt"),
];
