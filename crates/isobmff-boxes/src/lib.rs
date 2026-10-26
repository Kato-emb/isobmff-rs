//! Boxes of ISO/IEC 14496-12, each read into a value that owns its bytes
//!
//! A box here implements the traits of [`isobmff_core`]: it reads with
//! [`BoxDecode`](isobmff_core::BoxDecode), writes with
//! [`BoxEncode`](isobmff_core::BoxEncode), and names its box type with
//! [`BoxDefinition`](isobmff_core::BoxDefinition), as a box of any other crate
//! does.
//!
//! # Building a box
//!
//! The fields of a box are public and take the names its Syntax subclause
//! gives them. A box is `#[non_exhaustive]`, so a later revision of the
//! specification can add fields: it is built from its [`Default`], which holds
//! the value the specification gives every `template` field, and its fields are
//! then set one by one.
//!
//! A field the specification fixes with `=`, a `reserved` field and a
//! `pre_defined` field carry no value of their own: they are read past
//! whatever they hold, and written as the specification states them. A count
//! is the length of the table it counts, and a flag that turns a field on is
//! whether that field is there.
//!
//! # Untrusted input
//!
//! What a box allocates, and how many entries of a table it reads, are bounded
//! by the length of the payload it is read from: a count is held against the
//! payload before any entry is read. The caller sets the bound by the payload
//! it passes.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: every box owns what it was read
//! from, and a container owns the children it holds.

#![no_std]

extern crate alloc;

mod assp;
mod box_type_argument;
mod chunk_offset;
mod csgp;
mod cslg;
mod ctts;
mod data_entry;
mod data_types;
mod dinf;
mod dref;
mod edts;
mod elng;
mod elst;
mod etyp;
mod free;
mod ftyp;
mod full_box;
mod hdlr;
mod imda;
mod leva;
mod mdat;
mod mdhd;
mod mdia;
mod media_information_header;
mod mehd;
mod mfhd;
mod minf;
mod moof;
mod moov;
mod mvex;
mod mvhd;
mod otyp;
mod padb;
mod pdin;
mod saio;
mod saiz;
mod sample_group_entry;
mod sample_size;
mod sbgp;
mod sdtp;
mod sgpd;
mod ssrt;
mod stbl;
mod stdp;
mod stsc;
mod stsh;
mod stss;
mod stts;
mod styp;
mod tfdt;
mod tfhd;
mod tkhd;
mod traf;
mod trak;
mod tref;
mod trep;
mod trex;
mod trgr;
mod ttyp;

pub use data_entry::{
    DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox,
};
pub use data_types::{LanguageCode, Matrix, SampleFlags};
pub use dinf::DataInformationBox;
pub use dref::DataReferenceBox;
pub use edts::EditBox;
pub use elng::ExtendedLanguageBox;
pub use elst::{EditListBox, EditListEntry};
mod subs;
mod table;

pub use assp::{
    AlternativeStartupSequenceOffsetEntry, AlternativeStartupSequenceOffsets,
    AlternativeStartupSequencePropertiesBox,
};
pub use chunk_offset::{ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant};
pub use csgp::{CompactSampleToGroupBox, CompactSampleToGroupPattern};
pub use cslg::CompositionToDecodeBox;
pub use ctts::{CompositionOffsetBox, CompositionOffsetEntry};
pub use etyp::{ExtendedTypeBox, TypeCombinationBox};
pub use free::{Free, FreeSpaceBox, FreeType, Skip};
pub use ftyp::FileTypeBox;
pub use hdlr::HandlerBox;
pub use imda::IdentifiedMediaDataBox;
pub use leva::{Level, LevelAssignment, LevelAssignmentBox};
pub use mdat::MediaDataBox;
pub use mdhd::MediaHeaderBox;
pub use mdia::MediaBox;
pub use media_information_header::{
    HintMediaHeaderBox, MediaInformationHeader, NullMediaHeaderBox, SoundMediaHeaderBox,
    SubtitleMediaHeaderBox, VideoMediaHeaderBox, VolumetricVisualMediaHeaderBox,
};
pub use mehd::MovieExtendsHeaderBox;
pub use mfhd::MovieFragmentHeaderBox;
pub use minf::MediaInformationBox;
pub use moof::MovieFragmentBox;
pub use moov::MovieBox;
pub use mvex::MovieExtendsBox;
pub use mvhd::MovieHeaderBox;
pub use otyp::OriginalFileTypeBox;
pub use padb::PaddingBitsBox;
pub use pdin::{ProgressiveDownloadInfoBox, ProgressiveDownloadInfoEntry};
pub use saio::SampleAuxiliaryInformationOffsetsBox;
pub use saiz::{AuxInfoType, SampleAuxiliaryInformationSizesBox, SampleInfoSizes};
pub use sample_group_entry::{
    AlternativeStartupEntry, AlternativeStartupPiece, AudioPreRollEntry, CleanApertureEntry,
    EssentialDescriptionsHierarchyEntry, PixelAspectRatioEntry, RateShareEntry,
    RateShareOperationPoint, RateShareOperationPoints, RollRecoveryEntry,
    SampleGroupDescriptionEntry, SampleToMetadataItemEntry, SapEntry, T35SampleGroupEntry,
    TemporalLevelEntry, VisualDrapEntry, VisualEdrapEntry, VisualRandomAccessEntry,
};
pub use sample_size::{CompactSampleSizeBox, SampleSizeBox, SampleSizeVariant, SampleSizes};
pub use sbgp::{SampleToGroupBox, SampleToGroupEntry};
pub use sdtp::{SampleDependencyTypeBox, SampleDependencyTypeEntry};
pub use sgpd::SampleGroupDescriptionBox;
pub use ssrt::{SubSampleDescription, SubSampleReferenceTableBox};
pub use stbl::SampleTableBox;
pub use stdp::DegradationPriorityBox;
pub use stsc::{SampleToChunkBox, SampleToChunkEntry};
pub use stsh::{ShadowSyncSampleBox, ShadowSyncSampleEntry};
pub use stss::SyncSampleBox;
pub use stts::{TimeToSampleBox, TimeToSampleEntry};
pub use styp::SegmentTypeBox;
pub use subs::{
    SubSample, SubSampleInformationBox, SubSampleInformationEntry, SubSampleProperties,
};
pub use tfdt::TrackFragmentBaseMediaDecodeTimeBox;
pub use tfhd::TrackFragmentHeaderBox;
pub use tkhd::TrackHeaderBox;
pub use traf::TrackFragmentBox;
pub use trak::TrackBox;
pub use tref::{TrackReference, TrackReferenceBox, TrackReferenceTypeBox};
pub use trep::TrackExtensionPropertiesBox;
pub use trex::TrackExtendsBox;
pub use trgr::{StereoVideoGroupBox, TrackGroup, TrackGroupBox, TrackGroupTypeBox};
pub use ttyp::TrackTypeBox;

/// Returns the bytes of the whole box `value` writes
#[cfg(test)]
fn encoded<Box: isobmff_core::BoxEncode + isobmff_core::BoxDefinition>(
    value: &Box,
) -> alloc::vec::Vec<u8> {
    let mut buffer = alloc::vec![0; usize::try_from(value.encoded_len()).unwrap()];
    value.encode(&mut buffer).unwrap();

    buffer
}
