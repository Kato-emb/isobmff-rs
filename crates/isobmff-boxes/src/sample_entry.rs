//! [`SampleEntry`], the fields every sample entry opens with, and the bytes the class derived for its format lays out

mod audio;
mod audio_v1;
mod children;
mod entry;
mod fd_hint;
mod font;
mod haptic;
mod hint;
mod metadata;
mod mpeg2ts_hint;
mod protected_mpeg2ts_hint;
mod received_rtp_hint;
mod received_srtp_hint;
mod rtcp_hint;
mod rtp_hint;
mod subtitle;
mod text;
mod visual;
mod volumetric;

pub use audio::{AudioSampleEntry, AudioSampleEntryVariant};
pub use audio_v1::AudioSampleEntryV1;
pub use fd_hint::FdHintSampleEntry;
pub use font::FontSampleEntry;
pub use haptic::HapticSampleEntry;
pub use metadata::{
    BoxedMetadataSampleEntry, TextMetaDataSampleEntry, UriMetaSampleEntry, XmlMetaDataSampleEntry,
};
pub use mpeg2ts_hint::{Mpeg2TsReceptionSampleEntry, Mpeg2TsServerSampleEntry};
pub use protected_mpeg2ts_hint::ProtectedMpeg2TransportStreamSampleEntry;
pub use received_rtp_hint::{ProtectedRtpReceptionHintSampleEntry, ReceivedRtpHintSampleEntry};
pub use received_srtp_hint::ReceivedSrtpHintSampleEntry;
pub use rtcp_hint::{RtcpReceptionHintSampleEntry, SrtcpReceptionHintSampleEntry};
pub use rtp_hint::{RtpHintSampleEntry, SrtpHintSampleEntry};
pub use subtitle::{TextSubtitleSampleEntry, XmlSubtitleSampleEntry};
pub use text::SimpleTextSampleEntry;
pub use visual::VisualSampleEntry;
pub use volumetric::VolumetricVisualSampleEntry;

pub(crate) use children::{
    AnyEntryBoxes, AnyEntryChildren, children_len, encode_children, encode_optional,
};
pub use entry::SampleEntry;
pub(crate) use entry::{from_class, of_format, read_string, split_children};
pub(crate) use hint::HINT_TRACK_VERSION;
