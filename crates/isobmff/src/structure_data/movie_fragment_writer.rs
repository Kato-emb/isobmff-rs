//! [`MovieFragmentWriter`], which writes the movie fragments of a movie from where their samples lie in the media data of each

use alloc::vec::Vec;

use isobmff_boxes::{
    MovieFragmentBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox, TrackRunBox,
    TrackRunSample, TrackRunSamples,
};
use isobmff_core::BoxEncode;

use super::Error;
use super::sample::{self, Sample, TrackSampleDescription};
use crate::SampleLocation;

/// Length of the header of the `mdat` that follows each `moof`, with a `largesize`
const MEDIA_DATA_HEADER_LENGTH: u64 = 16;

/// The most bytes the `moof` header and its `mfhd` take
const MOVIE_FRAGMENT_LENGTH: u64 = 8 + 16;

/// The most bytes one sample adds to a `moof`: a `traf` header, a `tfhd` with every field, a `tfdt` of version 1, a `trun` with a data offset and first sample flags, and a row of every field
const SAMPLE_LENGTH: u64 = 8 + 40 + 20 + 24 + 16;

/// What a track gives the samples of its track fragments
#[derive(Debug, Clone)]
struct Track {
    track_id: u32,
    /// The `data_reference_index` of each sample entry of the track, and
    /// whether it places the media data in the same file
    entries: Vec<(u16, bool)>,
    // Why not sum the `sample_duration`s of the samples held: that sum on
    // every sample makes taking a fragment's samples quadratic in their
    // number, and the samples of earlier fragments are no longer held.
    /// Decoding time the next sample of the track is to have, once one is
    /// taken
    next_decoding_time: Option<u64>,
}

/// Writer of the `movie fragment`s of a movie, from where each of their samples lies in the media data of its fragment
///
/// [`MovieFragmentBox`] (`moof`), ISO/IEC 14496-12 §8.8.4.
///
/// The writer is created with the [`TrackSampleDescription`] of each track;
/// where two share a `track_ID`, the first describes the track.
/// [`handle_location`](Self::handle_location) takes the samples of one
/// fragment, of any of the tracks, in decoding order within each track, each
/// at its offset from the first byte of the media data of the fragment: a
/// fragment declares its samples from the `moof` it is in, §8.8.7, wherever
/// it lies in the file. [`finish`](Self::finish) returns the `moof` that
/// declares them and goes on to the next fragment; the caller writes the
/// `moof`, then the header of an `mdat` of 16 bytes with a `largesize`
/// (§4.2.2), then the media data, each sample at its offset. The `finish` of a
/// writer of structure-data ends one unit, unlike that of a muxer, which ends
/// the file. Each `moof` takes the next `sequence_number`, from 1, §8.8.5.
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
/// fragment ends, nor check that the samples neither overlap nor leave a gap
/// in the media data.
///
/// ```
/// use isobmff::boxes::{MovieBox, SampleFlags};
/// use isobmff::structure_data::{MovieFragmentWriter, StructureDataReader, TrackSampleDescription};
/// use isobmff::{BoxDecode, BoxEncode, BoxHeader, BoxSize, BoxType, ExtendedSize};
/// use isobmff::{SampleLocation, SampleProperties};
///
/// // A `moov` of one track, whose sample entry names the media data in the file
/// let file: &[u8] = include_bytes!("../../tests/fixtures/fragmented.mp4");
/// let (_ftyp, rest) = isobmff::boxes::FileTypeBox::decode(file)?;
/// let (moov, _) = MovieBox::decode(rest)?;
/// let trak = &moov.trak[0];
/// let track = TrackSampleDescription {
///     track_id: trak.tkhd.track_id,
///     stsd: &trak.mdia.minf.stbl.stsd,
///     dref: &trak.mdia.minf.dinf.dref,
/// };
///
/// // Write a fragment of two samples, three bytes and two of its media data
/// let sample = |decoding_time, offset, size| SampleLocation {
///     properties: SampleProperties {
///         track_id: track.track_id,
///         decoding_time,
///         sample_duration: 512,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::default(),
///         sample_description_index: 1,
///     },
///     offset,
///     size,
/// };
/// let mut writer = MovieFragmentWriter::new(&[track], 1024);
/// writer.handle_location(sample(0, 0, 3))?;
/// writer.handle_location(sample(512, 3, 2))?;
/// let moof = writer.finish();
/// let mut written = vec![0; usize::try_from(moof.encoded_len())?];
/// moof.encode(&mut written)?;
/// let size = ExtendedSize::new(16 + 5).ok_or("too long")?;
/// let header = BoxHeader::new(BoxType::compact(*b"mdat"), BoxSize::Extended(size)).ok_or("too long")?;
/// let start = written.len();
/// written.resize(start + 16, 0);
/// header.encode(&mut written[start..])?;
/// written.extend_from_slice(b"abcde");
///
/// // The fragment declares the samples where their bytes lie in the file
/// let mut reader = StructureDataReader::new(1024);
/// reader.handle_movie(&moov)?;
/// let locations: Vec<_> = reader.handle_movie_fragment(&moof, 0)?.collect();
/// let media_data = u64::try_from(written.len())? - 5;
/// assert_eq!(locations, [sample(0, media_data, 3), sample(512, media_data + 3, 2)]);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct MovieFragmentWriter {
    tracks: Vec<Track>,
    sample_count_limit: u64,
    /// The samples of the fragment, each with the index of its track, in
    /// decoding order within each track
    samples: Vec<(usize, Sample)>,
    // Why not find the end of the samples held when the fragment is written:
    // the bound `handle_location` keeps needs it on every sample, and finding
    // it there makes taking a fragment's samples quadratic in their number,
    // as `benches/benches/movie_fragment.rs` measures.
    /// Length of the media data of the fragment: where the sample that ends
    /// last ends
    media_data_length: u64,
    /// `sequence_number` of the next `moof`, until the `moof` of the last a
    /// `sequence_number` counts is written
    sequence_number: Option<u32>,
}

impl MovieFragmentWriter {
    /// Creates a writer of fragments of the tracks `tracks` describe, each fragment of at most `sample_count_limit` samples
    ///
    /// The writer keeps some tens of bytes for each sample of a fragment until
    /// it finishes the fragment. A `trun` counts its samples in 32 bits, so a
    /// fragment holds no more than `u32::MAX` whatever the limit.
    #[must_use]
    pub fn new(tracks: &[TrackSampleDescription<'_>], sample_count_limit: u64) -> Self {
        Self {
            tracks: tracks
                .iter()
                .map(|track| Track {
                    track_id: track.track_id,
                    entries: sample::entry_data_references(track.stsd, track.dref),
                    next_decoding_time: None,
                })
                .collect(),
            sample_count_limit,
            samples: Vec::new(),
            media_data_length: 0,
            sequence_number: Some(1),
        }
    }

    /// Takes the next sample of the fragment, where `location` places it: its offset counts from the first byte of the media data of the fragment
    ///
    /// # Errors
    /// * [`Error::UnknownTrack`]: the writer describes no track of the
    ///   sample's `track_ID`.
    /// * [`Error::SampleCountOverLimit`]: the fragment holds as many samples
    ///   as the limit, or as a `trun` can count.
    /// * [`Error::SampleDescriptionIndexOutOfRange`]: the track has no sample
    ///   entry of the sample's `sample_description_index`.
    /// * [`Error::MediaDataElsewhere`]: the sample entry places the media data
    ///   in another file.
    /// * [`Error::CompositionOffsetOutOfRange`]: the composition offset of
    ///   the sample does not fit 32 bits, signed.
    /// * [`Error::DecodingTimeGap`]: the decoding time of the sample is not
    ///   that of the sample of its track before it plus its `sample_duration`.
    /// * [`Error::Encode`]: a field of the `sample_flags` of the sample holds
    ///   a value wider than its bits.
    /// * [`Error::Overflow`]: the decoding time after the sample is past the
    ///   last a track can have, the bytes of the sample would lie past where
    ///   a `data_offset` of 32 bits, signed, reaches, or the movie has as many
    ///   fragments as a `sequence_number` counts.
    ///
    /// A sample refused is not taken, and the writer is left as it was.
    pub fn handle_location(&mut self, location: SampleLocation) -> Result<(), Error> {
        let properties = location.properties;
        let track_id = properties.track_id;
        let (index, track) = self
            .tracks
            .iter_mut()
            .enumerate()
            .find(|(_, track)| track.track_id == track_id)
            .ok_or(Error::UnknownTrack { track_id })?;
        let sample = sample::checked(
            &location,
            &track.entries,
            self.samples.len(),
            self.sample_count_limit,
        )?;
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
        properties.sample_flags.to_bits().map_err(Error::Encode)?;
        if self.sequence_number.is_none() {
            return Err(overflow);
        }
        let media_data_length = location
            .offset
            .saturating_add(u64::from(location.size))
            .max(self.media_data_length);
        // Why bound the fragment here rather than when it is finished: a
        // `data_offset` reaches no further than the `moof`, the header of the
        // `mdat` and the end of the media data, so a fragment whose bound fits
        // `i32` is declared whole, and finishing never fails.
        let sample_count = u64::try_from(self.samples.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let bound = SAMPLE_LENGTH
            .saturating_mul(sample_count)
            .saturating_add(MOVIE_FRAGMENT_LENGTH + MEDIA_DATA_HEADER_LENGTH)
            .saturating_add(media_data_length);
        if bound > i32::MAX.unsigned_abs().into() {
            return Err(overflow);
        }
        track.next_decoding_time = Some(next_decoding_time);
        self.samples.push((index, sample));
        self.media_data_length = media_data_length;
        Ok(())
    }

    /// Whether the fragment holds a sample taken since the last was finished
    #[must_use]
    pub fn has_samples(&self) -> bool {
        !self.samples.is_empty()
    }

    /// Returns the `moof` of the samples taken, and goes on to the next fragment
    ///
    /// The `moof` declares each sample where it lies after the header of an
    /// `mdat` of 16 bytes with a `largesize`, written right after the `moof`. Where no sample was taken,
    /// the `moof` holds no track fragment; it still takes a `sequence_number`.
    #[must_use]
    pub fn finish(&mut self) -> MovieFragmentBox {
        let mut moof = MovieFragmentBox::default();
        // Why take u32::MAX again once the numbers are spent: `handle_location`
        // refuses every sample then, so only a `moof` of no sample repeats it.
        moof.mfhd.sequence_number = self.sequence_number.unwrap_or(u32::MAX);
        for (index, track) in self.tracks.iter().enumerate() {
            let samples: Vec<Sample> = self
                .samples
                .iter()
                .filter(|(of, _)| *of == index)
                .map(|&(_, sample)| sample)
                .collect();
            let duration = samples.iter().fold(0_u64, |total, sample| {
                total.saturating_add(u64::from(sample.sample_duration))
            });
            let mut decoding_time = track
                .next_decoding_time
                .unwrap_or(0)
                .saturating_sub(duration);
            for described in samples.chunk_by(|earlier, later| {
                earlier.sample_description_index == later.sample_description_index
            }) {
                let runs: Vec<(u64, usize)> = described
                    .chunk_by(|earlier, later| {
                        earlier.offset.checked_add(u64::from(earlier.size)) == Some(later.offset)
                    })
                    .map(|run| (run.first().map_or(0, |sample| sample.offset), run.len()))
                    .collect();
                moof.traf.push(track_fragment(
                    described,
                    &runs,
                    decoding_time,
                    track.track_id,
                ));
                decoding_time = described.iter().fold(decoding_time, |time, sample| {
                    time.saturating_add(u64::from(sample.sample_duration))
                });
            }
        }
        // Why place each run in the `mdat` before the `moof` is encoded, and
        // saturate: a `data_offset` is 32 bits whatever its value, so the
        // length of the `moof` does not depend on the offsets, and the bound
        // `handle_location` keeps holds the `moof`, the header and the media
        // data within `i32`.
        let base = i32::try_from(moof.encoded_len().saturating_add(MEDIA_DATA_HEADER_LENGTH))
            .unwrap_or(i32::MAX);
        for trun in moof.traf.iter_mut().flat_map(|traf| traf.trun.iter_mut()) {
            trun.data_offset = trun
                .data_offset
                .map(|data_offset| data_offset.saturating_add(base));
        }
        self.samples.clear();
        self.media_data_length = 0;
        self.sequence_number = self
            .sequence_number
            .and_then(|number| number.checked_add(1));
        moof
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
        // and the bound `handle_location` keeps holds it within `i32`.
        trun.data_offset = Some(i32::try_from(offset).unwrap_or(i32::MAX));
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
