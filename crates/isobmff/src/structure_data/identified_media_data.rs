//! [`IdentifiedMediaData`], where the identified media data of a file lie, and the samples that wait for one not yet read

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::Error;
use crate::SampleLocation;

/// Where the offset of a sample counts from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    /// The first byte of the file
    File,
    /// The first byte of the payload of the `imda` of this `imda_identifier`,
    /// ISO/IEC 14496-12 §8.7.2.1
    IdentifiedMediaData(u32),
}

/// The file offset of the payload of each `imda` read, and the samples whose offsets count from an `imda` not yet read
#[derive(Debug, Clone)]
pub(super) struct IdentifiedMediaData {
    limit: u64,
    /// File offset of the payload of each `imda` read, by its `imda_identifier`
    payloads: BTreeMap<u32, u64>,
    /// The samples whose offsets count from the payload of an `imda` not yet
    /// read, by the `imda_identifier` of that `imda`
    waiting: BTreeMap<u32, Vec<SampleLocation>>,
    // Why count the samples that wait rather than add up the lengths of
    // `waiting`: the count is checked before every box is resolved, and the
    // sum runs over every identifier waited for.
    waiting_count: u64,
    /// The samples of the box being resolved that wait for an `imda`, with
    /// its identifier, until the box is taken
    // Why hold them apart rather than in `waiting` at once: a box refused
    // leaves the samples that wait as they were.
    placed: Vec<(u32, SampleLocation)>,
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

    /// Appends `location`, whose offset counts from `origin`, to `locations` at its file offset, or holds it for the box being resolved until the `imda` it counts from is read
    ///
    /// # Errors
    /// * [`Error::Overflow`]: the sample lies past the last file offset.
    pub(super) fn place(
        &mut self,
        origin: Origin,
        location: SampleLocation,
        locations: &mut Vec<SampleLocation>,
    ) -> Result<(), Error> {
        match origin {
            Origin::File => locations.push(location),
            Origin::IdentifiedMediaData(imda_identifier) => {
                match self.payloads.get(&imda_identifier) {
                    Some(&payload_offset) => locations.push(at(payload_offset, location)?),
                    None => self.placed.push((imda_identifier, location)),
                }
            }
        }
        Ok(())
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
        let waited = self
            .waiting
            .get(&imda_identifier)
            .map_or(&[][..], Vec::as_slice);
        let start = locations.len();
        for &location in waited {
            match at(payload_offset, location) {
                Ok(location) => locations.push(location),
                // Why drop those taken: a sample past the last offset
                // refuses the `imda`, and the samples go on waiting.
                Err(error) => {
                    locations.truncate(start);
                    return Err(error);
                }
            }
        }
        if let Some(waited) = self.waiting.remove(&imda_identifier) {
            self.waiting_count = self
                .waiting_count
                .saturating_sub(u64::try_from(waited.len()).unwrap_or(u64::MAX));
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
