//! [`Samples`], the locations of the samples the demuxer has read, and which of them it wants the bytes of

use alloc::vec::Vec;

use crate::SampleLocation;

/// The samples of the `moov` and of the `moof`s read, and how far they are reported and handed over to be gathered
#[derive(Debug, Clone, Default)]
pub(super) struct Samples {
    /// The location of each sample, in the order of their file offsets
    locations: Vec<SampleLocation>,
    /// How many of `locations` were reported
    reported: usize,
    /// Index of the first location whose bytes may still be requested
    next: usize,
    /// `track_ID` of each track whose samples are lent
    subscribed: Vec<u32>,
}

impl Samples {
    /// Lends the bytes of the samples of track `track_id` from now on
    pub(super) fn subscribe(&mut self, track_id: u32) {
        if !self.subscribed.contains(&track_id) {
            self.subscribed.push(track_id);
        }
    }

    /// Stops lending the bytes of the samples of track `track_id`
    pub(super) fn unsubscribe(&mut self, track_id: u32) {
        self.subscribed.retain(|&subscribed| subscribed != track_id);
    }

    /// The location at `index`
    pub(super) fn location(&self, index: usize) -> Option<SampleLocation> {
        self.locations.get(index).copied()
    }

    /// The first location not yet reported
    pub(super) fn unreported(&self) -> Option<SampleLocation> {
        self.location(self.reported)
    }

    /// Whether every location is reported
    pub(super) fn is_reported(&self) -> bool {
        self.reported == self.locations.len()
    }

    /// Counts the first location not yet reported as reported
    pub(super) fn report(&mut self) {
        self.reported = self.reported.saturating_add(1);
        // Why sort once every location is reported: the samples of an earlier
        // box still wanted may lie after those of a `moof`, and samples are
        // requested in file order.
        if self.is_reported() {
            self.locations.sort_by_key(|location| location.offset);
        }
    }

    /// Replaces the locations with those of a `moov`
    pub(super) fn replace(&mut self, locations: impl IntoIterator<Item = SampleLocation>) {
        self.locations.clear();
        self.locations.extend(locations);
    }

    /// Adds the locations of a `moof`
    pub(super) fn extend(&mut self, locations: impl IntoIterator<Item = SampleLocation>) {
        self.locations.extend(locations);
    }

    /// Drops the locations of the samples lent, before the `moof` at file offset `offset`, or not in this file as `is_in_this_file` tells, and counts those left as reported
    pub(super) fn drop_before(
        &mut self,
        offset: u64,
        is_in_this_file: impl Fn(&SampleLocation) -> bool,
    ) {
        // Why drop the locations before the `moof`: a sample of a subscribed
        // track before it was lent before the box reader passed it, and the
        // locations would otherwise grow with every fragment.
        // Why drop those in another file whatever their offset: their offset
        // is one of that file, not comparable with the `moof`'s, and their
        // bytes are never requested once their locations are reported.
        self.locations.drain(..self.next);
        self.locations
            .retain(|location| location.offset >= offset && is_in_this_file(location));
        self.next = 0;
        self.reported = self.locations.len();
    }

    /// How many locations are held
    pub(super) fn held(&self) -> u64 {
        length_of(self.locations.len())
    }

    /// The next sample of a subscribed track, in this file as `is_in_this_file` tells, whose bytes are wanted before the file offset `before` the box reader wants, with its index
    pub(super) fn target(
        &self,
        before: Option<u64>,
        is_in_this_file: impl Fn(&SampleLocation) -> bool,
    ) -> Option<(usize, SampleLocation)> {
        // Why also before the end of the input when the box reader wants
        // none: an `mdat` of size 0 runs to the end of the file, and its
        // samples are wanted though no box follows it.
        let before = before.unwrap_or(u64::MAX);
        let start = self.next;
        self.locations
            .get(start..)?
            .iter()
            .enumerate()
            // Why also a sample of no bytes at the position the box reader
            // wants: it lies at the end of the media data before that box, or
            // of the input, and needs no byte of either.
            .take_while(|(_, location)| {
                location.offset < before || (location.size == 0 && location.offset == before)
            })
            .find(|(_, location)| {
                self.subscribed.contains(&location.properties.track_id) && is_in_this_file(location)
            })
            .map(|(index, &location)| (start.saturating_add(index), location))
    }

    /// Requests no bytes of the samples up to the one at `index`
    pub(super) fn pass(&mut self, index: usize) {
        self.next = index.saturating_add(1);
    }
}

fn length_of(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}
