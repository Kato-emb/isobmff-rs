//! [`Index`], the extents a [`SampleReader`](super::SampleReader) holds while they are out of the order of their bytes

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::ops::Range;

use super::PendingSample;
use crate::sample::Sample;

/// Extents held out of the order of their bytes, found by the byte each lacks next
///
/// `held` keeps the extents in the order they were held, an extent handed
/// over leaving an empty slot behind it, and `front` is the number the extent
/// at its front was held as. `lacking` names each short extent by the byte it
/// lacks next and its number.
#[derive(Clone, Debug)]
pub(super) struct Index {
    held: VecDeque<Option<PendingSample>>,
    front: u64,
    lacking: BTreeSet<(u64, u64)>,
}

impl Index {
    /// Indexes `held`, the extents in the order they were held
    pub(super) fn new(held: VecDeque<PendingSample>) -> Self {
        let mut index = Self {
            held: VecDeque::with_capacity(held.len()),
            front: 0,
            lacking: BTreeSet::new(),
        };
        index.hold(held);

        index
    }

    /// Holds `extents` behind those held, in the order they come
    pub(super) fn hold(&mut self, extents: impl IntoIterator<Item = PendingSample>) {
        for pending in extents {
            if !pending.is_whole() {
                // Why not checked_add: the number counts the extents one reader
                // has held, which no input reaches 2^64 of.
                let number = self.front.wrapping_add(self.held.len() as u64);
                self.lacking.insert((pending.lacking().start, number));
            }
            self.held.push_back(Some(pending));
        }
    }

    /// Returns whether an extent held is short of its bytes
    pub(super) fn holds_short(&self) -> bool {
        !self.lacking.is_empty()
    }

    /// Returns the extent at the front of those held
    pub(super) fn front(&self) -> Option<&PendingSample> {
        self.held.iter().flatten().next()
    }

    /// Returns the extents held, in the order they were held
    #[cfg(test)]
    pub(super) fn held(&self) -> impl Iterator<Item = &PendingSample> {
        self.held.iter().flatten()
    }

    /// Hands back the extents held, in the order they were held
    pub(super) fn into_held(self) -> VecDeque<PendingSample> {
        self.held.into_iter().flatten().collect()
    }

    /// Fills the short extents `data`, the bytes `arriving` covers, carries the next byte of, and hands over the samples that makes whole
    pub(super) fn fill(
        &mut self,
        data: &[u8],
        arriving: &Range<u64>,
        ready: &mut VecDeque<Sample>,
    ) {
        // Why not taking them off the set one at a time: an extent the input
        // could not fill would be put back where it was and found again.
        let reached: Vec<(u64, u64)> = self
            .lacking
            .range((arriving.start, 0)..(arriving.end, 0))
            .copied()
            .collect();
        let mut made_whole = Vec::new();
        for key in reached {
            self.lacking.remove(&key);
            let (_, number) = key;
            let Some(pending) = self.slot(number).and_then(Option::as_mut) else {
                continue;
            };
            pending.take_from(data, arriving);
            if pending.is_whole() {
                made_whole.push(number);
            } else {
                let lacked_next = pending.lacking().start;
                self.lacking.insert((lacked_next, number));
            }
        }
        if made_whole.is_empty() {
            return;
        }

        self.report_front(ready);
        made_whole.sort_unstable();
        for number in made_whole {
            if let Some(pending) = self.slot(number).and_then(Option::take) {
                ready.push_back(pending.into_sample());
            }
        }
    }

    /// Hands over the whole samples at the front of those held, in the order they were held
    pub(super) fn report_front(&mut self, ready: &mut VecDeque<Sample>) {
        while let Some(front) = self.held.front() {
            if front.as_ref().is_some_and(|pending| !pending.is_whole()) {
                break;
            }
            if let Some(pending) = self.held.pop_front().flatten() {
                ready.push_back(pending.into_sample());
            }
            self.front = self.front.wrapping_add(1);
        }
    }

    /// Returns the slot of the extent held as `number`, which is empty once it was handed over
    fn slot(&mut self, number: u64) -> Option<&mut Option<PendingSample>> {
        let position = usize::try_from(number.wrapping_sub(self.front)).ok()?;

        self.held.get_mut(position)
    }
}
