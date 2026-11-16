//! The classes ISO/IEC 14496-12 derives from [`SampleGroupDescriptionEntry`] for each `grouping_type`, and the delimiting of an entry by its syntax

mod clean_aperture;
mod entry;
mod essential_descriptions_hierarchy;
mod pixel_aspect_ratio;
mod random_access;
mod rate_share;
mod sample_to_metadata_item;
mod t35;
mod temporal_level;

use isobmff_core::{Error, FieldReader, FourCc};

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

use self::entry::DerivedEntry;

/// Returns whether an entry of `grouping_type` runs to the end of the bytes it is given, so that only a stated length bounds it
pub(crate) fn runs_to_end(grouping_type: FourCc) -> bool {
    matches!(grouping_type.as_bytes(), b"alst" | b"it35")
}

/// Returns how many bytes an entry of `grouping_type` at the front of `reader` takes as its syntax lays it out, reading past them
///
/// An entry of a `SampleGroupDescriptionBox` of version 0 states no length,
/// so its syntax alone delimits it.
///
/// # Errors
///
/// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
///   `grouping_type`: the crate does not read the `grouping_type`, or its
///   fields run to the end of the entry.
/// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
///   payload ends inside the entry.
/// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue) for
///   `reserved`: the `reserved` bits of a `drap` or `edrp` entry are not zero.
pub(crate) fn delimited_len(
    grouping_type: FourCc,
    reader: &mut FieldReader<'_>,
) -> Result<usize, Error> {
    fn read_past<Class: DerivedEntry>(reader: &mut FieldReader<'_>) -> Result<usize, Error> {
        let before = reader.remainder().len();
        Class::read(reader)?;

        Ok(before.saturating_sub(reader.remainder().len()))
    }

    if runs_to_end(grouping_type) {
        return Err(Error::unsupported_value("grouping_type"));
    }
    match grouping_type.as_bytes() {
        b"roll" => read_past::<RollRecoveryEntry>(reader),
        b"prol" => read_past::<AudioPreRollEntry>(reader),
        b"rash" => read_past::<RateShareEntry>(reader),
        b"rap " => read_past::<VisualRandomAccessEntry>(reader),
        b"tele" => read_past::<TemporalLevelEntry>(reader),
        b"sap " => read_past::<SapEntry>(reader),
        b"stmi" => read_past::<SampleToMetadataItemEntry>(reader),
        b"drap" => read_past::<VisualDrapEntry>(reader),
        b"pasr" => read_past::<PixelAspectRatioEntry>(reader),
        b"casg" => read_past::<CleanApertureEntry>(reader),
        b"edrp" => read_past::<VisualEdrapEntry>(reader),
        b"esgh" => read_past::<EssentialDescriptionsHierarchyEntry>(reader),
        _ => Err(Error::unsupported_value("grouping_type")),
    }
}

/// Reads an entry of `grouping_type` off the front of `reader` as far as its syntax lays it out, keeping its bytes
///
/// # Errors
///
/// * The failures of [`delimited_len`].
pub(crate) fn read_delimited(
    grouping_type: FourCc,
    reader: &mut FieldReader<'_>,
) -> Result<SampleGroupDescriptionEntry, Error> {
    let rest = reader.remainder();
    let length = delimited_len(grouping_type, reader)?;

    Ok(SampleGroupDescriptionEntry {
        grouping_type,
        data: rest.get(..length).unwrap_or_default().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{Error, FieldReader, FourCc};

    use super::{
        AudioPreRollEntry, RateShareEntry, RateShareOperationPoint, RateShareOperationPoints,
        RollRecoveryEntry, SampleGroupDescriptionEntry, SapEntry, VisualDrapEntry,
        VisualEdrapEntry, VisualRandomAccessEntry, read_delimited,
    };

    /// The bytes an entry of every `grouping_type` ISO/IEC 14496-12 defines lays out, of those whose syntax delimits them
    const EVERY_ENTRY: [(&[u8; 4], &[u8]); 13] = [
        (b"roll", b"\xff\xfe"),
        (b"prol", b"\0\x05"),
        (b"rash", b"\0\x01\0\x32\0\0\x01\0\0\0\0\x10\x80"),
        (
            b"rash",
            b"\0\x02\0\0\0\x40\0\x14\0\0\x01\0\0\x1e\0\0\0\0\0\0\0\0\0",
        ),
        (b"rap ", b"\x83"),
        (b"tele", b"\x80"),
        (b"sap ", b"\x82"),
        (b"stmi", b"meta\0\0\0\x02\0\0\0\x01\0\0\0\x07"),
        (b"drap", b"\x40\0\0\0"),
        (b"pasr", b"\0\0\0\x04\0\0\0\x03"),
        (
            b"casg",
            b"\0\0\x07\x80\0\0\0\x01\0\0\x04\x38\0\0\0\x01\
              \xff\xff\xff\xf8\0\0\0\x01\0\0\0\0\0\0\0\x01",
        ),
        (b"edrp", b"\x24\0\x01\0\x03"),
        (b"esgh", b"\0\0\0\x02stsdcenc"),
    ];

    #[test]
    fn reads_every_entry_as_far_as_its_syntax_lays_it_out() {
        assert_eq!(
            EVERY_ENTRY
                .iter()
                .map(|(grouping_type, bytes)| {
                    let mut reader = FieldReader::new(bytes);
                    read_delimited(FourCc::new(**grouping_type), &mut reader)
                        .map(|entry| (entry, reader.remainder().len()))
                })
                .collect::<Vec<_>>(),
            EVERY_ENTRY
                .iter()
                .map(|(grouping_type, bytes)| {
                    Ok((
                        SampleGroupDescriptionEntry {
                            grouping_type: FourCc::new(**grouping_type),
                            data: bytes.to_vec(),
                        },
                        0,
                    ))
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_entry_of_a_grouping_type_its_syntax_does_not_delimit_is_refused() {
        assert_eq!(
            [
                read_delimited(
                    FourCc::new(*b"seig"),
                    &mut FieldReader::new(b"\0\0\x01\x10")
                ),
                read_delimited(
                    FourCc::new(*b"alst"),
                    &mut FieldReader::new(b"\0\x01\0\x01")
                ),
                read_delimited(FourCc::new(*b"it35"), &mut FieldReader::new(b"\x80\xb5")),
            ],
            [
                Err(Error::unsupported_value("grouping_type")),
                Err(Error::unsupported_value("grouping_type")),
                Err(Error::unsupported_value("grouping_type"))
            ]
        );
    }

    #[test]
    fn a_drap_or_edrp_entry_whose_reserved_bits_are_set_is_refused() {
        assert_eq!(
            [
                read_delimited(
                    FourCc::new(*b"drap"),
                    &mut FieldReader::new(b"\x40\0\0\x01")
                ),
                read_delimited(FourCc::new(*b"edrp"), &mut FieldReader::new(b"\x21\0\x01"))
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
