//! [`Index`], the extents a [`SampleReader`](super::SampleReader) holds once they fall out of the order of their bytes

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::ops::Range;

use super::PendingSample;
use crate::sample::Sample;

/// Extents a reader holds once they fall out of the order of their bytes, found by the byte each lacks next
///
/// `held` keeps the extents in the order they were held, an extent handed
/// over from behind a short one leaving its slot empty, and `front` is the
/// number the extent at its front was held as. `lacking` names each short extent by the byte it
/// lacks next and its number.
#[derive(Clone, Debug, Default)]
pub(super) struct Index {
    held: VecDeque<Option<PendingSample>>,
    front: u64,
    lacking: BTreeSet<(u64, u64)>,
}

impl Index {
    /// Holds `extents` behind those held, in the order they come
    pub(super) fn hold(&mut self, extents: impl IntoIterator<Item = PendingSample>) {
        for pending in extents {
            if !pending.is_whole() {
                // Why not checked_add: the number counts the extents held since
                // this index was made, which no input reaches 2^64 of.
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
        self.held.front().and_then(Option::as_ref)
    }

    /// Returns the extents held, in the order they were held
    #[cfg(test)]
    pub(super) fn held(&self) -> impl Iterator<Item = &PendingSample> {
        self.held.iter().flatten()
    }

    /// Fills the short extents whose next lacked byte lies in `data`, the bytes `arriving` covers, and hands over the samples this makes whole
    ///
    /// A sample `held_bytes_limit` refuses stops the fill there with the bytes
    /// it would have the reader hold, the samples made whole before it handed
    /// over.
    pub(super) fn fill(
        &mut self,
        data: &[u8],
        arriving: &Range<u64>,
        ready: &mut VecDeque<Sample>,
        held_bytes: &mut u64,
        held_bytes_limit: u64,
    ) -> Result<(), u64> {
        // Why not removing the keys while walking the range: the set cannot
        // change while the walk borrows it, and taking the first key of the
        // range afresh each time adds a search from the root per key on top of
        // the removal.
        let reached: Vec<(u64, u64)> = self
            .lacking
            .range((arriving.start, 0)..(arriving.end, 0))
            .copied()
            .collect();
        let mut made_whole = Vec::new();
        let mut refused = Ok(());
        for key in reached {
            self.lacking.remove(&key);
            let (_, number) = key;
            let Some(pending) = self.slot(number).and_then(Option::as_mut) else {
                continue;
            };
            if let Err(needed) = pending.take_from(data, arriving, held_bytes, held_bytes_limit) {
                self.lacking.insert(key);
                refused = Err(needed);
                break;
            }
            if pending.is_whole() {
                made_whole.push(number);
            } else {
                let lacked_next = pending.lacking().start;
                self.lacking.insert((lacked_next, number));
            }
        }
        if made_whole.is_empty() {
            return refused;
        }

        self.release_whole_front(ready);
        // Why not sort_unstable: the numbers come as ascending runs, one per
        // stretch of extents held in order that the input reaches, which a
        // stable sort merges where an unstable one sorts them afresh.
        made_whole.sort();
        for number in made_whole {
            if let Some(pending) = self.slot(number).and_then(Option::take) {
                ready.push_back(pending.into_sample());
            }
        }

        refused
    }

    /// Hands over the whole samples at the front of those held, in the order they were held
    pub(super) fn release_whole_front(&mut self, ready: &mut VecDeque<Sample>) {
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
