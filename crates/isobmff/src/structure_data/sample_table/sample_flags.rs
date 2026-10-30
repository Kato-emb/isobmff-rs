//! The tables of a sample table that state the `sample_flags` of each sample, ISO/IEC 14496-12 §8.6.2, §8.6.4, §8.7.6 and §8.5.3: read into the flags, and written of them

use core::iter::{Copied, Peekable};
use core::slice;

use isobmff_boxes::{
    DegradationPriorityBox, PaddingBitsBox, SampleDependencyTypeBox, SampleDependencyTypeEntry,
    SampleFlags, SampleTableBox, SyncSampleBox,
};
use isobmff_core::{BoxDefinition, BoxType};

use super::next_or_zero;
use crate::framing::to_u64;

/// What the `sdtp`, the `padb`, the `stdp` and the `stss` of a sample table state of the samples not read yet, in sample order
pub(super) struct SampleFlagsTables<'stbl> {
    dependencies: Option<Copied<slice::Iter<'stbl, SampleDependencyTypeEntry>>>,
    paddings: Option<Copied<slice::Iter<'stbl, u8>>>,
    priorities: Option<Copied<slice::Iter<'stbl, u16>>>,
    sync_samples: Option<Peekable<Copied<slice::Iter<'stbl, u32>>>>,
    /// Number of the last sample read
    sample_number: u64,
    /// Number of the sample numbers the `stss` marks out of order, among those read past
    unordered: u64,
}

impl<'stbl> SampleFlagsTables<'stbl> {
    /// Returns the tables of `stbl`, none of their samples read yet
    pub(super) fn new(stbl: &'stbl SampleTableBox) -> Self {
        Self {
            dependencies: stbl.sdtp.as_ref().map(|sdtp| sdtp.entries.iter().copied()),
            paddings: stbl.padb.as_ref().map(|padb| padb.pads.iter().copied()),
            priorities: stbl
                .stdp
                .as_ref()
                .map(|stdp| stdp.priorities.iter().copied()),
            sync_samples: stbl
                .stss
                .as_ref()
                .map(|stss| stss.sample_numbers.iter().copied().peekable()),
            sample_number: 0,
            unordered: 0,
        }
    }

    /// Returns the `sample_flags` of the next sample, as §8.8.3.1 lays them out
    ///
    /// The `sdtp` gives the dependencies of the sample (§8.6.4), the `padb` the
    /// bits it is padded with (§8.7.6), the `stdp` its degradation priority
    /// (§8.5.3), and the `stss` whether it is a sync sample, every sample being
    /// one where the track has no `stss` (§8.6.2). A missing table, or one
    /// that has run out of samples, leaves its fields zero. A sample number
    /// the `stss` marks out of order is passed over.
    pub(super) fn next_flags(&mut self) -> SampleFlags {
        let dependency = next_or_zero(&mut self.dependencies);
        let sample_padding_value = next_or_zero(&mut self.paddings);
        let sample_degradation_priority = next_or_zero(&mut self.priorities);
        self.sample_number = self.sample_number.saturating_add(1);
        let sample_number = self.sample_number;
        let is_sync_sample = self.sync_samples.as_mut().is_none_or(|marked| {
            while marked
                .next_if(|marked| u64::from(*marked) < sample_number)
                .is_some()
            {
                self.unordered = self.unordered.saturating_add(1);
            }
            marked
                .next_if(|marked| u64::from(*marked) == sample_number)
                .is_some()
        });

        let mut flags = SampleFlags::default();
        flags.is_leading = dependency.is_leading;
        flags.sample_depends_on = dependency.sample_depends_on;
        flags.sample_is_depended_on = dependency.sample_is_depended_on;
        flags.sample_has_redundancy = dependency.sample_has_redundancy;
        flags.sample_padding_value = sample_padding_value;
        flags.sample_is_non_sync_sample = !is_sync_sample;
        flags.sample_degradation_priority = sample_degradation_priority;
        flags
    }

    /// Returns how the `stss` disagrees with the samples read, the number of samples marked past the last of them and the number marked out of order
    pub(super) fn marked_out_of_place(self) -> (u64, u64) {
        let sample_number = self.sample_number;
        self.sync_samples.into_iter().flatten().fold(
            (0, self.unordered),
            |(past, unordered), marked| {
                if u64::from(marked) > sample_number {
                    (past.saturating_add(1), unordered)
                } else {
                    (past, unordered.saturating_add(1))
                }
            },
        )
    }
}

/// Returns the type of each table of a value per sample `stbl` has, among the `sdtp`, the `padb` and the `stdp`, and the number of samples it states
pub(super) fn stated_samples(stbl: &SampleTableBox) -> [(BoxType, Option<u64>); 3] {
    [
        (
            SampleDependencyTypeBox::BOX_TYPE,
            stbl.sdtp.as_ref().map(|sdtp| to_u64(sdtp.entries.len())),
        ),
        (
            PaddingBitsBox::BOX_TYPE,
            stbl.padb.as_ref().map(|padb| to_u64(padb.pads.len())),
        ),
        (
            DegradationPriorityBox::BOX_TYPE,
            stbl.stdp.as_ref().map(|stdp| to_u64(stdp.priorities.len())),
        ),
    ]
}

/// Fills the `stss`, the `sdtp`, the `padb` and the `stdp` of `stbl` with `flags`, the `sample_flags` of each sample in sample order
///
/// The `stss` is left out where every sample is a sync sample, the `sdtp`
/// where no sample states a dependency, the `padb` where no sample is padded,
/// and the `stdp` where every degradation priority is zero (§8.6.2, §8.6.4,
/// §8.7.6, §8.5.3).
pub(super) fn fill(flags: impl Iterator<Item = SampleFlags> + Clone, stbl: &mut SampleTableBox) {
    stbl.stss = flags
        .clone()
        .any(|flags| flags.sample_is_non_sync_sample)
        .then(|| {
            let mut stss = SyncSampleBox::default();
            stss.sample_numbers = (1_u32..)
                .zip(flags.clone())
                .filter(|(_, flags)| !flags.sample_is_non_sync_sample)
                .map(|(sample_number, _)| sample_number)
                .collect();
            stss
        });

    let dependency = |flags: SampleFlags| {
        let mut entry = SampleDependencyTypeEntry::default();
        entry.is_leading = flags.is_leading;
        entry.sample_depends_on = flags.sample_depends_on;
        entry.sample_is_depended_on = flags.sample_is_depended_on;
        entry.sample_has_redundancy = flags.sample_has_redundancy;
        entry
    };
    stbl.sdtp = flags
        .clone()
        .any(|flags| dependency(flags) != SampleDependencyTypeEntry::default())
        .then(|| {
            let mut sdtp = SampleDependencyTypeBox::default();
            sdtp.entries = flags.clone().map(dependency).collect();
            sdtp
        });

    stbl.padb = flags
        .clone()
        .any(|flags| flags.sample_padding_value != 0)
        .then(|| {
            let mut padb = PaddingBitsBox::default();
            padb.pads = flags
                .clone()
                .map(|flags| flags.sample_padding_value)
                .collect();
            padb
        });

    stbl.stdp = flags
        .clone()
        .any(|flags| flags.sample_degradation_priority != 0)
        .then(|| {
            let mut stdp = DegradationPriorityBox::default();
            stdp.priorities = flags
                .map(|flags| flags.sample_degradation_priority)
                .collect();
            stdp
        });
}
