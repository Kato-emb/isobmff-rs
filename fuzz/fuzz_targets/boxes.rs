//! Reading and writing back every box of [`isobmff::boxes`]
//!
//! The input is the bytes of a file as they lie, so a file of boxes is a seed
//! as it is. Each box [`Boxes`] frames from it is read as every type of
//! [`BOXES`] that bears its box type, and one run checks three properties of
//! each:
//!
//! 1. reading the payload does not panic
//! 2. a box read is written back, or refused with one of the errors
//!    [`refuses_a_value`] admits: a reader keeps values the specification
//!    forbids, which a writer does not write
//! 3. a box written back reads back as the box it was written from
//!
//! The entries of a [`SampleDescriptionBox`] are also read as every class of
//! [`SAMPLE_ENTRY_CLASSES`], and each class read is held to properties 2 and
//! 3 through the conversions to and from [`SampleEntry`].

#![no_main]

use std::fmt::Debug;

use isobmff::boxes::{
    AlbumLoudnessInfo, AlternativeStartupSequencePropertiesBox, AmbientViewingEnvironmentBox,
    AudioElementBox, AudioElementDescriptionBox, AudioElementPosInteractivityPolar,
    AudioElementProminenceInteractivity, AudioElementSelectionBox,
    AudioElementSelectionDescriptionBox, AudioRenderingIndicationBox, AudioSampleEntry,
    AudioSampleEntryV1, AudioSampleEntryVariant, BinaryXmlBox, BitRateBox,
    BoxedMetadataSampleEntry, BrandProperty, ChannelLayout, ChunkLargeOffsetBox, ChunkOffsetBox,
    CleanApertureBox, ColourInformationBox, CompactSampleSizeBox, CompactSampleToGroupBox,
    CompatibleSchemeTypeBox, CompleteTrackInfoBox, CompositionOffsetBox, CompositionToDecodeBox,
    CompressedMovieBox, CompressedMovieFragmentBox, CompressedSegmentIndexBox,
    CompressedSubsegmentIndexBox, ContentColourVolumeBox, ContentLightLevelBox, CopyrightBox,
    DataEntryImdaBox, DataEntrySeqNumImdaBox, DataEntryUrlBox, DataEntryUrnBox, DataInformationBox,
    DataReferenceBox, DegradationPriorityBox, EditBox, EditListBox, ExtendedLanguageBox,
    ExtendedTypeBox, ExtraDataBox, FdHintSampleEntry, FdItemInformationBox, FdPacketBox, FdSample,
    FdSessionGroupBox, FecInformationBox, FecReservoirBox, FilePartitionBox, FileReservoirBox,
    FileTypeBox, FontSampleEntry, Free, FreeSpaceBox, GroupIdToNameBox, GroupsListBox, HandlerBox,
    HandlerProperty, HapticSampleEntry, HintBytesSent64Box, HintBytesSentBox,
    HintImmediateBytesSentBox, HintInformationBox, HintLargestPacketBox, HintLongestPacketBox,
    HintMaxRateBox, HintMaxRelativeTimeBox, HintMediaBytesSentBox, HintMediaHeaderBox,
    HintMinRelativeTimeBox, HintPacketsSent64Box, HintPacketsSentBox, HintPayloadBytesSent64Box,
    HintPayloadBytesSentBox, HintPayloadIdBox, HintRepeatedBytesSentBox, HintStatisticsBox,
    IdentifiedMediaDataBox, InitialSampleTimeBox, ItemDataBox, ItemInfoBox, ItemInfoEntry,
    ItemLocationBox, ItemPropertiesBox, ItemPropertyAssociationBox, ItemPropertyContainerBox,
    ItemProtectionBox, ItemReferenceBox, KindBox, LabelBox, LevelAssignmentBox, LoudnessBox,
    MasteringDisplayColourVolumeBox, MediaBox, MediaDataBox, MediaHeaderBox, MediaInformationBox,
    MetaBox, MetadataKeyDeclarationBox, MetadataKeyTableBox, MetadataLocaleBox, MetadataSetupBox,
    MimeBox, MovieBox, MovieExtendsBox, MovieExtendsHeaderBox, MovieFragmentBox,
    MovieFragmentHeaderBox, MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox,
    MovieHeaderBox, Mpeg2TsReceptionSampleEntry, Mpeg2TsServerSampleEntry, NullMediaHeaderBox,
    OdBox, OriginalFileTypeBox, OriginalFormatBox, PaddingBitsBox, PartitionEntry, PatBox,
    PixelAspectRatioBox, PmtBox, PreselectionGroupBox, PrimaryItemBox, ProducerReferenceTimeBox,
    ProgressiveDownloadInfoBox, ProtectedMpeg2TransportStreamSampleEntry,
    ProtectedRtpReceptionHintSampleEntry, ProtectionSchemeInfoBox, ReceivedCryptoContextIdBox,
    ReceivedCsrcBox, ReceivedRtpHintSampleEntry, ReceivedSrtpHintSampleEntry, ReceivedSsrcBox,
    RedundantSampleOriginalTimingBox, RestrictedSchemeInfoBox, RolloverCounterBox,
    RtcpReceptionHintSampleEntry, RtpHeaderExtensionTlvBox, RtpHintSampleEntry,
    RtpMovieHintInformationBox, RtpOffsetTlvBox, RtpTrackSdpHintInformationBox,
    SampleAuxiliaryInformationOffsetsBox, SampleAuxiliaryInformationSizesBox,
    SampleDependencyTypeBox, SampleDescription, SampleDescriptionBox, SampleEntry,
    SampleGroupDescriptionBox, SamplePackingInformationBox, SampleSizeBox, SampleTableBox,
    SampleToChunkBox, SampleToGroupBox, SamplingRateBox, SchemeInformationBox, SchemeTypeBox,
    ScrambleSchemeInfoBox, SegmentIndexBox, SegmentTypeBox, SequenceOffsetBox, ShadowSyncSampleBox,
    SimpleTextSampleEntry, Skip, SoundMediaHeaderBox, SrtcpReceptionHintSampleEntry,
    SrtpHintSampleEntry, SrtpProcessBox, StereoVideoBox, StereoVideoGroupBox,
    SubSampleInformationBox, SubSampleReferenceTableBox, SubTrackBox, SubTrackDefinitionBox,
    SubTrackInformationBox, SubTrackSampleGroupBox, SubsegmentIndexBox, SubtitleMediaHeaderBox,
    SyncSampleBox, TextConfigBox, TextMetaDataSampleEntry, TextSubtitleSampleEntry, TimeOffsetBox,
    TimeToSampleBox, TimescaleEntryBox, TimestampSynchronyBox, TrackBox, TrackExtendsBox,
    TrackExtensionPropertiesBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
    TrackFragmentHeaderBox, TrackFragmentRandomAccessBox, TrackGroupBox, TrackHeaderBox,
    TrackLoudnessInfo, TrackReferenceBox, TrackRunBox, TrackSelectionBox, TrackTypeBox,
    TsTimingBox, TypeCombinationBox, UriBox, UriInitBox, UriMetaSampleEntry, UserDataBox,
    VideoMediaHeaderBox, VisualSampleEntry, VolumetricVisualMediaHeaderBox,
    VolumetricVisualSampleEntry, XmlBox, XmlMetaDataSampleEntry, XmlSubtitleSampleEntry,
};
use isobmff::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Boxes, Error, ErrorKind};
use libfuzzer_sys::fuzz_target;

/// Checks a payload of one box type, as [`round_trip`] does for a type
type Check = fn(&[u8]);

/// Every box type here, each with how a payload of it is checked
const BOXES: [(BoxType, Check); 192] = [
    (AlbumLoudnessInfo::BOX_TYPE, round_trip::<AlbumLoudnessInfo>),
    (
        AlternativeStartupSequencePropertiesBox::BOX_TYPE,
        round_trip::<AlternativeStartupSequencePropertiesBox>,
    ),
    (
        AmbientViewingEnvironmentBox::BOX_TYPE,
        round_trip::<AmbientViewingEnvironmentBox>,
    ),
    (AudioElementBox::BOX_TYPE, round_trip::<AudioElementBox>),
    (
        AudioElementDescriptionBox::BOX_TYPE,
        round_trip::<AudioElementDescriptionBox>,
    ),
    (
        AudioElementPosInteractivityPolar::BOX_TYPE,
        round_trip::<AudioElementPosInteractivityPolar>,
    ),
    (
        AudioElementProminenceInteractivity::BOX_TYPE,
        round_trip::<AudioElementProminenceInteractivity>,
    ),
    (
        AudioElementSelectionBox::BOX_TYPE,
        round_trip::<AudioElementSelectionBox>,
    ),
    (
        AudioElementSelectionDescriptionBox::BOX_TYPE,
        round_trip::<AudioElementSelectionDescriptionBox>,
    ),
    (
        AudioRenderingIndicationBox::BOX_TYPE,
        round_trip::<AudioRenderingIndicationBox>,
    ),
    (BinaryXmlBox::BOX_TYPE, round_trip::<BinaryXmlBox>),
    (BitRateBox::BOX_TYPE, round_trip::<BitRateBox>),
    (BrandProperty::BOX_TYPE, round_trip::<BrandProperty>),
    (ChannelLayout::BOX_TYPE, round_trip::<ChannelLayout>),
    (
        ChunkLargeOffsetBox::BOX_TYPE,
        round_trip::<ChunkLargeOffsetBox>,
    ),
    (ChunkOffsetBox::BOX_TYPE, round_trip::<ChunkOffsetBox>),
    (CleanApertureBox::BOX_TYPE, round_trip::<CleanApertureBox>),
    (
        ColourInformationBox::BOX_TYPE,
        round_trip::<ColourInformationBox>,
    ),
    (
        CompactSampleSizeBox::BOX_TYPE,
        round_trip::<CompactSampleSizeBox>,
    ),
    (
        CompactSampleToGroupBox::BOX_TYPE,
        round_trip::<CompactSampleToGroupBox>,
    ),
    (
        CompatibleSchemeTypeBox::BOX_TYPE,
        round_trip::<CompatibleSchemeTypeBox>,
    ),
    (
        CompleteTrackInfoBox::BOX_TYPE,
        round_trip::<CompleteTrackInfoBox>,
    ),
    (
        CompositionOffsetBox::BOX_TYPE,
        round_trip::<CompositionOffsetBox>,
    ),
    (
        CompositionToDecodeBox::BOX_TYPE,
        round_trip::<CompositionToDecodeBox>,
    ),
    (
        CompressedMovieBox::BOX_TYPE,
        round_trip::<CompressedMovieBox>,
    ),
    (
        CompressedMovieFragmentBox::BOX_TYPE,
        round_trip::<CompressedMovieFragmentBox>,
    ),
    (
        CompressedSegmentIndexBox::BOX_TYPE,
        round_trip::<CompressedSegmentIndexBox>,
    ),
    (
        CompressedSubsegmentIndexBox::BOX_TYPE,
        round_trip::<CompressedSubsegmentIndexBox>,
    ),
    (
        ContentColourVolumeBox::BOX_TYPE,
        round_trip::<ContentColourVolumeBox>,
    ),
    (
        ContentLightLevelBox::BOX_TYPE,
        round_trip::<ContentLightLevelBox>,
    ),
    (CopyrightBox::BOX_TYPE, round_trip::<CopyrightBox>),
    (DataEntryImdaBox::BOX_TYPE, round_trip::<DataEntryImdaBox>),
    (
        DataEntrySeqNumImdaBox::BOX_TYPE,
        round_trip::<DataEntrySeqNumImdaBox>,
    ),
    (DataEntryUrlBox::BOX_TYPE, round_trip::<DataEntryUrlBox>),
    (DataEntryUrnBox::BOX_TYPE, round_trip::<DataEntryUrnBox>),
    (
        DataInformationBox::BOX_TYPE,
        round_trip::<DataInformationBox>,
    ),
    (DataReferenceBox::BOX_TYPE, round_trip::<DataReferenceBox>),
    (
        DegradationPriorityBox::BOX_TYPE,
        round_trip::<DegradationPriorityBox>,
    ),
    (EditBox::BOX_TYPE, round_trip::<EditBox>),
    (EditListBox::BOX_TYPE, round_trip::<EditListBox>),
    (
        ExtendedLanguageBox::BOX_TYPE,
        round_trip::<ExtendedLanguageBox>,
    ),
    (ExtendedTypeBox::BOX_TYPE, round_trip::<ExtendedTypeBox>),
    (ExtraDataBox::BOX_TYPE, round_trip::<ExtraDataBox>),
    (
        FdItemInformationBox::BOX_TYPE,
        round_trip::<FdItemInformationBox>,
    ),
    (FdPacketBox::BOX_TYPE, round_trip::<FdPacketBox>),
    (FdSample::BOX_TYPE, round_trip::<FdSample>),
    (FdSessionGroupBox::BOX_TYPE, round_trip::<FdSessionGroupBox>),
    (FecInformationBox::BOX_TYPE, round_trip::<FecInformationBox>),
    (FecReservoirBox::BOX_TYPE, round_trip::<FecReservoirBox>),
    (FilePartitionBox::BOX_TYPE, round_trip::<FilePartitionBox>),
    (FileReservoirBox::BOX_TYPE, round_trip::<FileReservoirBox>),
    (FileTypeBox::BOX_TYPE, round_trip::<FileTypeBox>),
    (
        FreeSpaceBox::<Free>::BOX_TYPE,
        round_trip::<FreeSpaceBox<Free>>,
    ),
    (
        FreeSpaceBox::<Skip>::BOX_TYPE,
        round_trip::<FreeSpaceBox<Skip>>,
    ),
    (GroupIdToNameBox::BOX_TYPE, round_trip::<GroupIdToNameBox>),
    (GroupsListBox::BOX_TYPE, round_trip::<GroupsListBox>),
    (HandlerBox::BOX_TYPE, round_trip::<HandlerBox>),
    (HandlerProperty::BOX_TYPE, round_trip::<HandlerProperty>),
    (
        HintBytesSent64Box::BOX_TYPE,
        round_trip::<HintBytesSent64Box>,
    ),
    (HintBytesSentBox::BOX_TYPE, round_trip::<HintBytesSentBox>),
    (
        HintImmediateBytesSentBox::BOX_TYPE,
        round_trip::<HintImmediateBytesSentBox>,
    ),
    (
        HintInformationBox::BOX_TYPE,
        round_trip::<HintInformationBox>,
    ),
    (
        HintLargestPacketBox::BOX_TYPE,
        round_trip::<HintLargestPacketBox>,
    ),
    (
        HintLongestPacketBox::BOX_TYPE,
        round_trip::<HintLongestPacketBox>,
    ),
    (HintMaxRateBox::BOX_TYPE, round_trip::<HintMaxRateBox>),
    (
        HintMaxRelativeTimeBox::BOX_TYPE,
        round_trip::<HintMaxRelativeTimeBox>,
    ),
    (
        HintMediaBytesSentBox::BOX_TYPE,
        round_trip::<HintMediaBytesSentBox>,
    ),
    (
        HintMediaHeaderBox::BOX_TYPE,
        round_trip::<HintMediaHeaderBox>,
    ),
    (
        HintMinRelativeTimeBox::BOX_TYPE,
        round_trip::<HintMinRelativeTimeBox>,
    ),
    (
        HintPacketsSent64Box::BOX_TYPE,
        round_trip::<HintPacketsSent64Box>,
    ),
    (
        HintPacketsSentBox::BOX_TYPE,
        round_trip::<HintPacketsSentBox>,
    ),
    (
        HintPayloadBytesSent64Box::BOX_TYPE,
        round_trip::<HintPayloadBytesSent64Box>,
    ),
    (
        HintPayloadBytesSentBox::BOX_TYPE,
        round_trip::<HintPayloadBytesSentBox>,
    ),
    (HintPayloadIdBox::BOX_TYPE, round_trip::<HintPayloadIdBox>),
    (
        HintRepeatedBytesSentBox::BOX_TYPE,
        round_trip::<HintRepeatedBytesSentBox>,
    ),
    (HintStatisticsBox::BOX_TYPE, round_trip::<HintStatisticsBox>),
    (
        IdentifiedMediaDataBox::BOX_TYPE,
        round_trip::<IdentifiedMediaDataBox>,
    ),
    (
        InitialSampleTimeBox::BOX_TYPE,
        round_trip::<InitialSampleTimeBox>,
    ),
    (ItemDataBox::BOX_TYPE, round_trip::<ItemDataBox>),
    (ItemInfoBox::BOX_TYPE, round_trip::<ItemInfoBox>),
    (ItemInfoEntry::BOX_TYPE, round_trip::<ItemInfoEntry>),
    (ItemLocationBox::BOX_TYPE, round_trip::<ItemLocationBox>),
    (ItemPropertiesBox::BOX_TYPE, round_trip::<ItemPropertiesBox>),
    (
        ItemPropertyAssociationBox::BOX_TYPE,
        round_trip::<ItemPropertyAssociationBox>,
    ),
    (
        ItemPropertyContainerBox::BOX_TYPE,
        round_trip::<ItemPropertyContainerBox>,
    ),
    (ItemProtectionBox::BOX_TYPE, round_trip::<ItemProtectionBox>),
    (ItemReferenceBox::BOX_TYPE, round_trip::<ItemReferenceBox>),
    (KindBox::BOX_TYPE, round_trip::<KindBox>),
    (LabelBox::BOX_TYPE, round_trip::<LabelBox>),
    (
        LevelAssignmentBox::BOX_TYPE,
        round_trip::<LevelAssignmentBox>,
    ),
    (LoudnessBox::BOX_TYPE, round_trip::<LoudnessBox>),
    (
        MasteringDisplayColourVolumeBox::BOX_TYPE,
        round_trip::<MasteringDisplayColourVolumeBox>,
    ),
    (MediaBox::BOX_TYPE, round_trip::<MediaBox>),
    (MediaDataBox::BOX_TYPE, round_trip::<MediaDataBox>),
    (MediaHeaderBox::BOX_TYPE, round_trip::<MediaHeaderBox>),
    (
        MediaInformationBox::BOX_TYPE,
        round_trip::<MediaInformationBox>,
    ),
    (MetaBox::BOX_TYPE, round_trip::<MetaBox>),
    (
        MetadataKeyDeclarationBox::BOX_TYPE,
        round_trip::<MetadataKeyDeclarationBox>,
    ),
    (
        MetadataKeyTableBox::BOX_TYPE,
        round_trip::<MetadataKeyTableBox>,
    ),
    (MetadataLocaleBox::BOX_TYPE, round_trip::<MetadataLocaleBox>),
    (MetadataSetupBox::BOX_TYPE, round_trip::<MetadataSetupBox>),
    (MimeBox::BOX_TYPE, round_trip::<MimeBox>),
    (MovieBox::BOX_TYPE, round_trip::<MovieBox>),
    (MovieExtendsBox::BOX_TYPE, round_trip::<MovieExtendsBox>),
    (
        MovieExtendsHeaderBox::BOX_TYPE,
        round_trip::<MovieExtendsHeaderBox>,
    ),
    (MovieFragmentBox::BOX_TYPE, round_trip::<MovieFragmentBox>),
    (
        MovieFragmentHeaderBox::BOX_TYPE,
        round_trip::<MovieFragmentHeaderBox>,
    ),
    (
        MovieFragmentRandomAccessBox::BOX_TYPE,
        round_trip::<MovieFragmentRandomAccessBox>,
    ),
    (
        MovieFragmentRandomAccessOffsetBox::BOX_TYPE,
        round_trip::<MovieFragmentRandomAccessOffsetBox>,
    ),
    (MovieHeaderBox::BOX_TYPE, round_trip::<MovieHeaderBox>),
    (
        NullMediaHeaderBox::BOX_TYPE,
        round_trip::<NullMediaHeaderBox>,
    ),
    (OdBox::BOX_TYPE, round_trip::<OdBox>),
    (
        OriginalFileTypeBox::BOX_TYPE,
        round_trip::<OriginalFileTypeBox>,
    ),
    (OriginalFormatBox::BOX_TYPE, round_trip::<OriginalFormatBox>),
    (PaddingBitsBox::BOX_TYPE, round_trip::<PaddingBitsBox>),
    (PartitionEntry::BOX_TYPE, round_trip::<PartitionEntry>),
    (PatBox::BOX_TYPE, round_trip::<PatBox>),
    (
        PixelAspectRatioBox::BOX_TYPE,
        round_trip::<PixelAspectRatioBox>,
    ),
    (PmtBox::BOX_TYPE, round_trip::<PmtBox>),
    (
        PreselectionGroupBox::BOX_TYPE,
        round_trip::<PreselectionGroupBox>,
    ),
    (PrimaryItemBox::BOX_TYPE, round_trip::<PrimaryItemBox>),
    (
        ProducerReferenceTimeBox::BOX_TYPE,
        round_trip::<ProducerReferenceTimeBox>,
    ),
    (
        ProgressiveDownloadInfoBox::BOX_TYPE,
        round_trip::<ProgressiveDownloadInfoBox>,
    ),
    (
        ProtectionSchemeInfoBox::BOX_TYPE,
        round_trip::<ProtectionSchemeInfoBox>,
    ),
    (
        ReceivedCryptoContextIdBox::BOX_TYPE,
        round_trip::<ReceivedCryptoContextIdBox>,
    ),
    (ReceivedCsrcBox::BOX_TYPE, round_trip::<ReceivedCsrcBox>),
    (ReceivedSsrcBox::BOX_TYPE, round_trip::<ReceivedSsrcBox>),
    (
        RedundantSampleOriginalTimingBox::BOX_TYPE,
        round_trip::<RedundantSampleOriginalTimingBox>,
    ),
    (
        RestrictedSchemeInfoBox::BOX_TYPE,
        round_trip::<RestrictedSchemeInfoBox>,
    ),
    (
        RolloverCounterBox::BOX_TYPE,
        round_trip::<RolloverCounterBox>,
    ),
    (
        RtpHeaderExtensionTlvBox::BOX_TYPE,
        round_trip::<RtpHeaderExtensionTlvBox>,
    ),
    (
        RtpMovieHintInformationBox::BOX_TYPE,
        round_trip::<RtpMovieHintInformationBox>,
    ),
    (RtpOffsetTlvBox::BOX_TYPE, round_trip::<RtpOffsetTlvBox>),
    (
        RtpTrackSdpHintInformationBox::BOX_TYPE,
        round_trip::<RtpTrackSdpHintInformationBox>,
    ),
    (
        SampleAuxiliaryInformationOffsetsBox::BOX_TYPE,
        round_trip::<SampleAuxiliaryInformationOffsetsBox>,
    ),
    (
        SampleAuxiliaryInformationSizesBox::BOX_TYPE,
        round_trip::<SampleAuxiliaryInformationSizesBox>,
    ),
    (
        SampleDependencyTypeBox::BOX_TYPE,
        round_trip::<SampleDependencyTypeBox>,
    ),
    (SampleDescriptionBox::BOX_TYPE, sample_descriptions),
    (
        SampleGroupDescriptionBox::BOX_TYPE,
        round_trip::<SampleGroupDescriptionBox>,
    ),
    (
        SamplePackingInformationBox::BOX_TYPE,
        round_trip::<SamplePackingInformationBox>,
    ),
    (SampleSizeBox::BOX_TYPE, round_trip::<SampleSizeBox>),
    (SampleTableBox::BOX_TYPE, round_trip::<SampleTableBox>),
    (SampleToChunkBox::BOX_TYPE, round_trip::<SampleToChunkBox>),
    (SampleToGroupBox::BOX_TYPE, round_trip::<SampleToGroupBox>),
    (SamplingRateBox::BOX_TYPE, round_trip::<SamplingRateBox>),
    (
        SchemeInformationBox::BOX_TYPE,
        round_trip::<SchemeInformationBox>,
    ),
    (SchemeTypeBox::BOX_TYPE, round_trip::<SchemeTypeBox>),
    (
        ScrambleSchemeInfoBox::BOX_TYPE,
        round_trip::<ScrambleSchemeInfoBox>,
    ),
    (SegmentIndexBox::BOX_TYPE, round_trip::<SegmentIndexBox>),
    (SegmentTypeBox::BOX_TYPE, round_trip::<SegmentTypeBox>),
    (SequenceOffsetBox::BOX_TYPE, round_trip::<SequenceOffsetBox>),
    (
        ShadowSyncSampleBox::BOX_TYPE,
        round_trip::<ShadowSyncSampleBox>,
    ),
    (
        SoundMediaHeaderBox::BOX_TYPE,
        round_trip::<SoundMediaHeaderBox>,
    ),
    (SrtpProcessBox::BOX_TYPE, round_trip::<SrtpProcessBox>),
    (StereoVideoBox::BOX_TYPE, round_trip::<StereoVideoBox>),
    (
        StereoVideoGroupBox::BOX_TYPE,
        round_trip::<StereoVideoGroupBox>,
    ),
    (
        SubSampleInformationBox::BOX_TYPE,
        round_trip::<SubSampleInformationBox>,
    ),
    (
        SubSampleReferenceTableBox::BOX_TYPE,
        round_trip::<SubSampleReferenceTableBox>,
    ),
    (
        SubsegmentIndexBox::BOX_TYPE,
        round_trip::<SubsegmentIndexBox>,
    ),
    (
        SubtitleMediaHeaderBox::BOX_TYPE,
        round_trip::<SubtitleMediaHeaderBox>,
    ),
    (SubTrackBox::BOX_TYPE, round_trip::<SubTrackBox>),
    (
        SubTrackDefinitionBox::BOX_TYPE,
        round_trip::<SubTrackDefinitionBox>,
    ),
    (
        SubTrackInformationBox::BOX_TYPE,
        round_trip::<SubTrackInformationBox>,
    ),
    (
        SubTrackSampleGroupBox::BOX_TYPE,
        round_trip::<SubTrackSampleGroupBox>,
    ),
    (SyncSampleBox::BOX_TYPE, round_trip::<SyncSampleBox>),
    (TextConfigBox::BOX_TYPE, round_trip::<TextConfigBox>),
    (TimeOffsetBox::BOX_TYPE, round_trip::<TimeOffsetBox>),
    (TimescaleEntryBox::BOX_TYPE, round_trip::<TimescaleEntryBox>),
    (
        TimestampSynchronyBox::BOX_TYPE,
        round_trip::<TimestampSynchronyBox>,
    ),
    (TimeToSampleBox::BOX_TYPE, round_trip::<TimeToSampleBox>),
    (TrackBox::BOX_TYPE, round_trip::<TrackBox>),
    (TrackExtendsBox::BOX_TYPE, round_trip::<TrackExtendsBox>),
    (
        TrackExtensionPropertiesBox::BOX_TYPE,
        round_trip::<TrackExtensionPropertiesBox>,
    ),
    (
        TrackFragmentBaseMediaDecodeTimeBox::BOX_TYPE,
        round_trip::<TrackFragmentBaseMediaDecodeTimeBox>,
    ),
    (TrackFragmentBox::BOX_TYPE, round_trip::<TrackFragmentBox>),
    (
        TrackFragmentHeaderBox::BOX_TYPE,
        round_trip::<TrackFragmentHeaderBox>,
    ),
    (
        TrackFragmentRandomAccessBox::BOX_TYPE,
        round_trip::<TrackFragmentRandomAccessBox>,
    ),
    (TrackGroupBox::BOX_TYPE, round_trip::<TrackGroupBox>),
    (TrackHeaderBox::BOX_TYPE, round_trip::<TrackHeaderBox>),
    (TrackLoudnessInfo::BOX_TYPE, round_trip::<TrackLoudnessInfo>),
    (TrackReferenceBox::BOX_TYPE, round_trip::<TrackReferenceBox>),
    (TrackRunBox::BOX_TYPE, round_trip::<TrackRunBox>),
    (TrackSelectionBox::BOX_TYPE, round_trip::<TrackSelectionBox>),
    (TrackTypeBox::BOX_TYPE, round_trip::<TrackTypeBox>),
    (TsTimingBox::BOX_TYPE, round_trip::<TsTimingBox>),
    (
        TypeCombinationBox::BOX_TYPE,
        round_trip::<TypeCombinationBox>,
    ),
    (UriBox::BOX_TYPE, round_trip::<UriBox>),
    (UriInitBox::BOX_TYPE, round_trip::<UriInitBox>),
    (UserDataBox::BOX_TYPE, round_trip::<UserDataBox>),
    (
        VideoMediaHeaderBox::BOX_TYPE,
        round_trip::<VideoMediaHeaderBox>,
    ),
    (
        VolumetricVisualMediaHeaderBox::BOX_TYPE,
        round_trip::<VolumetricVisualMediaHeaderBox>,
    ),
    (XmlBox::BOX_TYPE, round_trip::<XmlBox>),
];

/// Every class a sample entry is read as, each with how an entry of it is checked
const SAMPLE_ENTRY_CLASSES: [fn(&SampleEntry); 25] = [
    class_round_trip::<AudioSampleEntry>,
    class_round_trip::<AudioSampleEntryV1>,
    class_round_trip::<AudioSampleEntryVariant>,
    class_round_trip::<BoxedMetadataSampleEntry>,
    class_round_trip::<FdHintSampleEntry>,
    class_round_trip::<FontSampleEntry>,
    class_round_trip::<HapticSampleEntry>,
    class_round_trip::<Mpeg2TsReceptionSampleEntry>,
    class_round_trip::<Mpeg2TsServerSampleEntry>,
    class_round_trip::<ProtectedMpeg2TransportStreamSampleEntry>,
    class_round_trip::<ProtectedRtpReceptionHintSampleEntry>,
    class_round_trip::<ReceivedRtpHintSampleEntry>,
    class_round_trip::<ReceivedSrtpHintSampleEntry>,
    class_round_trip::<RtcpReceptionHintSampleEntry>,
    class_round_trip::<RtpHintSampleEntry>,
    class_round_trip::<SimpleTextSampleEntry>,
    class_round_trip::<SrtcpReceptionHintSampleEntry>,
    class_round_trip::<SrtpHintSampleEntry>,
    class_round_trip::<TextMetaDataSampleEntry>,
    class_round_trip::<TextSubtitleSampleEntry>,
    class_round_trip::<UriMetaSampleEntry>,
    class_round_trip::<VisualSampleEntry>,
    class_round_trip::<VolumetricVisualSampleEntry>,
    class_round_trip::<XmlMetaDataSampleEntry>,
    class_round_trip::<XmlSubtitleSampleEntry>,
];

fuzz_target!(|bytes: &[u8]| {
    for framed in Boxes::new(bytes) {
        let Ok(framed) = framed else { break };
        let box_type = framed.header().box_type();

        for (_, check) in BOXES.iter().filter(|(listed, _)| *listed == box_type) {
            check(framed.payload());
        }
    }
});

/// Checks the three properties over a payload read as `Box`
fn round_trip<Box: BoxDecode + BoxEncode + PartialEq + Debug>(payload: &[u8]) {
    let Ok(read) = Box::decode_payload(payload) else {
        return;
    };

    let mut written = vec![0; usize::try_from(read.payload_len()).unwrap()];
    if let Err(error) = read.encode_payload(&mut written) {
        assert!(
            refuses_a_value(error.kind()),
            "box read fails to write back: {error}\n{read:?}"
        );
        return;
    }

    match Box::decode_payload(&written) {
        Ok(read_back) => assert_eq!(read_back, read, "box written back reads back as another"),
        Err(error) => panic!("box written back fails to read back: {error}\n{read:?}"),
    }
}

/// Checks a [`SampleDescriptionBox`], and every sample entry it holds as every class
fn sample_descriptions(payload: &[u8]) {
    round_trip::<SampleDescriptionBox>(payload);

    let Ok(read) = SampleDescriptionBox::decode_payload(payload) else {
        return;
    };
    for description in &read.sample_descriptions {
        if let SampleDescription::Typed(entry) = description {
            for check in SAMPLE_ENTRY_CLASSES {
                check(entry);
            }
        }
    }
}

/// Checks properties 2 and 3 over a sample entry read as `Class`
fn class_round_trip<Class>(entry: &SampleEntry)
where
    Class: for<'entry> TryFrom<&'entry SampleEntry, Error = Error> + PartialEq + Debug,
    SampleEntry: for<'class> TryFrom<&'class Class, Error = Error>,
{
    let Ok(read) = Class::try_from(entry) else {
        return;
    };

    let written = match SampleEntry::try_from(&read) {
        Ok(written) => written,
        Err(error) => {
            assert!(
                refuses_a_value(error.kind()),
                "sample entry read fails to write back: {error}\n{read:?}"
            );
            return;
        }
    };

    match Class::try_from(&written) {
        Ok(read_back) => assert_eq!(
            read_back, read,
            "sample entry written back reads back as another"
        ),
        Err(error) => panic!("sample entry written back fails to read back: {error}\n{read:?}"),
    }
}

/// Returns whether a writer refuses a value with `kind`, rather than failing to lay down what it holds
///
/// These are the values the specification does not allow, which a reader keeps
/// as found: a field, a string that is not in its encoding, and a container
/// holding a child more or fewer times than the specification lets it.
fn refuses_a_value(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::InvalidValue { .. }
            | ErrorKind::InvalidUtf8String
            | ErrorKind::InvalidUtfString
            | ErrorKind::InvalidBase64String
            | ErrorKind::MissingMandatoryBox { .. }
            | ErrorKind::DuplicateBox { .. }
            | ErrorKind::ForbiddenChildBox { .. }
            | ErrorKind::MissingVariantBox { .. }
            | ErrorKind::DuplicateVariantBox { .. }
    )
}
