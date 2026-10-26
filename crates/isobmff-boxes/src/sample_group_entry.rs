//! [`SampleGroupDescriptionEntry`], the entry a `SampleGroupDescriptionBox` holds, and the entries of ISO/IEC 14496-12 it reads

mod random_access;

use alloc::vec::Vec;

use isobmff_core::{Error, FieldReader, FieldWriter, FourCc};

pub use self::random_access::{
    AlternativeStartupEntry, AlternativeStartupPiece, AudioPreRollEntry, RollRecoveryEntry,
    SapEntry, VisualDrapEntry, VisualEdrapEntry, VisualRandomAccessEntry,
};
use crate::table::{
    TableEntry, counted_len, entries_len, read_counted, read_entries, write_counted, write_entries,
};

/// Operation point of a [`RateShareEntry`] that holds more than one
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct RateShareOperationPoint {
    /// Total bitrate available to share at this point, in kilobits per second
    pub available_bitrate: u32,
    /// Percentage of the available bitrate to allocate to the media, 0 for no information
    pub target_rate_share: u16,
}

impl TableEntry for RateShareOperationPoint {
    const ENCODED_LEN: u64 = 6;

    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        Ok(Self {
            available_bitrate: reader.read_u32()?,
            target_rate_share: reader.read_u16()?,
        })
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.available_bitrate)?;
        writer.write_u16(self.target_rate_share)
    }
}

/// Target rate shares of a [`RateShareEntry`], for one operation point or more
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RateShareOperationPoints {
    /// One operation point, whose share applies at every available bitrate
    Single {
        /// Percentage of the available bitrate to allocate to the media, 0 for no information
        target_rate_share: u16,
    },
    /// Operation points in increasing order of `available_bitrate`, more than
    /// one where the entry is written
    Multiple {
        /// The operation points
        operation_points: Vec<RateShareOperationPoint>,
    },
}

impl Default for RateShareOperationPoints {
    fn default() -> Self {
        Self::Single {
            target_rate_share: 0,
        }
    }
}

/// [`RateShareEntry`] (`rash`), ISO/IEC 14496-12 §10.2.2
///
/// How to share the available bitrate between the media of a track and the
/// others. `operation_point_count` is the number of operation points; a
/// [`Multiple`](RateShareOperationPoints::Multiple) of fewer than two is
/// refused when written, with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for
/// `operation_point_count`, as is one whose `available_bitrate`s are not
/// positive and strictly increasing, for `available_bitrate`.
#[doc(alias = "rash")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct RateShareEntry {
    /// Target rate shares
    pub operation_points: RateShareOperationPoints,
    /// Bitrate above which to allocate no more to the media, in kilobits per
    /// second, 0 for no information
    pub maximum_bitrate: u32,
    /// Bitrate below which to allocate nothing to the media, in kilobits per
    /// second, 0 for no information
    pub minimum_bitrate: u32,
    /// Order in which tracks are discarded, the highest first
    pub discard_priority: u8,
}

impl RateShareEntry {
    fn read(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let operation_point_count = reader.read_u16()?;
        let operation_points = if operation_point_count == 1 {
            RateShareOperationPoints::Single {
                target_rate_share: reader.read_u16()?,
            }
        } else {
            RateShareOperationPoints::Multiple {
                operation_points: read_entries(
                    reader,
                    u32::from(operation_point_count),
                    RateShareOperationPoint::ENCODED_LEN,
                    RateShareOperationPoint::read,
                )?,
            }
        };

        Ok(Self {
            operation_points,
            maximum_bitrate: reader.read_u32()?,
            minimum_bitrate: reader.read_u32()?,
            discard_priority: reader.read_bytes::<1>()?[0],
        })
    }

    fn encoded_len(&self) -> u64 {
        let operation_points_len = match &self.operation_points {
            RateShareOperationPoints::Single { .. } => 2,
            RateShareOperationPoints::Multiple { operation_points } => {
                entries_len(operation_points)
            }
        };

        operation_points_len.saturating_add(2 + 4 + 4 + 1)
    }

    fn write(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        match &self.operation_points {
            RateShareOperationPoints::Single { target_rate_share } => {
                writer.write_u16(1)?;
                writer.write_u16(*target_rate_share)?;
            }
            RateShareOperationPoints::Multiple { operation_points } => {
                let operation_point_count = u16::try_from(operation_points.len())
                    .ok()
                    .filter(|count| *count > 1)
                    .ok_or_else(|| Error::invalid_value("operation_point_count"))?;
                let increasing = operation_points
                    .iter()
                    .try_fold(0, |previous, point| {
                        (point.available_bitrate > previous).then_some(point.available_bitrate)
                    })
                    .is_some();
                if !increasing {
                    return Err(Error::invalid_value("available_bitrate"));
                }
                writer.write_u16(operation_point_count)?;
                write_entries(writer, operation_points)?;
            }
        }
        writer.write_u32(self.maximum_bitrate)?;
        writer.write_u32(self.minimum_bitrate)?;
        writer.write_bytes(&[self.discard_priority])
    }
}

/// [`TemporalLevelEntry`] (`tele`), ISO/IEC 14496-12 §10.5
///
/// A temporal level, which is the index of the entry: its samples depend on
/// none of a higher level.
#[doc(alias = "tele")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TemporalLevelEntry {
    /// Whether the samples of the level depend on none of other levels,
    /// `false` for no information
    pub level_independently_decodable: bool,
}

/// [`SampleToMetadataItemEntry`] (`stmi`), ISO/IEC 14496-12 §10.7
///
/// The metadata items that apply to the samples.
#[doc(alias = "stmi")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleToMetadataItemEntry {
    /// `handler_type` of the `MetaBox` that holds the items
    pub meta_box_handler_type: FourCc,
    /// `item_id` of every item that applies to the samples
    pub item_ids: Vec<u32>,
}

impl Default for SampleToMetadataItemEntry {
    fn default() -> Self {
        Self {
            meta_box_handler_type: FourCc::new([0; 4]),
            item_ids: Vec::new(),
        }
    }
}

/// [`PixelAspectRatioEntry`] (`pasr`), ISO/IEC 14496-12 §10.9
///
/// The pixel aspect ratio of the samples of a video track whose ratio
/// changes, as a `PixelAspectRatioBox` states it.
#[doc(alias = "pasr")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct PixelAspectRatioEntry {
    /// Relative width of a pixel
    pub h_spacing: u32,
    /// Relative height of a pixel
    pub v_spacing: u32,
}

/// [`CleanApertureEntry`] (`casg`), ISO/IEC 14496-12 §10.10
///
/// The clean aperture of the samples of a video track whose aperture
/// changes, as a `CleanApertureBox` states it, each value a fraction of a
/// numerator and a denominator.
#[doc(alias = "casg")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct CleanApertureEntry {
    /// Numerator of the width of the clean aperture, in pixels
    pub clean_aperture_width_n: i32,
    /// Denominator of the width of the clean aperture
    pub clean_aperture_width_d: i32,
    /// Numerator of the height of the clean aperture, in pixels
    pub clean_aperture_height_n: i32,
    /// Denominator of the height of the clean aperture
    pub clean_aperture_height_d: i32,
    /// Numerator of the horizontal offset of the center of the clean aperture
    pub horiz_off_n: i32,
    /// Denominator of the horizontal offset of the center of the clean aperture
    pub horiz_off_d: i32,
    /// Numerator of the vertical offset of the center of the clean aperture
    pub vert_off_n: i32,
    /// Denominator of the vertical offset of the center of the clean aperture
    pub vert_off_d: i32,
}

/// [`EssentialDescriptionsHierarchyEntry`] (`esgh`), ISO/IEC 14496-12 §10.12
///
/// The order in which a reader applies the essential sample group
/// descriptions, and the transformations of reserved values such as `stsd`
/// and `cenc`, to a sample.
#[doc(alias = "esgh")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct EssentialDescriptionsHierarchyEntry {
    /// `sample_group_description_type` of each, in the order they apply
    pub sample_group_description_types: Vec<FourCc>,
}

/// [`T35SampleGroupEntry`] (`it35`), ISO/IEC 14496-12 §10.13
///
/// An ITU-T T.35 message, whole or its part that identifies its type. The
/// data runs to the end of the entry, so it takes the length a
/// `SampleGroupDescriptionBox` of version 1 or more states.
#[doc(alias = "it35")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct T35SampleGroupEntry {
    /// Whether `itu_t_t35_data` holds the whole message, which the samples then do not
    pub complete_message_flag: bool,
    /// The message from its first byte, without emulation prevention bits
    pub itu_t_t35_data: Vec<u8>,
}

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
            b"roll" => reader
                .read_i16()
                .map(|roll_distance| Self::RollRecovery(RollRecoveryEntry { roll_distance })),
            b"prol" => reader
                .read_i16()
                .map(|roll_distance| Self::AudioPreRoll(AudioPreRollEntry { roll_distance })),
            b"rash" => RateShareEntry::read(reader).map(Self::RateShare),
            b"alst" => AlternativeStartupEntry::read(reader).map(Self::AlternativeStartup),
            b"rap " => VisualRandomAccessEntry::read(reader).map(Self::VisualRandomAccess),
            b"tele" => reader.read_bytes::<1>().map(|&[byte]| {
                Self::TemporalLevel(TemporalLevelEntry {
                    level_independently_decodable: byte & 0x80 != 0,
                })
            }),
            b"sap " => SapEntry::read(reader).map(Self::Sap),
            b"stmi" => reader.read_bytes::<4>().and_then(|&handler_type| {
                Ok(Self::SampleToMetadataItem(SampleToMetadataItemEntry {
                    meta_box_handler_type: FourCc::new(handler_type),
                    item_ids: read_counted(reader)?,
                }))
            }),
            b"drap" => VisualDrapEntry::read(reader).map(Self::VisualDrap),
            b"pasr" => reader.read_u32().and_then(|h_spacing| {
                Ok(Self::PixelAspectRatio(PixelAspectRatioEntry {
                    h_spacing,
                    v_spacing: reader.read_u32()?,
                }))
            }),
            b"casg" => reader.read_i32().and_then(|clean_aperture_width_n| {
                Ok(Self::CleanAperture(CleanApertureEntry {
                    clean_aperture_width_n,
                    clean_aperture_width_d: reader.read_i32()?,
                    clean_aperture_height_n: reader.read_i32()?,
                    clean_aperture_height_d: reader.read_i32()?,
                    horiz_off_n: reader.read_i32()?,
                    horiz_off_d: reader.read_i32()?,
                    vert_off_n: reader.read_i32()?,
                    vert_off_d: reader.read_i32()?,
                }))
            }),
            b"edrp" => VisualEdrapEntry::read(reader).map(Self::VisualEdrap),
            b"esgh" => read_counted(reader).map(|sample_group_description_types| {
                Self::EssentialDescriptionsHierarchy(EssentialDescriptionsHierarchyEntry {
                    sample_group_description_types,
                })
            }),
            b"it35" => reader.read_bytes::<1>().map(|&[byte]| {
                Self::T35SampleGroup(T35SampleGroupEntry {
                    complete_message_flag: byte & 0x80 != 0,
                    itu_t_t35_data: reader.take_remainder().to_vec(),
                })
            }),
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
            Self::SampleToMetadataItem(sample_to_item) => {
                counted_len(&sample_to_item.item_ids).saturating_add(4)
            }
            Self::VisualDrap(_) => 4,
            Self::PixelAspectRatio(_) => 8,
            Self::CleanAperture(_) => 32,
            Self::VisualEdrap(edrap) => edrap.encoded_len(),
            Self::EssentialDescriptionsHierarchy(hierarchy) => {
                counted_len(&hierarchy.sample_group_description_types)
            }
            Self::T35SampleGroup(t35) => u64::try_from(t35.itu_t_t35_data.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
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
            Self::RollRecovery(RollRecoveryEntry { roll_distance }) => {
                if *roll_distance == 0 {
                    return Err(Error::invalid_value("roll_distance"));
                }
                writer.write_i16(*roll_distance)
            }
            Self::AudioPreRoll(AudioPreRollEntry { roll_distance }) => {
                if *roll_distance <= 0 {
                    return Err(Error::invalid_value("roll_distance"));
                }
                writer.write_i16(*roll_distance)
            }
            Self::RateShare(rate_share) => rate_share.write(writer),
            Self::AlternativeStartup(alternative_startup) => alternative_startup.write(writer),
            Self::VisualRandomAccess(random_access) => random_access.write(writer),
            Self::TemporalLevel(temporal_level) => {
                writer.write_bytes(&[if temporal_level.level_independently_decodable {
                    0x80
                } else {
                    0
                }])
            }
            Self::Sap(sap) => sap.write(writer),
            Self::SampleToMetadataItem(sample_to_item) => {
                writer.write_bytes(sample_to_item.meta_box_handler_type.as_bytes())?;
                write_counted(writer, &sample_to_item.item_ids)
            }
            Self::VisualDrap(drap) => drap.write(writer),
            Self::PixelAspectRatio(pixel_aspect_ratio) => {
                writer.write_u32(pixel_aspect_ratio.h_spacing)?;
                writer.write_u32(pixel_aspect_ratio.v_spacing)
            }
            Self::CleanAperture(clean_aperture) => [
                clean_aperture.clean_aperture_width_n,
                clean_aperture.clean_aperture_width_d,
                clean_aperture.clean_aperture_height_n,
                clean_aperture.clean_aperture_height_d,
                clean_aperture.horiz_off_n,
                clean_aperture.horiz_off_d,
                clean_aperture.vert_off_n,
                clean_aperture.vert_off_d,
            ]
            .into_iter()
            .try_for_each(|value| writer.write_i32(value)),
            Self::VisualEdrap(edrap) => edrap.write(writer),
            Self::EssentialDescriptionsHierarchy(hierarchy) => {
                write_counted(writer, &hierarchy.sample_group_description_types)
            }
            Self::T35SampleGroup(t35) => {
                writer.write_bytes(&[if t35.complete_message_flag { 0x80 } else { 0 }])?;
                writer.write_slice(&t35.itu_t_t35_data)
            }
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
