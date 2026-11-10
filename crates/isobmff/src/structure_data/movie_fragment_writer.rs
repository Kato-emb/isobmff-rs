//! [`MovieFragmentWriter`], which writes the movie fragments of a movie from where their samples lie in the media data that follows each

use alloc::vec::Vec;

use isobmff_boxes::{
    MovieBox, MovieFragmentBox, SampleFlags, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
    TrackRunBox, TrackRunSample, TrackRunSamples,
};
use isobmff_core::BoxEncode;

use super::{Error, sample_table};
use crate::{SampleProperties, framing};

/// Length of the header of the `mdat` that follows each `moof`, which [`framing::media_data_header`] writes
const MEDIA_DATA_HEADER_LENGTH: u64 = 16;

/// The most bytes the `moof` header and its `mfhd` take
const MOVIE_FRAGMENT_LENGTH: u64 = 8 + 16;

/// The most bytes one sample adds to a `moof`: a `traf` header, a `tfhd` with every field, a `tfdt` of version 1, a `trun` with a data offset and first sample flags, and a row of every field
const SAMPLE_LENGTH: u64 = 8 + 40 + 20 + 24 + 16;

/// What a track of the `moov` gives the samples of its track fragments
#[derive(Debug, Clone)]
struct Track {
    track_id: u32,
    /// The `data_reference_index` of each sample entry of the track, from 1,
    /// and whether it places the media data in the same file
    entries: Vec<(u16, bool)>,
    // Why not sum the `sample_duration`s of the samples held: that sum on
    // every sample makes taking a fragment's samples quadratic in their
    // number, and the samples of earlier fragments are no longer held.
    /// Decoding time the next sample of the track is to have, once one is
    /// taken
    next_decoding_time: Option<u64>,
}

/// What a track fragment run holds of a sample
#[derive(Debug, Clone, Copy)]
struct Sample {
    track_id: u32,
    sample_description_index: u32,
    sample_duration: u32,
    size: u32,
    sample_composition_time_offset: i32,
    sample_flags: SampleFlags,
}

/// Writer of the `movie fragment`s of a movie, from what the structure-data declares of each sample and its size
///
/// [`MovieFragmentBox`] (`moof`), ISO/IEC 14496-12 §8.8.4, and the header of
/// the `mdat`, §8.1.1, that holds its samples.
///
/// [`handle_sample`](Self::handle_sample) takes the samples of one fragment,
/// of any of the tracks of the `moov`, in the order their bytes follow one
/// another in the `mdat`, and in decoding order within each track.
/// [`write_movie_fragment`](Self::write_movie_fragment) writes the `moof` that
/// declares them and the header of the `mdat`; the caller writes their bytes
/// after it, in the order it handed them over. Each `moof` takes the next
/// `sequence_number`, from 1, §8.8.5.
///
/// The fragment declares its samples in the fewest boxes and fields they
/// allow:
///
/// * a `traf` (§8.8.6) for each run of samples of a track that one sample
///   entry describes, its `tfhd` (§8.8.7) with `default-base-is-moof` and the
///   `sample_description_index`, and a `tfdt` (§8.8.12) with the decoding time
///   of its first sample;
/// * a `trun` (§8.8.8) for each run of samples of the `traf` whose bytes
///   follow one another, its `data_offset` counting from the first byte of the
///   `moof`;
/// * a `sample_duration`, a `sample_size` or `sample_flags` in the `tfhd`
///   where every sample of the `traf` has the same, the flags of the first
///   sample of a `trun` as `first_sample_flags` where only they differ, and
///   in each row otherwise; a `sample_composition_time_offset` in each row of
///   a `trun` where one is not 0, in version 1 where one is negative.
///
/// A fragment takes nothing from the `trex` of a track, so the `moov` holds
/// one of any defaults. The decoding time of the first sample of each track
/// is the caller's, and of each later sample the one before it plus its
/// `sample_duration`, across fragments; the writer does not choose where a
/// fragment ends.
///
/// ```
/// use isobmff::structure_data::{MovieFragmentWriter, StructureDataReader};
/// use isobmff::{SampleLocation, SampleProperties};
/// use isobmff_boxes::{MovieBox, MovieFragmentBox, SampleFlags};
/// use isobmff_core::BoxDecode;
///
/// // A `moov` of one track, whose sample entry names the media data in the file
/// let file: &[u8] = include_bytes!("../../tests/fixtures/fragmented.mp4");
/// let (_ftyp, rest) = isobmff_boxes::FileTypeBox::decode(file)?;
/// let (moov, _) = MovieBox::decode(rest)?;
/// let track_id = moov.trak[0].tkhd.track_id;
///
/// // Write a fragment of two samples, then their bytes
/// let sample = |decoding_time| SampleProperties {
///     track_id,
///     decoding_time,
///     sample_duration: 512,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// let mut writer = MovieFragmentWriter::new(&moov, 1024);
/// writer.handle_sample(sample(0), 3)?;
/// writer.handle_sample(sample(512), 2)?;
/// let mut written = Vec::new();
/// writer.write_movie_fragment(&mut written)?;
/// written.extend_from_slice(b"abcde");
///
/// // The fragment declares the samples where their bytes lie
/// let (moof, _) = MovieFragmentBox::decode(&written)?;
/// let mut reader = StructureDataReader::new(1024);
/// reader.handle_movie(&moov)?;
/// let locations: Vec<_> = reader.handle_movie_fragment(&moof, 0)?.collect();
/// let mdat = u64::try_from(written.len())? - 5;
/// assert_eq!(
///     locations,
///     [
///         SampleLocation { properties: sample(0), offset: mdat, size: 3 },
///         SampleLocation { properties: sample(512), offset: mdat + 3, size: 2 },
///     ]
/// );
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct MovieFragmentWriter {
    tracks: Vec<Track>,
    sample_count_limit: u64,
    /// The samples of the fragment, in the order their bytes follow one
    /// another
    samples: Vec<Sample>,
    // Why not sum the sizes of the samples held: that sum on every sample
    // makes taking a fragment's samples quadratic in their number.
    /// Number of bytes of the samples held
    media_data_length: u64,
    /// `sequence_number` of the next `moof`, until the `moof` of the last a
    /// `sequence_number` counts is written
    sequence_number: Option<u32>,
}

impl MovieFragmentWriter {
    /// Creates a writer of fragments of the tracks of `movie`, each of at most `sample_count_limit` samples
    ///
    /// The writer keeps some tens of bytes for each sample of a fragment until
    /// it writes the fragment. A `trun` counts its samples in 32 bits, so a
    /// fragment holds no more than `u32::MAX` whatever the limit.
    #[must_use]
    pub fn new(movie: &MovieBox, sample_count_limit: u64) -> Self {
        Self {
            tracks: movie
                .trak
                .iter()
                .map(|track| Track {
                    track_id: track.tkhd.track_id,
                    entries: sample_table::entry_data_references(track),
                    next_decoding_time: None,
                })
                .collect(),
            sample_count_limit,
            samples: Vec::new(),
            media_data_length: 0,
            sequence_number: Some(1),
        }
    }

    /// Takes the next sample of the fragment, of `size` bytes, which follow those of the sample before it
    ///
    /// # Errors
    /// * [`Error::SampleCountOverLimit`]: the fragment holds as many samples
    ///   as the limit, or as a `trun` can count.
    /// * [`Error::UnknownTrack`]: the `moov` has no track of the sample's
    ///   `track_ID`.
    /// * [`Error::SampleDescriptionIndexOutOfRange`]: the track has no sample
    ///   entry of the sample's `sample_description_index`.
    /// * [`Error::MediaDataElsewhere`]: the sample entry places the media data
    ///   in another file.
    /// * [`Error::DecodingTimeGap`]: the decoding time of the sample is not
    ///   that of the sample of its track before it plus its `sample_duration`.
    /// * [`Error::CompositionOffsetOutOfRange`]: the composition offset of
    ///   the sample does not fit 32 bits, signed.
    /// * [`Error::Encode`]: a field of the `sample_flags` of the sample holds
    ///   a value wider than its bits.
    /// * [`Error::Overflow`]: the decoding time after the sample is past the
    ///   last a track can have, the bytes of the fragment would lie past
    ///   where a `data_offset` of 32 bits, signed, reaches, or the movie has
    ///   as many fragments as a `sequence_number` counts.
    ///
    /// A sample refused is not taken, and the writer is left as it was.
    pub fn handle_sample(&mut self, properties: SampleProperties, size: u32) -> Result<(), Error> {
        let limit = self.sample_count_limit.min(u64::from(u32::MAX));
        let sample_count = u64::try_from(self.samples.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        if sample_count > limit {
            return Err(Error::SampleCountOverLimit {
                sample_count,
                limit,
            });
        }
        let track_id = properties.track_id;
        let track = self
            .tracks
            .iter_mut()
            .find(|track| track.track_id == track_id)
            .ok_or(Error::UnknownTrack { track_id })?;
        let sample_description_index = properties.sample_description_index;
        match usize::try_from(sample_description_index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| track.entries.get(index))
        {
            Some(&(_, true)) => {}
            Some(&(data_reference_index, false)) => {
                return Err(Error::MediaDataElsewhere {
                    track_id,
                    data_reference_index,
                });
            }
            None => {
                return Err(Error::SampleDescriptionIndexOutOfRange {
                    sample_description_index,
                });
            }
        }
        if let Some(expected) = track.next_decoding_time
            && expected != properties.decoding_time
        {
            return Err(Error::DecodingTimeGap {
                track_id,
                expected,
                found: properties.decoding_time,
            });
        }
        let overflow = Error::Overflow { track_id };
        let next_decoding_time = properties
            .decoding_time
            .checked_add(u64::from(properties.sample_duration))
            .ok_or(overflow)?;
        let sample_composition_time_offset =
            i32::try_from(properties.sample_composition_time_offset).map_err(|_| {
                Error::CompositionOffsetOutOfRange {
                    sample_composition_time_offset: properties.sample_composition_time_offset,
                }
            })?;
        properties.sample_flags.to_bits().map_err(Error::Encode)?;
        if self.sequence_number.is_none() {
            return Err(overflow);
        }
        let media_data_length = self
            .media_data_length
            .checked_add(u64::from(size))
            .ok_or(overflow)?;
        // Why bound the fragment here rather than when it is written: a
        // `data_offset` reaches no further than the `moof`, the header of the
        // `mdat` and the bytes of the samples before the run, so a fragment
        // whose bound fits `i32` writes, and one refused at writing could be
        // neither written nor taken back.
        let bound = SAMPLE_LENGTH
            .saturating_mul(sample_count)
            .saturating_add(MOVIE_FRAGMENT_LENGTH + MEDIA_DATA_HEADER_LENGTH)
            .saturating_add(media_data_length);
        if bound > i32::MAX.unsigned_abs().into() {
            return Err(overflow);
        }
        track.next_decoding_time = Some(next_decoding_time);
        self.samples.push(Sample {
            track_id,
            sample_description_index,
            sample_duration: properties.sample_duration,
            size,
            sample_composition_time_offset,
            sample_flags: properties.sample_flags,
        });
        self.media_data_length = media_data_length;
        Ok(())
    }

    /// Whether the fragment holds a sample taken since the last was written
    #[must_use]
    pub fn has_samples(&self) -> bool {
        !self.samples.is_empty()
    }

    /// Appends the `moof` of the samples taken and the header of the `mdat` that holds them to `out`, and starts the next fragment
    ///
    /// The bytes of the samples follow, in the order they were taken; the
    /// caller writes them. Where no sample was taken, nothing is written and
    /// the next fragment takes the same `sequence_number`.
    ///
    /// # Errors
    /// * [`Error::Encode`]: the `moof` does not write.
    ///
    /// A call that fails leaves `out` and the writer as they were.
    pub fn write_movie_fragment(&mut self, out: &mut Vec<u8>) -> Result<(), Error> {
        let Some(first) = self.samples.first() else {
            return Ok(());
        };
        let overflow = Error::Overflow {
            track_id: first.track_id,
        };
        let sequence_number = self.sequence_number.ok_or(overflow)?;
        let mut moof = MovieFragmentBox::default();
        moof.mfhd.sequence_number = sequence_number;
        // Why place each run in the `mdat` before the `moof` is encoded: a
        // `data_offset` is 32 bits whatever its value, so the length of the
        // `moof` does not depend on the offsets, and they are moved past it
        // once it is known.
        let mut offsets = Vec::with_capacity(self.samples.len());
        let mut offset = 0_u64;
        for sample in &self.samples {
            offsets.push(offset);
            offset = offset.saturating_add(u64::from(sample.size));
        }
        for track in &self.tracks {
            let indices: Vec<usize> = (0..self.samples.len())
                .filter(|&index| {
                    self.samples
                        .get(index)
                        .is_some_and(|sample| sample.track_id == track.track_id)
                })
                .collect();
            let duration = indices
                .iter()
                .filter_map(|&index| self.samples.get(index))
                .fold(0_u64, |total, sample| {
                    total.saturating_add(u64::from(sample.sample_duration))
                });
            let mut decoding_time = track
                .next_decoding_time
                .unwrap_or(0)
                .saturating_sub(duration);
            for described in indices.chunk_by(|&earlier, &later| {
                self.samples
                    .get(earlier)
                    .map(|sample| sample.sample_description_index)
                    == self
                        .samples
                        .get(later)
                        .map(|sample| sample.sample_description_index)
            }) {
                let samples: Vec<Sample> = described
                    .iter()
                    .filter_map(|&index| self.samples.get(index).copied())
                    .collect();
                let runs: Vec<(u64, usize)> = described
                    .chunk_by(|&earlier, &later| earlier.checked_add(1) == Some(later))
                    .map(|run| {
                        let offset = run.first().and_then(|&first| offsets.get(first));
                        (offset.copied().unwrap_or(0), run.len())
                    })
                    .collect();
                let traf = track_fragment(&samples, &runs, decoding_time, track.track_id);
                decoding_time = samples.iter().fold(decoding_time, |time, sample| {
                    time.saturating_add(u64::from(sample.sample_duration))
                });
                moof.traf.push(traf);
            }
        }
        let start = out.len();
        let length = usize::try_from(moof.encoded_len()).map_err(|_| overflow)?;
        let base = u64::try_from(length)
            .ok()
            .and_then(|length| length.checked_add(MEDIA_DATA_HEADER_LENGTH))
            .and_then(|base| i32::try_from(base).ok())
            .ok_or(overflow)?;
        for trun in moof.traf.iter_mut().flat_map(|traf| traf.trun.iter_mut()) {
            trun.data_offset = Some(
                trun.data_offset
                    .and_then(|data_offset| data_offset.checked_add(base))
                    .ok_or(overflow)?,
            );
        }
        let header = framing::media_data_header(self.media_data_length).ok_or(overflow)?;
        out.resize(start.saturating_add(length), 0);
        if let Err(source) = moof.encode(out.get_mut(start..).unwrap_or_default()) {
            out.truncate(start);
            return Err(Error::Encode(source));
        }
        out.extend_from_slice(&header);
        self.samples.clear();
        self.media_data_length = 0;
        self.sequence_number = sequence_number.checked_add(1);
        Ok(())
    }
}

/// Returns the `traf` of `samples`, of track `track_id` and one sample entry, the first at `decoding_time`, which lie in `runs` of the `mdat`: the offset of each run and its number of samples
fn track_fragment(
    samples: &[Sample],
    runs: &[(u64, usize)],
    decoding_time: u64,
    track_id: u32,
) -> TrackFragmentBox {
    let shared = |field: fn(&Sample) -> u32| {
        let first = samples.first().map(field)?;
        samples
            .iter()
            .all(|sample| field(sample) == first)
            .then_some(first)
    };
    let default_sample_duration = shared(|sample| sample.sample_duration);
    let default_sample_size = shared(|sample| sample.size);
    let mut rest = samples;
    let run_samples: Vec<&[Sample]> = runs
        .iter()
        .map(|&(_, length)| {
            let (run, after) = rest.split_at(length.min(rest.len()));
            rest = after;
            run
        })
        .collect();
    let first_flags = samples.first().map(|sample| sample.sample_flags);
    let mut later_flags = run_samples
        .iter()
        .flat_map(|run| run.iter().skip(1))
        .map(|sample| sample.sample_flags);
    let first_later_flags = later_flags.next();
    let later_default =
        first_later_flags.filter(|shared| later_flags.all(|flags| flags == *shared));
    let default_sample_flags = if samples
        .iter()
        .all(|sample| Some(sample.sample_flags) == first_flags)
    {
        first_flags
    } else {
        later_default
    };
    let flags_in_rows = default_sample_flags.is_none();

    let mut traf = TrackFragmentBox::default();
    traf.tfhd.default_base_is_moof = true;
    traf.tfhd.track_id = track_id;
    traf.tfhd.sample_description_index = samples
        .first()
        .map(|sample| sample.sample_description_index);
    traf.tfhd.default_sample_duration = default_sample_duration;
    traf.tfhd.default_sample_size = default_sample_size;
    traf.tfhd.default_sample_flags = default_sample_flags;
    let mut tfdt = TrackFragmentBaseMediaDecodeTimeBox::default();
    tfdt.version = u8::from(decoding_time > u64::from(u32::MAX));
    tfdt.base_media_decode_time = decoding_time;
    traf.tfdt = Some(tfdt);

    for (&(offset, _), run) in runs.iter().zip(run_samples) {
        let mut trun = TrackRunBox::default();
        // Why hold the offset in the `mdat` for now: the writer moves it past
        // the `moof` and the header of the `mdat` once their length is known,
        // and the bound `handle_sample` keeps holds it within `i32`.
        trun.data_offset = i32::try_from(offset).ok();
        let first_flags = run.first().map(|sample| sample.sample_flags);
        if !flags_in_rows && first_flags != default_sample_flags {
            trun.first_sample_flags = first_flags;
        }
        let with_offsets = run
            .iter()
            .any(|sample| sample.sample_composition_time_offset != 0);
        trun.version = u8::from(
            run.iter()
                .any(|sample| sample.sample_composition_time_offset < 0),
        );
        let rows: Vec<TrackRunSample> = run
            .iter()
            .map(|sample| {
                let mut row = TrackRunSample::default();
                if default_sample_duration.is_none() {
                    row.sample_duration = Some(sample.sample_duration);
                }
                if default_sample_size.is_none() {
                    row.sample_size = Some(sample.size);
                }
                if flags_in_rows {
                    row.sample_flags = Some(sample.sample_flags);
                }
                if with_offsets {
                    row.sample_composition_time_offset =
                        Some(i64::from(sample.sample_composition_time_offset));
                }
                row
            })
            .collect();
        trun.samples = if rows.iter().all(|row| *row == TrackRunSample::default()) {
            TrackRunSamples::EmptyRows {
                sample_count: u32::try_from(rows.len()).unwrap_or(u32::MAX),
            }
        } else {
            TrackRunSamples::Rows { samples: rows }
        };
        traf.trun.push(trun);
    }
    traf
}
