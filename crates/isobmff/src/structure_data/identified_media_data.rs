//! [`IdentifiedMediaData`], where the identified media data of a file lie, and the samples that wait for one not yet read

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::Error;
use crate::SampleLocation;

/// Where the offset of a sample counts from, which ISO/IEC 14496-12 §8.8.7.1 Table 6 may set apart from the `imda` the sample lies in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    /// The first byte of the file
    File,
    /// The first byte of the payload of the `imda` of this `imda_identifier`,
    /// ISO/IEC 14496-12 §8.7.2.1
    IdentifiedMediaData(u32),
}

/// A sample that waits for an `imda`
#[derive(Debug, Clone, Copy)]
struct Waiting {
    location: SampleLocation,
    /// Where the offset of `location` counts from
    origin: Origin,
    /// `imda_identifier` of the `imda` the sample lies in, which its data
    /// reference entry names
    imda_identifier: Option<u32>,
}

/// The file offset of the payload of each `imda` read, and the samples that wait for an `imda` not yet read
#[derive(Debug, Clone)]
pub(super) struct IdentifiedMediaData {
    limit: u64,
    /// File offset of the payload of each `imda` read, by its `imda_identifier`
    payloads: BTreeMap<u32, u64>,
    /// The samples that wait for an `imda` not yet read, by the
    /// `imda_identifier` of that `imda`: the one their offsets count from, or
    /// else the one they lie in
    waiting: BTreeMap<u32, Vec<Waiting>>,
    // Why count the samples that wait rather than add up the lengths of
    // `waiting`: the count is checked before every box is resolved, and the
    // sum runs over every identifier waited for.
    waiting_count: u64,
    /// The samples of the box being resolved that wait for an `imda`, with
    /// its identifier, until the box is taken
    // Why hold them apart rather than in `waiting` at once: a box refused
    // leaves the samples that wait as they were.
    placed: Vec<(u32, Waiting)>,
}

impl IdentifiedMediaData {
    /// No `imda` read, of at most `limit` to be remembered
    pub(super) const fn new(limit: u64) -> Self {
        Self {
            limit,
            payloads: BTreeMap::new(),
            waiting: BTreeMap::new(),
            waiting_count: 0,
            placed: Vec::new(),
        }
    }

    /// Number of the samples that wait for an `imda`
    pub(super) const fn waiting_count(&self) -> u64 {
        self.waiting_count
    }

    /// Appends `location`, whose offset counts from `origin`, to `locations` at its file offset once the `imda` it counts from, and the `imda` of `imda_identifier` it lies in, are read, or holds it for the box being resolved until they are
    ///
    /// # Errors
    /// * [`Error::Overflow`]: the sample lies past the last file offset.
    pub(super) fn place(
        &mut self,
        origin: Origin,
        imda_identifier: Option<u32>,
        location: SampleLocation,
        locations: &mut Vec<SampleLocation>,
    ) -> Result<(), Error> {
        let waiting = Waiting {
            location,
            origin,
            imda_identifier,
        };
        match self.resolved(waiting, None)? {
            Resolved::At(location) => locations.push(location),
            Resolved::Waits(waited, waiting) => self.placed.push((waited, waiting)),
        }
        Ok(())
    }

    /// Where `waiting` lies once the `imda` read, and that of `read` with its payload offset, are known
    fn resolved(&self, waiting: Waiting, read: Option<(u32, u64)>) -> Result<Resolved, Error> {
        let payload_offset = |identifier: u32| match read {
            Some((read, payload_offset)) if read == identifier => Some(payload_offset),
            Some(_) | None => self.payloads.get(&identifier).copied(),
        };
        let waiting = match waiting.origin {
            Origin::File => waiting,
            Origin::IdentifiedMediaData(origin) => match payload_offset(origin) {
                Some(payload_offset) => Waiting {
                    location: at(payload_offset, waiting.location)?,
                    origin: Origin::File,
                    ..waiting
                },
                None => return Ok(Resolved::Waits(origin, waiting)),
            },
        };
        Ok(
            match waiting
                .imda_identifier
                .filter(|&identifier| payload_offset(identifier).is_none())
            {
                Some(identifier) => Resolved::Waits(identifier, waiting),
                None => Resolved::At(waiting.location),
            },
        )
    }

    /// Ends the box being resolved: its samples held wait for their `imda` where it is `Ok`, and are dropped where it refuses the box
    pub(super) fn end_box<T>(&mut self, resolved: Result<T, Error>) -> Result<T, Error> {
        match resolved {
            Ok(resolved) => {
                self.waiting_count = self
                    .waiting_count
                    .saturating_add(u64::try_from(self.placed.len()).unwrap_or(u64::MAX));
                for (imda_identifier, location) in self.placed.drain(..) {
                    self.waiting
                        .entry(imda_identifier)
                        .or_default()
                        .push(location);
                }
                Ok(resolved)
            }
            Err(error) => {
                self.placed.clear();
                Err(error)
            }
        }
    }

    /// Remembers that the payload of the `imda` of `imda_identifier` starts at the file offset `payload_offset`, and appends the samples that wait for it to `locations` at their file offsets
    ///
    /// # Errors
    /// * [`Error::DuplicateImdaIdentifier`]: an `imda` of the same identifier
    ///   was read.
    /// * [`Error::IdentifiedMediaDataOverLimit`]: as many `imda` as the limit
    ///   were read.
    /// * [`Error::Overflow`]: a sample that waits for it lies past the last
    ///   file offset.
    ///
    /// Refused, it remembers nothing, and the samples go on waiting.
    pub(super) fn handle(
        &mut self,
        imda_identifier: u32,
        payload_offset: u64,
        locations: &mut Vec<SampleLocation>,
    ) -> Result<(), Error> {
        if self.payloads.contains_key(&imda_identifier) {
            return Err(Error::DuplicateImdaIdentifier { imda_identifier });
        }
        if u64::try_from(self.payloads.len()).unwrap_or(u64::MAX) >= self.limit {
            return Err(Error::IdentifiedMediaDataOverLimit {
                imda_identifier,
                limit: self.limit,
            });
        }
        let read = Some((imda_identifier, payload_offset));
        let start = locations.len();
        let mut still_waiting = Vec::new();
        for &waiting in self.waiting.get(&imda_identifier).into_iter().flatten() {
            match self.resolved(waiting, read) {
                Ok(Resolved::At(location)) => locations.push(location),
                Ok(Resolved::Waits(waited, waiting)) => still_waiting.push((waited, waiting)),
                // Why drop those taken: a sample past the last offset
                // refuses the `imda`, and the samples go on waiting.
                Err(error) => {
                    locations.truncate(start);
                    return Err(error);
                }
            }
        }
        if let Some(waited) = self.waiting.remove(&imda_identifier) {
            let taken = waited.len().saturating_sub(still_waiting.len());
            self.waiting_count = self
                .waiting_count
                .saturating_sub(u64::try_from(taken).unwrap_or(u64::MAX));
        }
        for (waited, waiting) in still_waiting {
            self.waiting.entry(waited).or_default().push(waiting);
        }
        self.payloads.insert(imda_identifier, payload_offset);
        Ok(())
    }

    /// Drops the samples that wait for an `imda`, the input having ended
    ///
    /// # Errors
    /// * [`Error::IdentifiedMediaDataNotFound`]: samples wait for an `imda`;
    ///   the error names the least identifier among those they wait for.
    pub(super) fn finish(&mut self) -> Result<(), Error> {
        let Some((&imda_identifier, waited)) = self.waiting.first_key_value() else {
            return Ok(());
        };
        let sample_count = u64::try_from(waited.len()).unwrap_or(u64::MAX);
        self.waiting.clear();
        self.waiting_count = 0;
        Err(Error::IdentifiedMediaDataNotFound {
            imda_identifier,
            sample_count,
        })
    }
}

/// Where a sample lies, once the `imda` read are known
enum Resolved {
    /// At this file offset
    At(SampleLocation),
    /// Not known until the `imda` of this `imda_identifier` is read
    Waits(u32, Waiting),
}

/// `location`, whose offset counts from the payload at the file offset `payload_offset`, at its file offset
fn at(payload_offset: u64, location: SampleLocation) -> Result<SampleLocation, Error> {
    let offset = payload_offset
        .checked_add(location.offset)
        .filter(|offset| offset.checked_add(u64::from(location.size)).is_some())
        .ok_or(Error::Overflow {
            track_id: location.properties.track_id,
        })?;
    Ok(SampleLocation { offset, ..location })
}
