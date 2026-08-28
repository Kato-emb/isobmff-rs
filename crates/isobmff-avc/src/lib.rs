//! Sample entries and decoder configuration of ISO/IEC 14496-15, the carriage
//! of AVC video in ISO base media files
//!
//! An AVC track describes its samples with an [`AVCSampleEntry`] — `avc1`
//! when the parameter sets lie in the entry alone, `avc3` when they may also
//! lie in the samples — which holds an [`AVCConfigurationBox`] (`avcC`)
//! around the [`AVCDecoderConfigurationRecord`] a decoder starts from. The
//! record is read field by field; the parameter sets it carries stay the NAL
//! units ISO/IEC 14496-10 lays out, unread.
//!
//! The entries of a `stsd` are kept as
//! [`AnyBox`](isobmff_core::AnyBox) by `isobmff-boxes`; one that names an
//! AVC coding decodes here through
//! [`AVCSampleEntry::decode_payload`], and goes back in through `AnyBox::from`.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: parameter sets and the boxes an
//! entry holds are owned.

#![no_std]

extern crate alloc;

mod avc_sample_entry;
mod avcc;
mod decoder_configuration_record;

pub use avc_sample_entry::{AVCSampleEntry, AVCSampleEntryType};
pub use avcc::AVCConfigurationBox;
pub use decoder_configuration_record::{
    AVCDecoderConfigurationRecord, HighProfileFields, LengthSizeMinusOne,
};
