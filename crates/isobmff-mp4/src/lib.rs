//! Sample entries and elementary stream descriptors of ISO/IEC 14496-14, the
//! MP4 file format
//!
//! An MPEG-4 audio track describes its samples with an
//! [`MP4AudioSampleEntry`] (`mp4a`), which holds an [`ESDBox`] (`esds`) around
//! the [`ESDescriptor`] of ISO/IEC 14496-1. The descriptor tree is read down to
//! the [`DecoderConfigDescriptor`] and the [`DecoderSpecificInfo`] it carries —
//! for AAC, the `AudioSpecificConfig` of ISO/IEC 14496-3, which stays the bytes
//! it lies as — and to the `predefined` value of the [`SLConfigDescriptor`].
//! Descriptors this crate has no type for are kept as [`RawDescriptor`] and
//! written back.
//!
//! The entries of a `stsd` are kept as [`AnyBox`](isobmff_core::AnyBox) by
//! `isobmff-boxes`; one named `mp4a` decodes here through
//! [`MP4AudioSampleEntry::decode_payload`], and goes back in through
//! `AnyBox::from`.
//!
//! Descriptors are not boxes, so what goes wrong inside them is reported by
//! this crate's own [`Error`], which carries a box failure through as one of
//! its kinds.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: descriptors and the boxes an entry
//! holds are owned.

#![no_std]

extern crate alloc;

mod descriptor;
mod error;
mod es_descriptor;
mod esds;
mod mp4a;

pub use descriptor::{DescriptorTag, RawDescriptor};
pub use error::{Error, ErrorKind};
pub use es_descriptor::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, SLConfigDescriptor,
};
pub use esds::ESDBox;
pub use mp4a::MP4AudioSampleEntry;
