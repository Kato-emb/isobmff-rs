//! [`SampleEntry`], the abstract class every sample entry derives from, and the classes ISO/IEC 14496-12 derives from it

mod audio;
mod audio_v1;
mod children;
mod entry;
mod fd_hint;
mod font;
mod haptic;
mod hint;
mod metadata;
mod mime_format;
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

pub(crate) use children::encode_optional;
pub use entry::SampleEntry;
pub(crate) use entry::read_string;
