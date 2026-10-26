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

mod amve;
mod ardi;
mod assp;
mod audio_element;
mod audio_element_selection;
mod box_type_argument;
mod brnd;
mod btrt;
mod cclv;
mod chnl;
mod chunk_offset;
mod cinf;
mod clap;
mod clli;
mod colr;
mod cprt;
mod csgp;
mod cslg;
mod ctts;
mod data_entry;
mod data_types;
mod dinf;
mod dmix;
mod dref;
mod edts;
mod elng;
mod elst;
mod etyp;
mod free;
mod ftyp;
mod full_box;
mod grpl;
mod hdlp;
mod hdlr;
mod idat;
mod iinf;
mod iloc;
mod imda;
mod infe;
mod ipco;
mod ipma;
mod ipro;
mod iprp;
mod iref;
mod keyd;
mod keys;
mod kind;
mod labl;
mod leva;
mod loca;
mod loudness;
mod ludt;
mod mdat;
mod mdcv;
mod mdhd;
mod mdia;
mod media_information_header;
mod mehd;
mod meta;
mod mfhd;
mod mfra;
mod mfro;
mod mime;
mod minf;
mod moof;
mod moov;
mod mvex;
mod mvhd;
mod otyp;
mod padb;
mod pasp;
mod pdin;
mod pitm;
mod prft;
mod prsl;
mod rsot;
mod saio;
mod saiz;
mod sample_entry;
mod sample_group_entry;
mod sample_size;
mod sbgp;
mod sdtp;
mod setu;
mod sgpd;
mod sidx;
mod srat;
mod ssix;
mod ssrt;
mod stbl;
mod stdp;
mod strd;
mod stri;
mod strk;
mod stsc;
mod stsd;
mod stsg;
mod stsh;
mod stss;
mod stts;
mod styp;
mod tfdt;
mod tfhd;
mod tfra;
mod tkhd;
mod traf;
mod trak;
mod tref;
mod trep;
mod trex;
mod trgr;
mod trun;
mod tsel;
mod ttyp;
mod txtc;
mod udta;
mod uri;
mod urii;
mod xml;

pub use data_entry::{
    DataEntry, DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox,
};
pub use data_types::{CompressorName, LanguageCode, Matrix, SampleFlags};
pub use dinf::DataInformationBox;
pub use dmix::{DownMixInstruction, DownMixInstructions, DownmixCoefficients};
pub use dref::DataReferenceBox;
pub use edts::EditBox;
pub use elng::ExtendedLanguageBox;
pub use elst::{EditListBox, EditListEntry};
mod subs;
mod table;

pub use amve::AmbientViewingEnvironmentBox;
pub use ardi::AudioRenderingIndicationBox;
pub use assp::{
    AlternativeStartupSequenceOffsetEntry, AlternativeStartupSequenceOffsets,
    AlternativeStartupSequencePropertiesBox,
};
pub use audio_element::{
    AudioElementBox, AudioElementDescriptionBox, AudioElementPosInteractivityPolar,
    AudioElementProminenceInteractivity,
};
pub use audio_element_selection::{AudioElementSelectionBox, AudioElementSelectionDescriptionBox};
pub use brnd::BrandProperty;
pub use btrt::BitRateBox;
pub use cclv::{ContentColourVolumeBox, ContentColourVolumePrimaries};
pub use chnl::{ChannelLayout, ChannelStructure, SpeakerPosition};
pub use chunk_offset::{ChunkLargeOffsetBox, ChunkOffsetBox, ChunkOffsetVariant};
pub use cinf::CompleteTrackInfoBox;
pub use clap::CleanApertureBox;
pub use clli::ContentLightLevelBox;
pub use colr::ColourInformationBox;
pub use cprt::CopyrightBox;
pub use csgp::{CompactSampleToGroupBox, CompactSampleToGroupPattern};
pub use cslg::CompositionToDecodeBox;
pub use ctts::{CompositionOffsetBox, CompositionOffsetEntry};
pub use etyp::{ExtendedTypeBox, TypeCombinationBox};
pub use free::{Free, FreeSpaceBox, FreeType, Skip};
pub use ftyp::FileTypeBox;
pub use grpl::{EntityToGroup, EntityToGroupBox, GroupsListBox};
pub use hdlp::HandlerProperty;
pub use hdlr::HandlerBox;
pub use idat::ItemDataBox;
pub use iinf::ItemInfoBox;
pub use iloc::{ItemLocation, ItemLocationBox, ItemLocationExtent};
pub use imda::IdentifiedMediaDataBox;
pub use infe::{FdItemInfoExtension, ItemInfoEntry, ItemInfoExtension, ItemType};
pub use ipco::{ItemProperty, ItemPropertyContainerBox};
pub use ipma::{ItemPropertyAssociation, ItemPropertyAssociationBox, ItemPropertyAssociationEntry};
pub use ipro::ItemProtectionBox;
pub use iprp::ItemPropertiesBox;
pub use iref::{ItemReference, ItemReferenceBox, SingleItemTypeReferenceBox};
pub use keyd::MetadataKeyDeclarationBox;
pub use keys::{MetadataKey, MetadataKeyBox, MetadataKeyTableBox};
pub use kind::KindBox;
pub use labl::LabelBox;
pub use leva::{Level, LevelAssignment, LevelAssignmentBox};
pub use loca::MetadataLocaleBox;
pub use loudness::{
    AlbumLoudnessInfo, LoudnessBase, LoudnessInfoType, LoudnessMeasurement, TrackLoudnessInfo,
};
pub use ludt::LoudnessBox;
pub use mdat::MediaDataBox;
pub use mdcv::MasteringDisplayColourVolumeBox;
pub use mdhd::MediaHeaderBox;
pub use mdia::MediaBox;
pub use media_information_header::{
    HintMediaHeaderBox, MediaInformationHeader, NullMediaHeaderBox, SoundMediaHeaderBox,
    SubtitleMediaHeaderBox, VideoMediaHeaderBox, VolumetricVisualMediaHeaderBox,
};
pub use mehd::MovieExtendsHeaderBox;
pub use meta::MetaBox;
pub use mfhd::MovieFragmentHeaderBox;
pub use mfra::MovieFragmentRandomAccessBox;
pub use mfro::MovieFragmentRandomAccessOffsetBox;
pub use mime::MimeBox;
pub use minf::MediaInformationBox;
pub use moof::MovieFragmentBox;
pub use moov::MovieBox;
pub use mvex::MovieExtendsBox;
pub use mvhd::MovieHeaderBox;
pub use otyp::OriginalFileTypeBox;
pub use padb::PaddingBitsBox;
pub use pasp::PixelAspectRatioBox;
pub use pdin::{ProgressiveDownloadInfoBox, ProgressiveDownloadInfoEntry};
pub use pitm::PrimaryItemBox;
pub use prft::ProducerReferenceTimeBox;
pub use prsl::PreselectionGroupBox;
pub use rsot::RedundantSampleOriginalTimingBox;
pub use saio::SampleAuxiliaryInformationOffsetsBox;
pub use saiz::{AuxInfoType, SampleAuxiliaryInformationSizesBox, SampleInfoSizes};
pub use sample_entry::{
    AudioSampleEntry, AudioSampleEntryV1, AudioSampleEntryVariant, BoxedMetadataSampleEntry,
    FontSampleEntry, HapticSampleEntry, SampleEntry, SimpleTextSampleEntry,
    TextMetaDataSampleEntry, TextSubtitleSampleEntry, UriMetaSampleEntry, VisualSampleEntry,
    VolumetricVisualSampleEntry, XmlMetaDataSampleEntry, XmlSubtitleSampleEntry,
};
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
pub use setu::MetadataSetupBox;
pub use sgpd::SampleGroupDescriptionBox;
pub use sidx::{SegmentIndexBox, SegmentIndexReference};
pub use srat::SamplingRateBox;
pub use ssix::{Subsegment, SubsegmentIndexBox, SubsegmentRange};
pub use ssrt::{SubSampleDescription, SubSampleReferenceTableBox};
pub use stbl::SampleTableBox;
pub use stdp::DegradationPriorityBox;
pub use strd::SubTrackDefinitionBox;
pub use stri::SubTrackInformationBox;
pub use strk::SubTrackBox;
pub use stsc::{SampleToChunkBox, SampleToChunkEntry};
pub use stsd::{SampleDescription, SampleDescriptionBox};
pub use stsg::SubTrackSampleGroupBox;
pub use stsh::{ShadowSyncSampleBox, ShadowSyncSampleEntry};
pub use stss::SyncSampleBox;
pub use stts::{TimeToSampleBox, TimeToSampleEntry};
pub use styp::SegmentTypeBox;
pub use subs::{
    SubSample, SubSampleInformationBox, SubSampleInformationEntry, SubSampleProperties,
};
pub use tfdt::TrackFragmentBaseMediaDecodeTimeBox;
pub use tfhd::TrackFragmentHeaderBox;
pub use tfra::{TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry};
pub use tkhd::TrackHeaderBox;
pub use traf::TrackFragmentBox;
pub use trak::TrackBox;
pub use tref::{TrackReference, TrackReferenceBox, TrackReferenceTypeBox};
pub use trep::TrackExtensionPropertiesBox;
pub use trex::TrackExtendsBox;
pub use trgr::{StereoVideoGroupBox, TrackGroup, TrackGroupBox, TrackGroupTypeBox};
pub use trun::{TrackRunBox, TrackRunSample, TrackRunSamples};
pub use tsel::TrackSelectionBox;
pub use ttyp::TrackTypeBox;
pub use txtc::TextConfigBox;
pub use udta::UserDataBox;
pub use uri::UriBox;
pub use urii::UriInitBox;
pub use xml::{BinaryXmlBox, XmlBox, XmlVariant};

/// Returns the bytes of the whole box `value` writes
#[cfg(test)]
fn encoded<Box: isobmff_core::BoxEncode + isobmff_core::BoxDefinition>(
    value: &Box,
) -> alloc::vec::Vec<u8> {
    let mut buffer = alloc::vec![0; usize::try_from(value.encoded_len()).unwrap()];
    value.encode(&mut buffer).unwrap();

    buffer
}
