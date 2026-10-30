//! Samples gathered out of the media data of a file as its bytes arrive
//!
//! Media data is the bytes of samples, which structure-data describes by
//! reference (ISO/IEC 14496-12 §8.1.1, A.3). A [`MediaDataReader`] takes the
//! [`SampleLocation`]s structure-data resolves to, and the bytes of the file
//! as they arrive, and gathers each sample out of them.

mod writer;

use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::ops::Range;

use crate::byte_range::ByteRange;
use crate::error::DemuxError;
use crate::framing::to_u64;
use crate::sample::{Sample, SampleLocation};

pub(crate) use writer::MediaDataWriter;

/// Limits a [`MediaDataReader`] holds sample locations and their media data up to
///
/// Each bounds what the reader allocates: the number of locations it holds,
/// the media data of one sample, and the media data allocated for the samples
/// not yet taken, together.
///
/// # Examples
///
/// ```
/// use isobmff::media_data::MediaDataLimits;
///
/// let limits = MediaDataLimits {
///     held_location_count: 1_048_576,
///     sample_bytes: 16 * 1024 * 1024,
///     held_bytes: 16 * 1024 * 1024,
/// };
/// assert_eq!(limits.held_location_count, 1_048_576);
/// ```
#[expect(
    clippy::exhaustive_structs,
    reason = "a limit added later is one every caller has to set, not one that quietly takes a default"
)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MediaDataLimits {
    /// Number of locations the reader holds up to
    pub held_location_count: u64,
    /// Bytes of media data of one sample the reader holds up to
    pub sample_bytes: u64,
    /// Bytes of media data the reader holds allocated for the samples not yet taken up to, together
    pub held_bytes: u64,
}

/// Reader of the media data of a file: the bytes of the samples whose locations it holds, gathered as they arrive
///
/// The reader takes bytes from any file offset, in any order, cut anywhere,
/// and more than once: of each piece of input it keeps the bytes that a
/// location it holds names, and drops the rest. Bytes that arrive before the
/// location that names them are not kept for it. Two locations that name the
/// same bytes are gathered apart, each into its own sample.
///
/// What the reader holds is bounded by the number of locations, and by the
/// media data allocated for them. A location counts from the call that hands
/// it over until [`poll_sample`](Self::poll_sample) takes its sample. The
/// media data of a sample is allocated, as long as its location says, when the
/// first byte of it arrives, and counts until its sample is taken. A location
/// that names more media data than one sample may hold is refused as it is
/// handed over. The input a
/// sample's allocation would take past the limit is not kept for it, nor for
/// the samples held after it, and is wanted again: the caller takes samples
/// with `poll_sample` and passes it again.
///
/// A refused call leaves the reader as it was.
///
/// # Examples
///
/// ```
/// use isobmff::boxes::SampleFlags;
/// use isobmff::media_data::{MediaDataLimits, MediaDataReader};
/// use isobmff::{Sample, SampleLocation, SampleProperties};
///
/// let properties = SampleProperties {
///     track_id: 1,
///     decoding_time: 0,
///     sample_duration: 1024,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// let mut reader = MediaDataReader::new(MediaDataLimits {
///     held_location_count: 1024,
///     sample_bytes: 1024 * 1024,
///     held_bytes: 16 * 1024 * 1024,
/// });
///
/// // The media data of the sample is the four bytes at file offset 100
/// reader.handle_sample_locations([SampleLocation::new(properties, 1, 100..104)])?;
///
/// // Its bytes arrive in two pieces, the later one first
/// reader.handle_input(102, b"CDEF")?;
/// assert_eq!(reader.poll_sample(), None);
/// reader.handle_input(98, b"..AB")?;
/// assert_eq!(reader.poll_sample(), Some(Sample::new(properties, b"ABCD".to_vec())));
/// reader.finish()?;
/// # Ok::<(), isobmff::DemuxError>(())
/// ```
#[derive(Debug)]
pub struct MediaDataReader {
    limits: MediaDataLimits,
    /// Locations held, in the order they were handed over
    held: VecDeque<HeldLocation>,
    /// `track_ID`s of the tracks whose locations are not held
    discarded_tracks: BTreeSet<u32>,
}

/// Location the reader holds, and as much of its media data as has arrived
#[derive(Debug)]
struct HeldLocation {
    location: SampleLocation,
    /// Media data of the sample, empty until a byte of it arrives, then as long as its byte range
    data: Vec<u8>,
    /// File offsets of the bytes of the sample that have not arrived, in the order they lie, none of them empty
    missing: Vec<Range<u64>>,
}

impl MediaDataReader {
    /// Creates the reader that holds sample locations up to `limits`
    ///
    /// On a target whose memory addresses fewer bytes than
    /// `limits.sample_bytes` or `limits.held_bytes`, the reader holds up to
    /// what it addresses. One sample is held up to `limits.held_bytes` when
    /// `limits.sample_bytes` is more, as a longer one could never be
    /// allocated.
    #[must_use]
    pub fn new(limits: MediaDataLimits) -> Self {
        let held_bytes = limits
            .held_bytes
            .min(u64::try_from(isize::MAX).unwrap_or(u64::MAX));
        Self {
            limits: MediaDataLimits {
                sample_bytes: limits.sample_bytes.min(held_bytes),
                held_bytes,
                ..limits
            },
            held: VecDeque::new(),
            discarded_tracks: BTreeSet::new(),
        }
    }

    /// Holds `locations`, behind those held before them, until their samples are taken
    ///
    /// # Errors
    ///
    /// * [`HeldLocationLimitExceeded`](DemuxError::HeldLocationLimitExceeded):
    ///   holding the locations would take those held past the limit, and none
    ///   of them is held.
    /// * [`SampleBytesLimitExceeded`](DemuxError::SampleBytesLimitExceeded):
    ///   a location names more media data than the limit of one sample, and
    ///   none of them is held.
    pub fn handle_sample_locations(
        &mut self,
        locations: impl IntoIterator<Item = SampleLocation>,
    ) -> Result<(), DemuxError> {
        let held_before = self.held.len();
        for location in locations.into_iter().filter(|location| {
            !self
                .discarded_tracks
                .contains(&location.properties().track_id)
        }) {
            let byte_range = location.byte_range();
            let held_locations = to_u64(self.held.len()).saturating_add(1);
            let declared_bytes = length(&byte_range);
            let refused = if held_locations > self.limits.held_location_count {
                Some(DemuxError::HeldLocationLimitExceeded {
                    held_locations,
                    limit_locations: self.limits.held_location_count,
                })
            } else if declared_bytes > self.limits.sample_bytes {
                Some(DemuxError::SampleBytesLimitExceeded {
                    declared_bytes,
                    limit_bytes: self.limits.sample_bytes,
                })
            } else {
                None
            };
            if let Some(error) = refused {
                self.held.truncate(held_before);
                return Err(error);
            }
            self.held.push_back(HeldLocation {
                location,
                data: Vec::new(),
                missing: [byte_range]
                    .into_iter()
                    .filter(|range| !range.is_empty())
                    .collect(),
            });
        }

        Ok(())
    }

    /// Takes the bytes of `input`, which starts at file offset `offset`, that the held locations name
    ///
    /// The media data of a sample is allocated when the first byte of it
    /// arrives, in the order the locations are held, while the media data
    /// allocated for the samples not yet taken stays within the limit. The
    /// bytes of the sample whose allocation would not, and of the samples held
    /// after it that have no allocation yet, are not kept: they are wanted
    /// again.
    ///
    /// # Errors
    ///
    /// * [`HeldBytesLimitExceeded`](DemuxError::HeldBytesLimitExceeded): the
    ///   input names bytes of a sample whose allocation would take the media
    ///   data allocated past the limit, and none of its bytes is kept. The
    ///   reader is left as it was: the samples whose bytes have arrived are to
    ///   be taken with [`poll_sample`](Self::poll_sample) before the input is
    ///   passed again.
    pub fn handle_input(&mut self, offset: u64, input: &[u8]) -> Result<(), DemuxError> {
        let input_range = offset..offset.saturating_add(to_u64(input.len()));
        let mut allocated_bytes = self
            .held
            .iter()
            .map(|held| to_u64(held.data.len()))
            .fold(0, u64::saturating_add);
        let mut refused = None;
        let mut kept = false;
        for held in &mut self.held {
            let reached = held
                .missing
                .iter()
                .any(|missing| missing.start < input_range.end && input_range.start < missing.end);
            if !reached {
                continue;
            }
            if held.data.is_empty() {
                // Why not allocate a later sample once one is refused: it would hold the bytes the earlier one needs, and samples are taken in the order they are held.
                if refused.is_some() {
                    continue;
                }
                let held_bytes =
                    allocated_bytes.saturating_add(length(&held.location.byte_range()));
                if held_bytes > self.limits.held_bytes {
                    refused = Some(DemuxError::HeldBytesLimitExceeded {
                        held_bytes,
                        limit_bytes: self.limits.held_bytes,
                    });
                    continue;
                }
                allocated_bytes = held_bytes;
            }
            held.fill(&input_range, input);
            kept = true;
        }

        match refused {
            Some(refused) if !kept => Err(refused),
            Some(_) | None => Ok(()),
        }
    }

    /// Returns the sample of the location held first, once its bytes have all arrived
    pub fn poll_sample(&mut self) -> Option<Sample> {
        if !self.held.front()?.missing.is_empty() {
            return None;
        }
        let held = self.held.pop_front()?;

        Some(Sample::new(*held.location.properties(), held.data))
    }

    /// Returns the run of bytes that a held location lacks and that lies first in the file, or `None` when none lacks any
    #[must_use]
    pub fn wanted_input(&self) -> Option<ByteRange> {
        self.held
            .iter()
            .filter_map(|held| held.missing.first())
            .min_by_key(|missing| missing.start)
            .map(|missing| ByteRange::new(missing.start, Some(length(missing))))
    }

    /// Declares that no more input arrives, which the bytes of every held location must have
    ///
    /// The reader is left as it was: the samples whose bytes have arrived are
    /// still taken with [`poll_sample`](Self::poll_sample).
    ///
    /// # Errors
    ///
    /// * [`UnfinishedSample`](DemuxError::UnfinishedSample): a held location,
    ///   the first of those that do, lacks bytes.
    pub fn finish(&self) -> Result<(), DemuxError> {
        match self.held.iter().find(|held| !held.missing.is_empty()) {
            Some(held) => {
                let needed_bytes = length(&held.location.byte_range());
                Err(DemuxError::UnfinishedSample {
                    track_id: held.location.properties().track_id,
                    needed_bytes,
                    available_bytes: needed_bytes
                        .saturating_sub(held.missing.iter().map(length).sum()),
                })
            }
            None => Ok(()),
        }
    }

    /// Drops every held location and the bytes gathered for them, keeping the limits
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Drops the held locations of the samples of track `track_id` and the bytes gathered for them, and holds none of the track later
    pub(crate) fn discard_track(&mut self, track_id: u32) {
        self.discarded_tracks.insert(track_id);
        self.held
            .retain(|held| held.location.properties().track_id != track_id);
    }
}

impl HeldLocation {
    /// Copies the bytes of `input`, which lies at `input_range`, that the sample lacks into its media data
    fn fill(&mut self, input_range: &Range<u64>, input: &[u8]) {
        let byte_range = self.location.byte_range();
        let mut index = 0;
        while let Some(missing) = self.missing.get(index).cloned() {
            let start = missing.start.max(input_range.start);
            let end = missing.end.min(input_range.end);
            if start >= end {
                index = index.saturating_add(1);
                continue;
            }
            if self.data.is_empty() {
                // Why not allocate as the location is held: a location the input never reaches would take its bytes for nothing.
                self.data.resize(to_index(length(&byte_range)), 0);
            }
            // Why not check these conversions: start..end lies inside both the input and the byte range of the sample, whose data is as long as that range.
            let from = to_index(start.saturating_sub(input_range.start));
            let to = to_index(start.saturating_sub(byte_range.start));
            let copied = to_index(end.saturating_sub(start));
            if let (Some(source), Some(target)) = (
                input.get(from..from.saturating_add(copied)),
                self.data.get_mut(to..to.saturating_add(copied)),
            ) {
                target.copy_from_slice(source);
            }
            let rest = [missing.start..start, end..missing.end]
                .into_iter()
                .filter(|piece| !piece.is_empty());
            let kept = rest.clone().count();
            self.missing.splice(index..=index, rest);
            index = index.saturating_add(kept);
        }
    }
}

/// Returns the number of bytes `range` runs over
fn length(range: &Range<u64>) -> u64 {
    range.end.saturating_sub(range.start)
}

/// Converts a distance within bytes held in memory to an index into them
fn to_index(distance: u64) -> usize {
    usize::try_from(distance).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::SampleFlags;

    use super::{MediaDataLimits, MediaDataReader};
    use crate::byte_range::ByteRange;
    use crate::error::DemuxError;
    use crate::sample::{Sample, SampleLocation, SampleProperties};

    /// Bytes of a file whose byte at each file offset is that offset, modulo 256
    fn file(range: Range<u64>) -> Vec<u8> {
        range
            .map(|offset| u8::try_from(offset % 256).unwrap())
            .collect()
    }

    /// Returns the values declared about sample `decoding_time` of `track_id`
    fn properties(track_id: u32, decoding_time: u64) -> SampleProperties {
        SampleProperties {
            track_id,
            decoding_time,
            sample_duration: 1,
            sample_composition_time_offset: 0,
            sample_flags: SampleFlags::default(),
            sample_description_index: 1,
        }
    }

    /// Returns the location of sample `decoding_time` of track 1, at `byte_range` of the file itself
    fn location(decoding_time: u64, byte_range: Range<u64>) -> SampleLocation {
        SampleLocation::new(properties(1, decoding_time), 1, byte_range)
    }

    /// Returns the sample `location` names, with the bytes [`file`] has there
    fn sample(location: &SampleLocation) -> Sample {
        Sample::new(*location.properties(), file(location.byte_range()))
    }

    /// Takes every sample whose bytes have arrived
    fn poll_all(reader: &mut MediaDataReader) -> Vec<Sample> {
        core::iter::from_fn(|| reader.poll_sample()).collect()
    }

    /// Reader with no limit to speak of
    fn unlimited() -> MediaDataReader {
        MediaDataReader::new(MediaDataLimits {
            held_location_count: u64::MAX,
            sample_bytes: u64::MAX,
            held_bytes: u64::MAX,
        })
    }

    #[test]
    fn samples_gather_from_input_cut_across_their_boundaries() {
        let locations = [
            location(0, 10..13),
            location(1, 13..20),
            location(2, 20..21),
        ];
        let mut reader = unlimited();
        reader.handle_sample_locations(locations.clone()).unwrap();

        let mut samples = Vec::new();
        for start in (8_u64..24).step_by(4) {
            reader.handle_input(start, &file(start..start + 4)).unwrap();
            samples.extend(poll_all(&mut reader));
        }

        assert_eq!(samples, locations.iter().map(sample).collect::<Vec<_>>());
        assert_eq!(reader.wanted_input(), None);
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn samples_come_out_in_the_order_of_their_locations_whatever_order_the_bytes_arrive_in() {
        let locations = [
            location(0, 30..40),
            location(1, 10..20),
            location(2, 20..30),
        ];
        let mut reader = unlimited();
        reader.handle_sample_locations(locations.clone()).unwrap();

        reader.handle_input(25, &file(25..30)).unwrap();
        reader.handle_input(10, &file(10..20)).unwrap();
        assert_eq!(poll_all(&mut reader), []);
        reader.handle_input(35, &file(35..40)).unwrap();
        reader.handle_input(20, &file(20..25)).unwrap();
        reader.handle_input(30, &file(30..35)).unwrap();

        assert_eq!(
            poll_all(&mut reader),
            locations.iter().map(sample).collect::<Vec<_>>()
        );
    }

    #[test]
    fn bytes_that_arrive_twice_gather_as_once() {
        let location = location(0, 10..20);
        let mut reader = unlimited();
        reader.handle_sample_locations([location.clone()]).unwrap();

        reader.handle_input(12, &file(12..16)).unwrap();
        reader.handle_input(10, &file(10..14)).unwrap();
        reader.handle_input(12, &file(12..16)).unwrap();
        assert_eq!(reader.poll_sample(), None);
        reader.handle_input(10, &file(10..20)).unwrap();

        assert_eq!(poll_all(&mut reader), [sample(&location)]);
    }

    #[test]
    fn locations_that_name_the_same_bytes_gather_into_samples_of_their_own() {
        let locations = [
            location(0, 10..20),
            SampleLocation::new(properties(2, 0), 1, 10..20),
            SampleLocation::new(properties(3, 0), 1, 15..25),
        ];
        let mut reader = unlimited();
        reader.handle_sample_locations(locations.clone()).unwrap();

        reader.handle_input(10, &file(10..25)).unwrap();

        assert_eq!(
            poll_all(&mut reader),
            locations.iter().map(sample).collect::<Vec<_>>()
        );
    }

    #[test]
    fn bytes_that_arrive_before_their_location_is_held_are_dropped() {
        let location = location(0, 10..14);
        let mut reader = unlimited();

        reader.handle_input(10, &file(10..14)).unwrap();
        reader.handle_sample_locations([location.clone()]).unwrap();

        assert_eq!(reader.poll_sample(), None);
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(10, Some(4))));
    }

    #[test]
    fn a_location_of_no_bytes_is_whole_as_it_is_held() {
        let locations = [location(0, 10..10), location(1, 10..12)];
        let mut reader = unlimited();
        reader.handle_sample_locations(locations.clone()).unwrap();

        assert_eq!(poll_all(&mut reader), [sample(&locations[0])]);
    }

    #[test]
    fn a_sample_waits_behind_the_location_held_before_it() {
        let locations = [location(0, 10..12), location(1, 12..14)];
        let mut reader = unlimited();
        reader.handle_sample_locations(locations.clone()).unwrap();

        reader.handle_input(12, &file(12..14)).unwrap();
        assert_eq!(reader.poll_sample(), None);
        reader.handle_input(10, &file(10..12)).unwrap();

        assert_eq!(
            poll_all(&mut reader),
            locations.iter().map(sample).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_input_wanted_is_the_first_run_of_bytes_a_held_location_lacks() {
        let mut reader = unlimited();
        reader
            .handle_sample_locations([location(0, 30..40), location(1, 10..20)])
            .unwrap();
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(10, Some(10))));

        reader.handle_input(10, &file(10..13)).unwrap();
        reader.handle_input(15, &file(15..20)).unwrap();
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(13, Some(2))));

        reader.handle_input(13, &file(13..15)).unwrap();
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(30, Some(10))));
    }

    #[test]
    fn locations_past_the_held_location_limit_are_refused_and_none_of_them_is_held() {
        let mut reader = MediaDataReader::new(MediaDataLimits {
            held_location_count: 2,
            sample_bytes: u64::MAX,
            held_bytes: u64::MAX,
        });
        reader
            .handle_sample_locations([location(0, 10..12)])
            .unwrap();

        assert_eq!(
            reader.handle_sample_locations([location(1, 12..14), location(2, 14..16)]),
            Err(DemuxError::HeldLocationLimitExceeded {
                held_locations: 3,
                limit_locations: 2,
            })
        );
        reader.handle_input(10, &file(10..16)).unwrap();
        assert_eq!(poll_all(&mut reader), [sample(&location(0, 10..12))]);
        assert_eq!(reader.wanted_input(), None);
    }

    #[test]
    fn locations_past_the_sample_bytes_limit_are_refused_and_none_of_them_is_held() {
        let mut reader = MediaDataReader::new(MediaDataLimits {
            held_location_count: u64::MAX,
            sample_bytes: 5,
            held_bytes: u64::MAX,
        });
        reader
            .handle_sample_locations([location(0, 10..14)])
            .unwrap();

        assert_eq!(
            reader.handle_sample_locations([location(1, 14..16), location(2, 16..22)]),
            Err(DemuxError::SampleBytesLimitExceeded {
                declared_bytes: 6,
                limit_bytes: 5,
            })
        );
        reader.handle_input(10, &file(10..22)).unwrap();
        assert_eq!(poll_all(&mut reader), [sample(&location(0, 10..14))]);
        assert_eq!(reader.wanted_input(), None);
    }

    #[test]
    fn a_location_past_the_held_bytes_is_refused_as_past_the_sample_bytes() {
        let mut reader = MediaDataReader::new(MediaDataLimits {
            held_location_count: u64::MAX,
            sample_bytes: u64::MAX,
            held_bytes: 4,
        });

        assert_eq!(
            reader.handle_sample_locations([location(0, 10..15)]),
            Err(DemuxError::SampleBytesLimitExceeded {
                declared_bytes: 5,
                limit_bytes: 4,
            })
        );
    }

    /// Reader that holds 6 bytes of media data allocated, and the locations of samples of 4, 3 and 2 bytes at file offset 10
    fn holding_six_bytes() -> (MediaDataReader, [SampleLocation; 3]) {
        let mut reader = MediaDataReader::new(MediaDataLimits {
            held_location_count: u64::MAX,
            sample_bytes: u64::MAX,
            held_bytes: 6,
        });
        let locations = [
            location(0, 10..14),
            location(1, 14..17),
            location(2, 17..19),
        ];
        reader.handle_sample_locations(locations.clone()).unwrap();
        (reader, locations)
    }

    #[test]
    fn input_past_the_held_bytes_gathers_the_samples_that_fit_and_wants_the_rest_again() {
        let (mut reader, [first, second, third]) = holding_six_bytes();

        reader.handle_input(10, &file(10..19)).unwrap();
        assert_eq!(poll_all(&mut reader), [sample(&first)]);
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(14, Some(3))));

        reader.handle_input(10, &file(10..19)).unwrap();
        assert_eq!(poll_all(&mut reader), [sample(&second), sample(&third)]);
    }

    #[test]
    fn input_of_which_the_held_bytes_keep_nothing_is_refused_and_leaves_the_reader_as_it_was() {
        let (mut reader, [first, second, _]) = holding_six_bytes();
        reader.handle_input(10, &file(10..14)).unwrap();

        assert_eq!(
            reader.handle_input(14, &file(14..17)),
            Err(DemuxError::HeldBytesLimitExceeded {
                held_bytes: 7,
                limit_bytes: 6,
            })
        );
        assert_eq!(reader.wanted_input(), Some(ByteRange::new(14, Some(3))));
        assert_eq!(poll_all(&mut reader), [sample(&first)]);
        reader.handle_input(14, &file(14..17)).unwrap();
        assert_eq!(poll_all(&mut reader), [sample(&second)]);
    }

    #[test]
    fn a_taken_sample_no_longer_counts_toward_the_limits() {
        let mut reader = MediaDataReader::new(MediaDataLimits {
            held_location_count: 1,
            sample_bytes: u64::MAX,
            held_bytes: 4,
        });
        reader
            .handle_sample_locations([location(0, 10..14)])
            .unwrap();
        reader.handle_input(10, &file(10..14)).unwrap();
        assert_eq!(poll_all(&mut reader), [sample(&location(0, 10..14))]);

        reader
            .handle_sample_locations([location(1, 14..18)])
            .unwrap();
        reader.handle_input(14, &file(14..18)).unwrap();
        assert_eq!(poll_all(&mut reader), [sample(&location(1, 14..18))]);
    }

    #[test]
    fn finishing_with_a_sample_short_of_its_bytes_names_the_first_such_sample() {
        let mut reader = unlimited();
        reader
            .handle_sample_locations([
                location(0, 10..12),
                SampleLocation::new(properties(2, 0), 1, 12..20),
                location(1, 20..30),
            ])
            .unwrap();

        reader.handle_input(10, &file(10..13)).unwrap();
        reader.handle_input(18, &file(18..20)).unwrap();

        assert_eq!(
            reader.finish(),
            Err(DemuxError::UnfinishedSample {
                track_id: 2,
                needed_bytes: 8,
                available_bytes: 3,
            })
        );
        assert_eq!(poll_all(&mut reader), [sample(&location(0, 10..12))]);
    }

    #[test]
    fn clearing_drops_what_is_held_and_keeps_the_limits() {
        let mut reader = MediaDataReader::new(MediaDataLimits {
            held_location_count: 1,
            sample_bytes: u64::MAX,
            held_bytes: u64::MAX,
        });
        reader
            .handle_sample_locations([location(0, 10..14)])
            .unwrap();
        reader.handle_input(10, &file(10..12)).unwrap();

        reader.clear();

        assert_eq!(reader.wanted_input(), None);
        assert_eq!(reader.finish(), Ok(()));
        assert_eq!(
            reader.handle_sample_locations([location(1, 20..22), location(2, 22..24)]),
            Err(DemuxError::HeldLocationLimitExceeded {
                held_locations: 2,
                limit_locations: 1,
            })
        );
    }
}
