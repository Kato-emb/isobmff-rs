//! [`SampleGroupDescriptionEntry`], the entry a `SampleGroupDescriptionBox` holds, and the entries of ISO/IEC 14496-12 it reads

mod clean_aperture;
mod essential_descriptions_hierarchy;
mod pixel_aspect_ratio;
mod random_access;
mod rate_share;
mod sample_to_metadata_item;
mod t35;
mod temporal_level;

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

pub use self::clean_aperture::CleanApertureEntry;
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

/// [`SampleGroupDescriptionEntry`], ISO/IEC 14496-12 §8.9.3, an entry of a `SampleGroupDescriptionBox`
///
/// The `grouping_type` of the box decides the syntax of its entries: an entry
/// of a `grouping_type` ISO/IEC 14496-12 defines is read into its type, and
/// any other entry is kept as its bytes.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SampleGroupDescriptionEntry {
    /// Entry of `grouping_type` `roll`
    RollRecovery(RollRecoveryEntry),
    /// Entry of `grouping_type` `prol`
    AudioPreRoll(AudioPreRollEntry),
    /// Entry of `grouping_type` `rash`
    RateShare(RateShareEntry),
    /// Entry of `grouping_type` `alst`
    AlternativeStartup(AlternativeStartupEntry),
    /// Entry of `grouping_type` `rap `
    VisualRandomAccess(VisualRandomAccessEntry),
    /// Entry of `grouping_type` `tele`
    TemporalLevel(TemporalLevelEntry),
    /// Entry of `grouping_type` `sap `
    Sap(SapEntry),
    /// Entry of `grouping_type` `stmi`
    SampleToMetadataItem(SampleToMetadataItemEntry),
    /// Entry of `grouping_type` `drap`
    VisualDrap(VisualDrapEntry),
    /// Entry of `grouping_type` `pasr`
    PixelAspectRatio(PixelAspectRatioEntry),
    /// Entry of `grouping_type` `casg`
    CleanAperture(CleanApertureEntry),
    /// Entry of `grouping_type` `edrp`
    VisualEdrap(VisualEdrapEntry),
    /// Entry of `grouping_type` `esgh`
    EssentialDescriptionsHierarchy(EssentialDescriptionsHierarchyEntry),
    /// Entry of `grouping_type` `it35`
    T35SampleGroup(T35SampleGroupEntry),
    /// Entry of a `grouping_type` this crate does not read, or of one it reads
    /// that does not decode, kept as its bytes
    Other(Vec<u8>),
}

impl SampleGroupDescriptionEntry {
    /// Returns the `grouping_type` of the entry, or `None` for one kept as its bytes
    pub(crate) fn grouping_type(&self) -> Option<FourCc> {
        let grouping_type = match self {
            Self::RollRecovery(_) => b"roll",
            Self::AudioPreRoll(_) => b"prol",
            Self::RateShare(_) => b"rash",
            Self::AlternativeStartup(_) => b"alst",
            Self::VisualRandomAccess(_) => b"rap ",
            Self::TemporalLevel(_) => b"tele",
            Self::Sap(_) => b"sap ",
            Self::SampleToMetadataItem(_) => b"stmi",
            Self::VisualDrap(_) => b"drap",
            Self::PixelAspectRatio(_) => b"pasr",
            Self::CleanAperture(_) => b"casg",
            Self::VisualEdrap(_) => b"edrp",
            Self::EssentialDescriptionsHierarchy(_) => b"esgh",
            Self::T35SampleGroup(_) => b"it35",
            Self::Other(_) => return None,
        };

        Some(FourCc::new(*grouping_type))
    }

    /// Returns whether an entry of `grouping_type` runs to the end of the bytes it is given, so that only a stated length bounds it
    pub(crate) fn runs_to_end(grouping_type: FourCc) -> bool {
        matches!(grouping_type.as_bytes(), b"alst" | b"it35")
    }

    /// Reads an entry of `grouping_type` off the front of `reader`, or returns `None` for a `grouping_type` this crate does not read
    ///
    /// # Errors
    ///
    /// * [`TruncatedPayload`](isobmff_core::ErrorKind::TruncatedPayload): the
    ///   payload ends inside the entry.
    /// * [`UnsupportedValue`](isobmff_core::ErrorKind::UnsupportedValue): the
    ///   `reserved` bits of a `drap` or `edrp` entry are not zero.
    pub(crate) fn read(
        grouping_type: FourCc,
        reader: &mut FieldReader<'_>,
    ) -> Option<Result<Self, Error>> {
        let read = match grouping_type.as_bytes() {
            b"roll" => RollRecoveryEntry::read(reader).map(Self::RollRecovery),
            b"prol" => AudioPreRollEntry::read(reader).map(Self::AudioPreRoll),
            b"rash" => RateShareEntry::read(reader).map(Self::RateShare),
            b"alst" => AlternativeStartupEntry::read(reader).map(Self::AlternativeStartup),
            b"rap " => VisualRandomAccessEntry::read(reader).map(Self::VisualRandomAccess),
            b"tele" => TemporalLevelEntry::read(reader).map(Self::TemporalLevel),
            b"sap " => SapEntry::read(reader).map(Self::Sap),
            b"stmi" => SampleToMetadataItemEntry::read(reader).map(Self::SampleToMetadataItem),
            b"drap" => VisualDrapEntry::read(reader).map(Self::VisualDrap),
            b"pasr" => PixelAspectRatioEntry::read(reader).map(Self::PixelAspectRatio),
            b"casg" => CleanApertureEntry::read(reader).map(Self::CleanAperture),
            b"edrp" => VisualEdrapEntry::read(reader).map(Self::VisualEdrap),
            b"esgh" => EssentialDescriptionsHierarchyEntry::read(reader)
                .map(Self::EssentialDescriptionsHierarchy),
            b"it35" => T35SampleGroupEntry::read(reader).map(Self::T35SampleGroup),
            _ => return None,
        };

        Some(read)
    }

    /// Returns how many bytes the entry takes, saturating at `u64::MAX`
    pub(crate) fn encoded_len(&self) -> u64 {
        match self {
            Self::RollRecovery(_) | Self::AudioPreRoll(_) => 2,
            Self::RateShare(rate_share) => rate_share.encoded_len(),
            Self::AlternativeStartup(alternative_startup) => alternative_startup.encoded_len(),
            Self::VisualRandomAccess(_) | Self::TemporalLevel(_) | Self::Sap(_) => 1,
            Self::SampleToMetadataItem(sample_to_item) => sample_to_item.encoded_len(),
            Self::VisualDrap(_) => 4,
            Self::PixelAspectRatio(_) => 8,
            Self::CleanAperture(_) => 32,
            Self::VisualEdrap(edrap) => edrap.encoded_len(),
            Self::EssentialDescriptionsHierarchy(hierarchy) => hierarchy.encoded_len(),
            Self::T35SampleGroup(t35) => t35.encoded_len(),
            Self::Other(bytes) => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        }
    }

    /// Writes the entry onto the front of what `writer` has left
    ///
    /// # Errors
    ///
    /// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue): a field holds
    ///   a value its entry refuses to write.
    /// * [`TruncatedBuffer`](isobmff_core::ErrorKind::TruncatedBuffer): the
    ///   buffer ends inside the entry.
    pub(crate) fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        match self {
            Self::RollRecovery(roll) => roll.write(writer),
            Self::AudioPreRoll(pre_roll) => pre_roll.write(writer),
            Self::RateShare(rate_share) => rate_share.write(writer),
            Self::AlternativeStartup(alternative_startup) => alternative_startup.write(writer),
            Self::VisualRandomAccess(random_access) => random_access.write(writer),
            Self::TemporalLevel(temporal_level) => temporal_level.write(writer),
            Self::Sap(sap) => sap.write(writer),
            Self::SampleToMetadataItem(sample_to_item) => sample_to_item.write(writer),
            Self::VisualDrap(drap) => drap.write(writer),
            Self::PixelAspectRatio(pixel_aspect_ratio) => pixel_aspect_ratio.write(writer),
            Self::CleanAperture(clean_aperture) => clean_aperture.write(writer),
            Self::VisualEdrap(edrap) => edrap.write(writer),
            Self::EssentialDescriptionsHierarchy(hierarchy) => hierarchy.write(writer),
            Self::T35SampleGroup(t35) => t35.write(writer),
            Self::Other(bytes) => writer.write_slice(bytes),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

    use super::{
        AlternativeStartupEntry, AlternativeStartupPiece, AudioPreRollEntry, CleanApertureEntry,
        EssentialDescriptionsHierarchyEntry, PixelAspectRatioEntry, RateShareEntry,
        RateShareOperationPoint, RateShareOperationPoints, RollRecoveryEntry,
        SampleGroupDescriptionEntry, SampleToMetadataItemEntry, SapEntry, T35SampleGroupEntry,
        TemporalLevelEntry, VisualDrapEntry, VisualEdrapEntry, VisualRandomAccessEntry,
    };

    /// Entries of every `grouping_type` ISO/IEC 14496-12 defines, each with the bytes its syntax lays it out in
    fn every_entry() -> Vec<(&'static [u8; 4], &'static [u8], SampleGroupDescriptionEntry)> {
        use SampleGroupDescriptionEntry as Entry;

        vec![
            (
                b"roll",
                b"\xff\xfe",
                Entry::RollRecovery(RollRecoveryEntry { roll_distance: -2 }),
            ),
            (
                b"prol",
                b"\0\x05",
                Entry::AudioPreRoll(AudioPreRollEntry { roll_distance: 5 }),
            ),
            (
                b"rash",
                b"\0\x01\0\x32\0\0\x01\0\0\0\0\x10\x80",
                Entry::RateShare(RateShareEntry {
                    operation_points: RateShareOperationPoints::Single {
                        target_rate_share: 50,
                    },
                    maximum_bitrate: 256,
                    minimum_bitrate: 16,
                    discard_priority: 128,
                }),
            ),
            (
                b"rash",
                b"\0\x02\0\0\0\x40\0\x14\0\0\x01\0\0\x1e\0\0\0\0\0\0\0\0\0",
                Entry::RateShare(RateShareEntry {
                    operation_points: RateShareOperationPoints::Multiple {
                        operation_points: vec![
                            RateShareOperationPoint {
                                available_bitrate: 64,
                                target_rate_share: 20,
                            },
                            RateShareOperationPoint {
                                available_bitrate: 256,
                                target_rate_share: 30,
                            },
                        ],
                    },
                    ..RateShareEntry::default()
                }),
            ),
            (
                b"alst",
                b"\0\x02\0\x01\0\0\0\0\0\0\x0b\xb8\0\x01\0\x02",
                Entry::AlternativeStartup(AlternativeStartupEntry {
                    first_output_sample: 1,
                    sample_offsets: vec![0, 3000],
                    pieces: vec![AlternativeStartupPiece {
                        num_output_samples: 1,
                        num_total_samples: 2,
                    }],
                }),
            ),
            (
                b"rap ",
                b"\x83",
                Entry::VisualRandomAccess(VisualRandomAccessEntry {
                    num_leading_samples_known: true,
                    num_leading_samples: 3,
                }),
            ),
            (
                b"tele",
                b"\x80",
                Entry::TemporalLevel(TemporalLevelEntry {
                    level_independently_decodable: true,
                }),
            ),
            (
                b"sap ",
                b"\x82",
                Entry::Sap(SapEntry {
                    dependent_flag: true,
                    sap_type: 2,
                }),
            ),
            (
                b"stmi",
                b"meta\0\0\0\x02\0\0\0\x01\0\0\0\x07",
                Entry::SampleToMetadataItem(SampleToMetadataItemEntry {
                    meta_box_handler_type: FourCc::new(*b"meta"),
                    item_ids: vec![1, 7],
                }),
            ),
            (
                b"drap",
                b"\x40\0\0\0",
                Entry::VisualDrap(VisualDrapEntry { drap_type: 2 }),
            ),
            (
                b"pasr",
                b"\0\0\0\x04\0\0\0\x03",
                Entry::PixelAspectRatio(PixelAspectRatioEntry {
                    h_spacing: 4,
                    v_spacing: 3,
                }),
            ),
            (
                b"casg",
                b"\0\0\x07\x80\0\0\0\x01\0\0\x04\x38\0\0\0\x01\
                  \xff\xff\xff\xf8\0\0\0\x01\0\0\0\0\0\0\0\x01",
                Entry::CleanAperture(CleanApertureEntry {
                    clean_aperture_width_n: 1920,
                    clean_aperture_width_d: 1,
                    clean_aperture_height_n: 1080,
                    clean_aperture_height_d: 1,
                    horiz_off_n: -8,
                    horiz_off_d: 1,
                    vert_off_n: 0,
                    vert_off_d: 1,
                }),
            ),
            (
                b"edrp",
                b"\x24\0\x01\0\x03",
                Entry::VisualEdrap(VisualEdrapEntry {
                    edrap_type: 1,
                    ref_sap_or_edrap_idx_deltas: vec![1, 3],
                }),
            ),
            (
                b"esgh",
                b"\0\0\0\x02stsdcenc",
                Entry::EssentialDescriptionsHierarchy(EssentialDescriptionsHierarchyEntry {
                    sample_group_description_types: vec![
                        FourCc::new(*b"stsd"),
                        FourCc::new(*b"cenc"),
                    ],
                }),
            ),
            (
                b"it35",
                b"\x80\xb5\0\x31",
                Entry::T35SampleGroup(T35SampleGroupEntry {
                    complete_message_flag: true,
                    itu_t_t35_data: vec![0xb5, 0x00, 0x31],
                }),
            ),
        ]
    }

    #[test]
    fn reads_every_entry_from_the_bytes_its_syntax_lays_out() {
        let entries = every_entry();

        assert_eq!(
            entries
                .iter()
                .map(|(grouping_type, bytes, _)| {
                    let mut reader = FieldReader::new(bytes);
                    SampleGroupDescriptionEntry::read(FourCc::new(**grouping_type), &mut reader)
                        .unwrap()
                        .map(|entry| (entry, reader.remainder().len()))
                })
                .collect::<Vec<_>>(),
            entries
                .iter()
                .map(|(_, _, entry)| Ok((entry.clone(), 0)))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn writes_every_entry_in_the_bytes_its_syntax_lays_out() {
        let entries = every_entry();

        assert_eq!(
            entries
                .iter()
                .map(|(_, _, entry)| {
                    let mut buffer = vec![0; usize::try_from(entry.encoded_len()).unwrap()];
                    entry.write(&mut FieldWriter::new(&mut buffer)).unwrap();
                    (entry.grouping_type(), buffer)
                })
                .collect::<Vec<_>>(),
            entries
                .iter()
                .map(|(grouping_type, bytes, _)| {
                    (Some(FourCc::new(**grouping_type)), bytes.to_vec())
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_grouping_type_the_crate_does_not_read_is_not_read() {
        assert_eq!(
            SampleGroupDescriptionEntry::read(
                FourCc::new(*b"seig"),
                &mut FieldReader::new(b"\0\0\x01\x10")
            ),
            None
        );
    }

    #[test]
    fn a_drap_or_edrp_entry_whose_reserved_bits_are_set_is_refused() {
        assert_eq!(
            [
                SampleGroupDescriptionEntry::read(
                    FourCc::new(*b"drap"),
                    &mut FieldReader::new(b"\x40\0\0\x01")
                ),
                SampleGroupDescriptionEntry::read(
                    FourCc::new(*b"edrp"),
                    &mut FieldReader::new(b"\x21\0\x01")
                )
            ],
            [
                Some(Err(Error::unsupported_value("reserved"))),
                Some(Err(Error::unsupported_value("reserved")))
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
            SampleGroupDescriptionEntry::RollRecovery(RollRecoveryEntry { roll_distance: 0 }),
            SampleGroupDescriptionEntry::AudioPreRoll(AudioPreRollEntry { roll_distance: -1 }),
            SampleGroupDescriptionEntry::VisualRandomAccess(VisualRandomAccessEntry {
                num_leading_samples_known: true,
                num_leading_samples: 0x80,
            }),
            SampleGroupDescriptionEntry::RateShare(RateShareEntry {
                operation_points: RateShareOperationPoints::Multiple {
                    operation_points: vec![operation_point(64)],
                },
                ..RateShareEntry::default()
            }),
            SampleGroupDescriptionEntry::RateShare(RateShareEntry {
                operation_points: RateShareOperationPoints::Multiple {
                    operation_points: vec![operation_point(64), operation_point(64)],
                },
                ..RateShareEntry::default()
            }),
            SampleGroupDescriptionEntry::Sap(SapEntry {
                dependent_flag: false,
                sap_type: 7,
            }),
            SampleGroupDescriptionEntry::VisualDrap(VisualDrapEntry { drap_type: 4 }),
            SampleGroupDescriptionEntry::VisualEdrap(VisualEdrapEntry {
                edrap_type: 4,
                ref_sap_or_edrap_idx_deltas: vec![1],
            }),
            SampleGroupDescriptionEntry::VisualEdrap(VisualEdrapEntry {
                edrap_type: 1,
                ref_sap_or_edrap_idx_deltas: Vec::new(),
            }),
        ];

        assert_eq!(
            refused
                .iter()
                .map(|entry| entry.write(&mut FieldWriter::new(&mut [0; 32])))
                .collect::<Vec<_>>(),
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
