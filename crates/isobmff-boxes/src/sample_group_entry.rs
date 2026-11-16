//! The classes ISO/IEC 14496-12 derives from [`SampleGroupDescriptionEntry`] for each `grouping_type`, and the length of those whose syntax fixes it

mod clean_aperture;
mod entry;
mod essential_descriptions_hierarchy;
mod pixel_aspect_ratio;
mod random_access;
mod rate_share;
mod sample_to_metadata_item;
mod t35;
mod temporal_level;

use isobmff_core::FourCc;

pub use self::clean_aperture::CleanApertureEntry;
pub use self::entry::SampleGroupDescriptionEntry;
pub use self::essential_descriptions_hierarchy::EssentialDescriptionsHierarchyEntry;
pub use self::pixel_aspect_ratio::PixelAspectRatioEntry;
pub use self::random_access::{
    AlternativeStartupEntry, AlternativeStartupPiece, AudioPreRollEntry, RollRecoveryEntry,
    SapEntry, VisualDrapEntry, VisualEdrapEntry, VisualRandomAccessEntry,
};
pub use self::rate_share::{RateShareEntry, RateShareOperationPoint, RateShareOperationPoints};
pub use self::sample_to_metadata_item::SampleToMetadataItemEntry;
pub use self::t35::T35SampleGroupEntry;
pub use self::temporal_level::TemporalLevelEntry;

/// Returns how many bytes an entry of `grouping_type` takes where its syntax fixes the length, or `None` where it does not
///
/// An entry of a `SampleGroupDescriptionBox` of version 0 states no length,
/// so the box reads one only of a `grouping_type` of a fixed length.
pub(crate) fn fixed_len(grouping_type: FourCc) -> Option<u64> {
    Some(match grouping_type.as_bytes() {
        b"roll" => RollRecoveryEntry::ENCODED_LEN,
        b"prol" => AudioPreRollEntry::ENCODED_LEN,
        b"rap " => VisualRandomAccessEntry::ENCODED_LEN,
        b"tele" => TemporalLevelEntry::ENCODED_LEN,
        b"sap " => SapEntry::ENCODED_LEN,
        b"drap" => VisualDrapEntry::ENCODED_LEN,
        b"pasr" => PixelAspectRatioEntry::ENCODED_LEN,
        b"casg" => CleanApertureEntry::ENCODED_LEN,
        _ => return None,
    })
}
