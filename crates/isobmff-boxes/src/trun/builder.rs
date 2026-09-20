//! [`TrackRunBuilder`], the rows of one track run gathered before the header they fall under is known

use alloc::vec::Vec;

use crate::tfhd::TrackFragmentHeaderBox;
use crate::trun::{
    COMPOSITION_TIME_OFFSET_MAXIMUM, COMPOSITION_TIME_OFFSET_MINIMUM, TrackRunBox, TrackRunSample,
};

/// Composition time offset one of the two versions of a `trun` writes
///
/// Version 0 of the box writes the offset unsigned in 32 bits and version 1
/// signed (ISO/IEC 14496-12 §8.8.8), so a value between
/// `-2_147_483_648..=4_294_967_295` is one a row can carry, and this holds
/// such a value alone.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CompositionTimeOffset(i64);

impl CompositionTimeOffset {
    /// Creates the offset from its value
    ///
    /// Returns `None` when `offset` lies outside what either version writes.
    #[must_use]
    pub const fn new(offset: i64) -> Option<Self> {
        if offset < COMPOSITION_TIME_OFFSET_MINIMUM || offset > COMPOSITION_TIME_OFFSET_MAXIMUM {
            return None;
        }

        Some(Self(offset))
    }

    /// Returns the value of the offset
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// One sample of a run as it is handed to a [`TrackRunBuilder`], every field stated
///
/// A row of a [`TrackRunBox`] carries only the fields the header of its
/// fragment leaves unstated; this states every one, and the builder settles
/// which of them the row written carries.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TrackRunRow {
    sample_duration: u32,
    sample_size: u32,
    sample_flags: u32,
    sample_composition_time_offset: CompositionTimeOffset,
}

impl TrackRunRow {
    /// Creates the row from what the sample states
    #[must_use]
    pub const fn new(
        sample_duration: u32,
        sample_size: u32,
        sample_flags: u32,
        sample_composition_time_offset: CompositionTimeOffset,
    ) -> Self {
        Self {
            sample_duration,
            sample_size,
            sample_flags,
            sample_composition_time_offset,
        }
    }

    /// Returns how long the sample lasts, in the media time scale
    #[must_use]
    pub const fn sample_duration(&self) -> u32 {
        self.sample_duration
    }

    /// Returns how many bytes the sample occupies
    #[must_use]
    pub const fn sample_size(&self) -> u32 {
        self.sample_size
    }

    /// Returns the flags of the sample
    #[must_use]
    pub const fn sample_flags(&self) -> u32 {
        self.sample_flags
    }

    /// Returns the offset from the decode time of the sample to its composition time
    #[must_use]
    pub const fn sample_composition_time_offset(&self) -> CompositionTimeOffset {
        self.sample_composition_time_offset
    }
}

/// Gathers the rows of one track run, and writes the run against the header its fragment states
///
/// A [`TrackRunBox`] states once, for every row, which fields the rows carry,
/// and its version once for every composition time offset; a caller laying
/// samples down one at a time knows neither until the run is over. The
/// builder takes the rows with every field stated and settles both when the
/// run is [built](Self::build): a field the header states a default for, which
/// every row of the run agrees with, is left out of the rows, and the flags
/// only the first row differs on are its `first_sample_flags` (ISO/IEC
/// 14496-12 §8.8.8).
///
/// The two versions of the box write the offsets unsigned and signed, so one
/// run reaches either past [`i32::MAX`] or below zero — not both.
/// [`push`](Self::push) hands back a row the run cannot hold beside the ones it
/// has, which is where a caller starts the next run.
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{CompositionTimeOffset, TrackFragmentFlags, TrackFragmentHeaderBox, TrackRunBuilder, TrackRunRow, TrackRunSample};
///
/// // Two samples lasting 1024 units each, of different sizes
/// let offset = CompositionTimeOffset::new(0).unwrap();
/// let mut run = TrackRunBuilder::new(TrackRunRow::new(1_024, 4, 0, offset));
/// run.push(TrackRunRow::new(1_024, 2, 0, offset)).unwrap();
///
/// // Against a header stating the duration and the flags, only the size is written per row
/// let header = TrackFragmentHeaderBox::new(TrackFragmentFlags::ZERO, 1, None, None, Some(1_024), None, Some(0));
/// let track_run = run.build(Some(100), &header);
/// assert_eq!(
///     track_run.samples(),
///     [
///         TrackRunSample::new(None, Some(4), None, None).unwrap(),
///         TrackRunSample::new(None, Some(2), None, None).unwrap(),
///     ]
/// );
///
/// // A row whose offset no version writes beside the ones held is handed back
/// let mut signed = TrackRunBuilder::new(TrackRunRow::new(1_024, 4, 0, CompositionTimeOffset::new(-8).unwrap()));
/// let wide = TrackRunRow::new(1_024, 4, 0, CompositionTimeOffset::new(i64::from(u32::MAX)).unwrap());
/// assert_eq!(signed.push(wide), Err(wide));
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrackRunBuilder {
    rows: Vec<TrackRunRow>,
    holds_negative_offset: bool,
    holds_wide_offset: bool,
}

impl TrackRunBuilder {
    /// Starts a run with its first row
    #[must_use]
    pub fn new(first: TrackRunRow) -> Self {
        let mut run = Self {
            rows: Vec::new(),
            holds_negative_offset: false,
            holds_wide_offset: false,
        };
        run.hold(first);

        run
    }

    /// Adds `row` to the run, or hands it back when no version of the box writes its offset beside the ones held
    ///
    /// # Errors
    ///
    /// The row itself, when its composition time offset is negative while a
    /// row held reaches past [`i32::MAX`], or the other way round.
    pub fn push(&mut self, row: TrackRunRow) -> Result<(), TrackRunRow> {
        let offset = row.sample_composition_time_offset.get();
        if (offset.is_negative() && self.holds_wide_offset)
            || (offset > i64::from(i32::MAX) && self.holds_negative_offset)
        {
            return Err(row);
        }
        self.hold(row);

        Ok(())
    }

    /// Returns the rows of the run, in the order they were added
    #[must_use]
    pub fn rows(&self) -> &[TrackRunRow] {
        &self.rows
    }

    /// Builds the run as `tfhd` has it: the fields whose defaults every row agrees with are left out
    ///
    /// `data_offset` is what the run states for where its data lies. Flags
    /// that only the first row differs from the default on are written as its
    /// `first_sample_flags`; a composition time offset is written per row
    /// where any row states one other than zero.
    #[must_use]
    pub fn build(&self, data_offset: Option<i32>, tfhd: &TrackFragmentHeaderBox) -> TrackRunBox {
        let rows = || self.rows.iter();
        let differs = |default: Option<u32>, field: fn(&TrackRunRow) -> u32| {
            default.is_none_or(|default| rows().any(|row| field(row) != default))
        };
        let carries_duration =
            differs(tfhd.default_sample_duration(), TrackRunRow::sample_duration);
        let carries_size = differs(tfhd.default_sample_size(), TrackRunRow::sample_size);
        let carries_offsets = rows().any(|row| row.sample_composition_time_offset.get() != 0);
        let (carries_flags, first_sample_flags) = match tfhd.default_sample_flags() {
            Some(default) if rows().skip(1).all(|row| row.sample_flags == default) => (
                false,
                rows()
                    .next()
                    .map(|first| first.sample_flags)
                    .filter(|first| *first != default),
            ),
            _default_the_rows_do_not_share => (true, None),
        };

        let samples = rows()
            .map(|row| TrackRunSample {
                sample_duration: carries_duration.then_some(row.sample_duration),
                sample_size: carries_size.then_some(row.sample_size),
                sample_flags: carries_flags.then_some(row.sample_flags),
                sample_composition_time_offset: carries_offsets
                    .then_some(row.sample_composition_time_offset.get()),
            })
            .collect();

        TrackRunBox {
            data_offset,
            first_sample_flags,
            samples,
        }
    }

    /// Holds `row`, and notes where its composition time offset falls
    fn hold(&mut self, row: TrackRunRow) {
        let offset = row.sample_composition_time_offset.get();
        self.holds_negative_offset |= offset.is_negative();
        self.holds_wide_offset |= offset > i64::from(i32::MAX);
        self.rows.push(row);
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::{CompositionTimeOffset, TrackRunBuilder, TrackRunRow};
    use crate::tfhd::{TrackFragmentFlags, TrackFragmentHeaderBox};
    use crate::trun::{TrackRunBox, TrackRunSample};

    /// Row of a sample lasting 1024 units and occupying 4 bytes, flagged `sample_flags`, composed at `offset`
    fn row(sample_flags: u32, offset: i64) -> TrackRunRow {
        TrackRunRow::new(
            1_024,
            4,
            sample_flags,
            CompositionTimeOffset::new(offset).unwrap(),
        )
    }

    /// Header of track 1 stating the defaults given
    fn header(
        default_sample_duration: Option<u32>,
        default_sample_size: Option<u32>,
        default_sample_flags: Option<u32>,
    ) -> TrackFragmentHeaderBox {
        TrackFragmentHeaderBox::new(
            TrackFragmentFlags::ZERO,
            1,
            None,
            None,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        )
    }

    /// Run of the rows given, built against `header` with no data offset
    fn built(rows: &[TrackRunRow], header: &TrackFragmentHeaderBox) -> TrackRunBox {
        let (first, rest) = rows.split_first().unwrap();
        let mut run = TrackRunBuilder::new(*first);
        for row in rest {
            run.push(*row).unwrap();
        }

        run.build(None, header)
    }

    #[test]
    fn an_offset_outside_what_either_version_writes_is_refused() {
        assert_eq!(CompositionTimeOffset::new(i64::from(u32::MAX) + 1), None);
        assert_eq!(CompositionTimeOffset::new(i64::from(i32::MIN) - 1), None);
        assert_eq!(
            CompositionTimeOffset::new(-8).map(CompositionTimeOffset::get),
            Some(-8)
        );
    }

    #[test]
    fn a_row_no_version_writes_beside_the_rows_held_is_handed_back() {
        let wide = row(0, i64::from(u32::MAX));
        let negative = row(0, -8);
        let mut holding_negative = TrackRunBuilder::new(negative);
        let mut holding_wide = TrackRunBuilder::new(wide);

        assert_eq!(holding_negative.push(wide), Err(wide));
        assert_eq!(holding_wide.push(negative), Err(negative));
        assert_eq!(holding_negative.push(row(0, 8)), Ok(()));
        assert_eq!(holding_negative.rows(), [negative, row(0, 8)]);
    }

    #[test]
    fn the_fields_the_header_defaults_cover_are_left_out_of_the_rows() {
        let run = built(
            &[row(0, 0), row(0, 0)],
            &header(Some(1_024), Some(4), Some(0)),
        );

        assert_eq!(
            run,
            TrackRunBox::new(
                None,
                None,
                vec![TrackRunSample::new(None, None, None, None).unwrap(); 2]
            )
            .unwrap()
        );
    }

    #[test]
    fn a_field_a_row_differs_from_its_default_on_is_stated_by_every_row() {
        let run = built(&[row(0, 0), row(0, 0)], &header(Some(512), None, Some(0)));

        assert_eq!(
            run,
            TrackRunBox::new(
                None,
                None,
                vec![TrackRunSample::new(Some(1_024), Some(4), None, None).unwrap(); 2]
            )
            .unwrap()
        );
    }

    #[test]
    fn flags_only_the_first_row_differs_on_are_its_own() {
        let run = built(
            &[row(0x0200_0000, 0), row(0x0101_0000, 0)],
            &header(Some(1_024), Some(4), Some(0x0101_0000)),
        );

        assert_eq!(
            run,
            TrackRunBox::new(
                None,
                Some(0x0200_0000),
                vec![TrackRunSample::new(None, None, None, None).unwrap(); 2]
            )
            .unwrap()
        );
    }

    #[test]
    fn flags_a_later_row_differs_on_are_stated_by_every_row() {
        let run = built(
            &[row(0x0101_0000, 0), row(0x0200_0000, 0)],
            &header(Some(1_024), Some(4), Some(0x0101_0000)),
        );

        assert_eq!(
            run,
            TrackRunBox::new(
                None,
                None,
                vec![
                    TrackRunSample::new(None, None, Some(0x0101_0000), None).unwrap(),
                    TrackRunSample::new(None, None, Some(0x0200_0000), None).unwrap(),
                ]
            )
            .unwrap()
        );
    }

    #[test]
    fn an_offset_other_than_zero_has_every_row_state_one() {
        let run = built(
            &[row(0, 0), row(0, 8)],
            &header(Some(1_024), Some(4), Some(0)),
        );

        assert_eq!(
            run,
            TrackRunBox::new(
                None,
                None,
                vec![
                    TrackRunSample::new(None, None, None, Some(0)).unwrap(),
                    TrackRunSample::new(None, None, None, Some(8)).unwrap(),
                ]
            )
            .unwrap()
        );
    }
}
