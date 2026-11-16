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

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::Error;

    use super::entry::entry_of;
    use super::{
        AudioPreRollEntry, RateShareEntry, RateShareOperationPoint, RateShareOperationPoints,
        RollRecoveryEntry, SampleGroupDescriptionEntry, SapEntry, VisualDrapEntry,
        VisualEdrapEntry, VisualRandomAccessEntry,
    };

    #[test]
    fn a_drap_or_edrp_entry_whose_reserved_bits_are_set_is_refused() {
        assert_eq!(
            [
                VisualDrapEntry::try_from(&entry_of(b"drap", b"\x40\0\0\x01")).map(|_| ()),
                VisualEdrapEntry::try_from(&entry_of(b"edrp", b"\x21\0\x01")).map(|_| ())
            ],
            [
                Err(Error::unsupported_value("reserved")),
                Err(Error::unsupported_value("reserved"))
            ]
        );
    }

    #[test]
    fn values_the_specification_does_not_allow_are_refused_when_written() {
        let operation_point = |available_bitrate| RateShareOperationPoint {
            available_bitrate,
            target_rate_share: 10,
        };
        let refused = [
            SampleGroupDescriptionEntry::try_from(&RollRecoveryEntry { roll_distance: 0 }),
            SampleGroupDescriptionEntry::try_from(&AudioPreRollEntry { roll_distance: -1 }),
            SampleGroupDescriptionEntry::try_from(&VisualRandomAccessEntry {
                num_leading_samples_known: true,
                num_leading_samples: 0x80,
            }),
            SampleGroupDescriptionEntry::try_from(&RateShareEntry {
                operation_points: RateShareOperationPoints::Multiple {
                    operation_points: vec![operation_point(64)],
                },
                ..RateShareEntry::default()
            }),
            SampleGroupDescriptionEntry::try_from(&RateShareEntry {
                operation_points: RateShareOperationPoints::Multiple {
                    operation_points: vec![operation_point(64), operation_point(64)],
                },
                ..RateShareEntry::default()
            }),
            SampleGroupDescriptionEntry::try_from(&SapEntry {
                dependent_flag: false,
                sap_type: 7,
            }),
            SampleGroupDescriptionEntry::try_from(&VisualDrapEntry { drap_type: 4 }),
            SampleGroupDescriptionEntry::try_from(&VisualEdrapEntry {
                edrap_type: 4,
                ref_sap_or_edrap_idx_deltas: vec![1],
            }),
            SampleGroupDescriptionEntry::try_from(&VisualEdrapEntry {
                edrap_type: 1,
                ref_sap_or_edrap_idx_deltas: Vec::new(),
            }),
        ];

        assert_eq!(
            refused,
            [
                Err(Error::invalid_value("roll_distance")),
                Err(Error::invalid_value("roll_distance")),
                Err(Error::invalid_value("num_leading_samples")),
                Err(Error::invalid_value("operation_point_count")),
                Err(Error::invalid_value("available_bitrate")),
                Err(Error::invalid_value("SAP_type")),
                Err(Error::invalid_value("DRAP_type")),
                Err(Error::invalid_value("edrap_type")),
                Err(Error::invalid_value("num_ref_sap_or_edrap_samples_minus1"))
            ]
        );
    }
}
