//! Movie fragment random access, ISO/IEC 14496-12 §8.8.9 to §8.8.11: the table of the sync samples of movie fragments, and the `mfro` that finds it from the end of the file

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use isobmff_boxes::{
    MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox, TrackFragmentRandomAccessBox,
    TrackFragmentRandomAccessEntry,
};
use isobmff_core::{BoxDecode, BoxEncode, Error};

use super::movie_fragment::Run;
use crate::error::MuxError;

/// Returns the file offset of the `mfra` the last 16 bytes of a file of `file_length` bytes point at, where they are an `mfro`, ISO/IEC 14496-12 §8.8.11
///
/// The `mfro` is the last box of the `mfra`, and repeats its size, so the
/// `mfra` starts that many bytes before the end of the file. The offset is a
/// hint: what is there may not be an `mfra` (§8.8.9).
///
/// `None` where the bytes are not an `mfro` of 16 bytes, or it gives a size
/// past the start of the file.
///
/// # Examples
///
/// ```
/// use isobmff::structure_data::movie_fragment_random_access_start;
///
/// // An mfro of an mfra of 61 bytes, at the end of a file of 1000
/// let last_bytes = *b"\0\0\0\x10mfro\0\0\0\0\0\0\0\x3d";
/// assert_eq!(movie_fragment_random_access_start(1000, &last_bytes), Some(939));
/// assert_eq!(movie_fragment_random_access_start(60, &last_bytes), None);
/// assert_eq!(movie_fragment_random_access_start(1000, &[0; 16]), None);
/// ```
#[must_use]
pub fn movie_fragment_random_access_start(file_length: u64, last_bytes: &[u8; 16]) -> Option<u64> {
    let (mfro, rest) = MovieFragmentRandomAccessOffsetBox::decode(last_bytes).ok()?;
    if !rest.is_empty() {
        return None;
    }

    file_length.checked_sub(u64::from(mfro.parent_size))
}

/// Track fragment random access tables of the movie fragments written, a `tfra` for each track that has a sync sample listed
#[derive(Debug, Default)]
pub(super) struct RandomAccessTables {
    /// Each `tfra` by the index of its track among the tracks of the movie
    tfra: BTreeMap<usize, TrackFragmentRandomAccessBox>,
}

impl RandomAccessTables {
    /// Lists the first sync sample of each of `track_fragments`, of the movie fragment whose `moof` starts at file offset `moof_offset`
    ///
    /// The track fragments, and the track runs of each, are numbered from 1
    /// in their order (§8.8.10.3). The time of an entry is the presentation
    /// time of its sample in the timescale of its track: the decoding time
    /// plus the composition time offset. A sync sample before presentation
    /// time 0 has no time to list, and the next of its track fragment is
    /// listed instead.
    pub(super) fn handle_movie_fragment(
        &mut self,
        track_fragments: &[Vec<&Run<'_>>],
        moof_offset: u64,
    ) {
        for (traf_number, track_runs) in (1..).zip(track_fragments) {
            let sync_sample = (1..).zip(track_runs).find_map(|(trun_number, run)| {
                run.samples
                    .iter()
                    .zip(1..)
                    .find_map(|(sample, sample_delta)| {
                        let properties = sample.properties();
                        if properties.sample_flags.sample_is_non_sync_sample {
                            return None;
                        }
                        let mut entry = TrackFragmentRandomAccessEntry::default();
                        entry.time = properties
                            .decoding_time
                            .checked_add_signed(properties.sample_composition_time_offset)?;
                        entry.moof_offset = moof_offset;
                        entry.traf_number = traf_number;
                        entry.trun_number = trun_number;
                        entry.sample_delta = sample_delta;
                        Some((run, entry))
                    })
            });
            if let Some((run, entry)) = sync_sample {
                self.tfra
                    .entry(run.track)
                    .or_insert_with(|| {
                        let mut tfra = TrackFragmentRandomAccessBox::default();
                        tfra.track_id = run.track_id;
                        tfra
                    })
                    .entries
                    .push(entry);
            }
        }
    }

    /// Returns the `mfra` of the sync samples listed, its `mfro` giving its size
    ///
    /// The `tfra`s go in the order of the tracks of the movie. Each takes
    /// version 0 where its times and offsets fit in 32 bits, and the fewest
    /// bytes for its numbers. A track with no sync sample listed has no
    /// `tfra`: one of no entry declares every sample a sync sample
    /// (§8.8.10.3).
    ///
    /// # Errors
    ///
    /// * [`Box`](MuxError::Box): the `mfra` is longer than its `mfro` counts.
    pub(super) fn finish(self) -> Result<MovieFragmentRandomAccessBox, MuxError> {
        let mut mfra = MovieFragmentRandomAccessBox::default();
        mfra.tfra = self.tfra.into_values().collect();
        for tfra in &mut mfra.tfra {
            let narrow = tfra.entries.iter().all(|entry| {
                u32::try_from(entry.time).is_ok() && u32::try_from(entry.moof_offset).is_ok()
            });
            tfra.version = u8::from(!narrow);
            let largest = |number: fn(&TrackFragmentRandomAccessEntry) -> u32| {
                length_size(tfra.entries.iter().map(number).max().unwrap_or_default())
            };
            tfra.length_size_of_traf_num = largest(|entry| entry.traf_number);
            tfra.length_size_of_trun_num = largest(|entry| entry.trun_number);
            tfra.length_size_of_sample_num = largest(|entry| entry.sample_delta);
        }
        mfra.mfro.parent_size = u32::try_from(mfra.encoded_len()).map_err(|_| MuxError::Box {
            error: Error::invalid_value("parent_size"),
        })?;

        Ok(mfra)
    }
}

/// Returns the `length_size_of_*` of `number`: the bytes it takes, less one
fn length_size(number: u32) -> u8 {
    let length_size = number.checked_ilog2().map_or(0, |log| log / 8);
    u8::try_from(length_size).unwrap_or(3)
}

#[cfg(test)]
mod tests {
    use isobmff_boxes::SampleFlags;

    use super::{RandomAccessTables, length_size, movie_fragment_random_access_start};
    use crate::sample::{Sample, SampleProperties};
    use crate::structure_data::fixtures::{encoded, movie, track, written};
    use crate::structure_data::movie_fragment::{runs, track_fragments};

    /// Returns a sample of `track_id` at `decoding_time`, lasting 10, presented `offset` after it, and a sync sample where `sync`
    fn sample(track_id: u32, decoding_time: u64, offset: i64, sync: bool) -> Sample {
        let mut sample_flags = SampleFlags::default();
        sample_flags.sample_is_non_sync_sample = !sync;
        Sample::new(
            SampleProperties {
                track_id,
                decoding_time,
                sample_duration: 10,
                sample_composition_time_offset: offset,
                sample_flags,
                sample_description_index: 1,
            },
            b"A".to_vec(),
        )
    }

    #[test]
    fn the_first_sync_sample_of_each_track_in_a_fragment_is_listed_by_its_track_fragment_track_run_and_number()
     {
        let movie = movie([1, 2, 3].map(|track_id| track(track_id, 0, &[], &[], &[])));
        let mut tables = RandomAccessTables::default();
        let first = [
            sample(1, 0, 0, false),
            sample(2, 0, 5, true),
            sample(3, 0, 0, false),
            sample(1, 10, 0, true),
            sample(1, 20, 0, true),
        ];
        let second = [
            sample(1, 30, -40, true),
            sample(1, 40, 0, true),
            sample(2, 10, 0, false),
        ];

        for (samples, moof_offset) in [(&first[..], 100), (&second[..], 1 << 32)] {
            let runs = runs(&movie, samples, None).unwrap();
            tables.handle_movie_fragment(&track_fragments(&runs), moof_offset);
        }
        let mfra = tables.finish().unwrap();

        let tfra_1 = written(
            b"tfra",
            &[
                &[1, 0, 0, 0],
                &1_u32.to_be_bytes(),
                &[0; 4],
                &2_u32.to_be_bytes(),
                &10_u64.to_be_bytes(),
                &100_u64.to_be_bytes(),
                &[1, 2, 1],
                &40_u64.to_be_bytes(),
                &(1_u64 << 32).to_be_bytes(),
                &[1, 1, 2],
            ],
        );
        let tfra_2 = written(
            b"tfra",
            &[
                &[0; 4],
                &2_u32.to_be_bytes(),
                &[0; 4],
                &1_u32.to_be_bytes(),
                &5_u32.to_be_bytes(),
                &100_u32.to_be_bytes(),
                &[2, 1, 1],
            ],
        );
        let mfra_len = 8 + tfra_1.len() + tfra_2.len() + 16;
        assert_eq!(
            encoded(&mfra),
            written(
                b"mfra",
                &[
                    &tfra_1,
                    &tfra_2,
                    &written(
                        b"mfro",
                        &[&[0; 4], &u32::try_from(mfra_len).unwrap().to_be_bytes()]
                    ),
                ],
            )
        );
    }

    #[test]
    fn a_number_takes_the_fewest_bytes_that_hold_it() {
        assert_eq!(
            [0, 0xff, 0x100, 0xff_ffff, 0x100_0000, u32::MAX].map(length_size),
            [0, 0, 1, 2, 3, 3]
        );
    }

    #[test]
    fn the_mfra_starts_the_size_its_mfro_repeats_before_the_end_of_the_file() {
        let mfro = *b"\0\0\0\x10mfro\0\0\0\0\0\0\0\x3d";
        let not_an_mfro = *b"\0\0\0\x10free\0\0\0\0\0\0\0\x3d";
        let mfro_of_version_1 = *b"\0\0\0\x10mfro\x01\0\0\0\0\0\0\x3d";

        assert_eq!(movie_fragment_random_access_start(61, &mfro), Some(0));
        assert_eq!(movie_fragment_random_access_start(60, &mfro), None);
        assert_eq!(movie_fragment_random_access_start(1000, &not_an_mfro), None);
        assert_eq!(
            movie_fragment_random_access_start(1000, &mfro_of_version_1),
            None
        );
    }
}
