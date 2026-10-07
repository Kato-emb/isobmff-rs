//! [`SampleTableWriter`], the samples of a presentation laid out as the sample tables of a movie, ISO/IEC 14496-12 §8.5.1 and §8.7

mod open_track;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::mem;

use isobmff_boxes::{
    ChunkOffsets, CompositionOffsetBox, DegradationPriorityBox, MovieBox, PaddingBitsBox,
    SampleDependencyTypeBox, SampleDescriptionBox, SampleSizes, SampleTableBox, SampleToChunkBox,
    SyncSampleBox, TimeToSampleBox, TrackBox,
};

use crate::error::Error;
use crate::sample::Sample;
use crate::sample_description::SampleDescriptions;
use crate::sample_table_writer::open_track::OpenTrack;

/// Lays the samples of a presentation out as the sample tables of a movie
///
/// The writer takes the samples chunk by chunk — a chunk being a contiguous
/// set of samples of one track (ISO/IEC 14496-12 §3.1.2) — and hands over the
/// bytes of each through [`poll_data`](Self::poll_data), to be laid down where
/// the caller opened the chunk.
/// What it keeps is what the sample tables of a track state about them: the
/// decode timeline (`stts`, §8.6.1.2), the chunks the samples lie in (`stsc`,
/// §8.7.4), their sizes (a `stsz`, §8.7.3.2, never a `stz2`) and where each
/// chunk starts (`stco` or `co64`, §8.7.5), and the optional tables stating their composition time
/// offsets (`ctts`, §8.6.1.3) and the fields of their `sample_flags` (`sdtp`,
/// `padb`, `stss` and `stdp`, §8.8.3.1), which
/// [`poll_sample_tables`](Self::poll_sample_tables) hands over per track as
/// [`SampleTables`] once [`finish`](Self::finish) has built them. The writer is made for the movie the
/// tables go into, which it checks the samples against; the `stsd` of each
/// track, and the movie itself, stay with the caller.
///
/// # Layout
///
/// * A chunk is opened by [`begin_chunk`](Self::begin_chunk) stating where in
///   the file its first byte lies, and holds the samples handed over until
///   [`finish_chunk`](Self::finish_chunk) closes it. The samples of a chunk lie
///   one after another from there, and the chunk belongs to the track of its
///   first sample.
/// * The tables of a track count its chunks in the order they were opened and
///   its samples in the order they were handed over, so the chunks of a track
///   have to be opened at rising offsets for
///   [`sample_table::sample_extents`](crate::sample_table::sample_extents) to
///   hand the samples back in that order.
/// * Each table is stated the way its box chooses from the values laid down:
///   [`SampleSizeBox::from_sizes`](isobmff_boxes::SampleSizeBox::from_sizes),
///   [`TimeToSampleBox::from_deltas`], [`SampleToChunkBox::from_chunks`] and
///   [`ChunkOffsets::from_offsets`].
/// * An optional table is left out when every sample of the track states what
///   its absence does: the `ctts` when every offset is zero, the `stss` when
///   every sample is a sync sample, and the `sdtp`, the `padb` and the `stdp`
///   when every field they state is zero. A track whose samples are none of
///   them sync samples carries an `stss` listing none.
///
/// # Contract
///
/// * A sample belongs to a track the movie declares by a `trak`, which is
///   otherwise [`UnknownTrackId`](crate::Error::UnknownTrackId), and is
///   described by an `stsd` entry of that track whose data reference is the
///   file itself, as a reader of the tables resolves it (§8.5.2, §8.7.2).
///   The first sample of a chunk is checked, and the samples after it are
///   held to its track and entry: the failures of the entry are those of the
///   reader —
///   [`UnknownSampleDescriptionIndex`](crate::Error::UnknownSampleDescriptionIndex),
///   [`UnknownDataReferenceIndex`](crate::Error::UnknownDataReferenceIndex),
///   [`ExternalDataReference`](crate::Error::ExternalDataReference), or
///   the failure of an entry that does not read as a sample entry, carried
///   on [`Box`](crate::Error::Box).
/// * Handing a sample over or closing a chunk while no chunk is open is
///   [`NoChunkOpen`](crate::Error::NoChunkOpen), and opening a chunk or
///   declaring the samples over while one is still open is
///   [`ChunkStillOpen`](crate::Error::ChunkStillOpen). A sample of another
///   track than the chunk holds is
///   [`TrackIdMismatch`](crate::Error::TrackIdMismatch). A chunk no
///   sample was handed over to is not recorded.
/// * The decode timeline of a track starts at zero and states how long each
///   sample lasts, never when it is decoded, so a sample of a track has to
///   start where the one before it ends — the first at zero — which is
///   otherwise
///   [`DecodeTimeMismatch`](crate::Error::DecodeTimeMismatch).
/// * The samples of a chunk are all described by one `stsd` entry, which the
///   run of chunks states for them (§8.7.4): a chunk mixing two is
///   [`SampleDescriptionIndexMismatch`](crate::Error::SampleDescriptionIndexMismatch).
/// * A sample stating a composition time offset outside what 32 signed bits
///   hold is
///   [`CompositionTimeOffsetOutOfRange`](crate::Error::CompositionTimeOffsetOutOfRange):
///   one past [`i32::MAX`] is refused too, which the readers here take as
///   negative.
/// * A chunk holding more samples than an `stsc` entry counts, or numbered
///   past what one reaches, is reported by [`finish`](Self::finish), where the
///   tables are built: the failure of the box, carried on
///   [`Box`](crate::Error::Box).
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::Error::AlreadyFinished) aside: every
///   later call reports that same failure again.
/// * [`finish`](Self::finish) declares the samples over and builds the
///   tables. A chunk opened or closed or a sample handed over then, or a
///   second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::Error::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{
///     ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets, SampleFlags, SampleToChunkBox,
///     SampleToChunkEntry,
/// };
/// use isobmff_sample::{Sample, SampleProperties, SampleTableWriter};
/// # use isobmff_test_support::{movie_declaring, track};
///
/// // A writer for a movie of tracks 1 and 2
/// let movie = movie_declaring(vec![track(1), track(2)]);
/// let mut writer = SampleTableWriter::new(&movie);
///
/// // Two samples of track 1 in a chunk at 1000, then one of track 2 in a chunk at 1008
/// writer.begin_chunk(1_000)?;
/// writer.handle_sample(Sample::new(
///     SampleProperties {
///         track_id: 1,
///         decode_time: 0,
///         sample_duration: 1_024,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::ZERO,
///         sample_description_index: 1,
///     },
///     b"SAMP".to_vec(),
/// ))?;
/// writer.handle_sample(Sample::new(
///     SampleProperties {
///         track_id: 1,
///         decode_time: 1_024,
///         sample_duration: 1_024,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::ZERO,
///         sample_description_index: 1,
///     },
///     b"DATA".to_vec(),
/// ))?;
/// writer.finish_chunk()?;
/// writer.begin_chunk(1_008)?;
/// writer.handle_sample(Sample::new(
///     SampleProperties {
///         track_id: 2,
///         decode_time: 0,
///         sample_duration: 512,
///         sample_composition_time_offset: 0,
///         sample_flags: SampleFlags::ZERO,
///         sample_description_index: 1,
///     },
///     b"MORE".to_vec(),
/// ))?;
/// writer.finish_chunk()?;
///
/// // The bytes of the samples are handed over to be laid down in the order they came
/// let data: Vec<Vec<u8>> = std::iter::from_fn(|| writer.poll_data()).collect();
/// assert_eq!(data, [b"SAMP".to_vec(), b"DATA".to_vec(), b"MORE".to_vec()]);
///
/// // Each track gets the tables its samples were laid out in
/// writer.finish()?;
/// let (track_id, tables) = writer.poll_sample_tables().unwrap();
/// assert_eq!(track_id, 1);
/// assert_eq!(
///     *tables.stsc(),
///     SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 2, 1)])
/// );
/// let (track_id, tables) = writer.poll_sample_tables().unwrap();
/// assert_eq!(track_id, 2);
/// assert_eq!(
///     *tables.chunk_offsets(),
///     ChunkOffsets::Stco(ChunkOffsetBox::new(vec![ChunkOffsetEntry::new(1_008)]))
/// );
/// assert_eq!(writer.poll_sample_tables(), None);
/// # Ok::<(), isobmff_sample::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct SampleTableWriter {
    trak: Vec<TrackBox>,
    tracks: BTreeMap<u32, OpenTrack>,
    data: VecDeque<Vec<u8>>,
    tables: BTreeMap<u32, SampleTables>,
    state: State,
}

/// The sample tables of one track, as its samples were laid out in them
///
/// These are the tables a [`SampleTableBox`] (`stbl`, ISO/IEC 14496-12 §8.5.1)
/// must hold, every one but the `stsd`, which describes the samples rather
/// than laying them out, and which
/// [`into_sample_table`](Self::into_sample_table) takes to make the `stbl`
/// whole — and the optional tables the samples called for.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SampleTables {
    stts: TimeToSampleBox,
    stsc: SampleToChunkBox,
    sample_sizes: SampleSizes,
    chunk_offsets: ChunkOffsets,
    ctts: Option<CompositionOffsetBox>,
    stss: Option<SyncSampleBox>,
    sdtp: Option<SampleDependencyTypeBox>,
    padb: Option<PaddingBitsBox>,
    stdp: Option<DegradationPriorityBox>,
}

impl SampleTables {
    /// Returns the decode time of every sample, stated as deltas
    #[must_use]
    pub const fn stts(&self) -> &TimeToSampleBox {
        &self.stts
    }

    /// Returns the chunk each sample lies in
    #[must_use]
    pub const fn stsc(&self) -> &SampleToChunkBox {
        &self.stsc
    }

    /// Returns how many bytes each sample occupies, which the writer states in a `stsz`
    #[must_use]
    pub const fn sample_sizes(&self) -> &SampleSizes {
        &self.sample_sizes
    }

    /// Returns where every chunk of the track lies, at the width the offsets called for
    #[must_use]
    pub const fn chunk_offsets(&self) -> &ChunkOffsets {
        &self.chunk_offsets
    }

    /// Returns the offset from the decode time of every sample to its composition time, unless every one is zero
    #[must_use]
    pub const fn ctts(&self) -> Option<&CompositionOffsetBox> {
        self.ctts.as_ref()
    }

    /// Returns which samples are sync samples, unless every one is
    #[must_use]
    pub const fn stss(&self) -> Option<&SyncSampleBox> {
        self.stss.as_ref()
    }

    /// Returns how each sample depends on the others, unless no sample states it
    #[must_use]
    pub const fn sdtp(&self) -> Option<&SampleDependencyTypeBox> {
        self.sdtp.as_ref()
    }

    /// Returns how many bits at the end of each sample are padding, unless no sample has any
    #[must_use]
    pub const fn padb(&self) -> Option<&PaddingBitsBox> {
        self.padb.as_ref()
    }

    /// Returns the degradation priority of each sample, unless every one is zero
    #[must_use]
    pub const fn stdp(&self) -> Option<&DegradationPriorityBox> {
        self.stdp.as_ref()
    }

    /// Makes the `stbl` of the track out of these tables and the `stsd` describing its samples
    #[must_use]
    pub fn into_sample_table(self, stsd: SampleDescriptionBox) -> SampleTableBox {
        let mut stbl = SampleTableBox::new(
            stsd,
            self.stts,
            self.stsc,
            self.sample_sizes,
            self.chunk_offsets,
        );
        if let Some(ctts) = self.ctts {
            stbl = stbl.with_ctts(ctts);
        }
        if let Some(stss) = self.stss {
            stbl = stbl.with_stss(stss);
        }
        if let Some(sdtp) = self.sdtp {
            stbl = stbl.with_sdtp(sdtp);
        }
        if let Some(padb) = self.padb {
            stbl = stbl.with_padb(padb);
        }
        if let Some(stdp) = self.stdp {
            stbl = stbl.with_stdp(stdp);
        }

        stbl
    }
}

/// Where the writer stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Between chunks, waiting for the next one to be opened
    Between,
    /// Holding a chunk open, and taking the samples it carries
    Chunk(OpenChunk),
    /// Told the samples are over, and taking nothing more
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

/// Chunk being written, and what its samples settled once the first arrived
#[derive(Clone, Copy, Debug)]
struct OpenChunk {
    chunk_offset: u64,
    held: Option<HeldSamples>,
}

impl OpenChunk {
    /// Places `sample` at the end of this chunk, on the tables of its track in `tracks`, and hands its bytes back
    ///
    /// The first sample of the chunk is checked against `trak`, the tracks
    /// the movie declares.
    fn place(
        &mut self,
        sample: Sample,
        trak: &[TrackBox],
        tracks: &mut BTreeMap<u32, OpenTrack>,
    ) -> Result<Vec<u8>, Error> {
        let properties = sample.properties();
        let track_id = properties.track_id;
        let held = match &mut self.held {
            Some(held) => held,
            None => {
                let trak = trak
                    .iter()
                    .find(|trak| trak.tkhd().track_id() == track_id)
                    .ok_or(Error::UnknownTrackId { track_id })?;
                SampleDescriptions::new(trak)
                    .data_reference_index(properties.sample_description_index)?;
                self.held.insert(HeldSamples {
                    track_id,
                    sample_description_index: properties.sample_description_index,
                    sample_count: 0,
                })
            }
        };
        if held.track_id != track_id {
            return Err(Error::TrackIdMismatch {
                stated_track_id: track_id,
                established_track_id: held.track_id,
            });
        }
        if held.sample_description_index != properties.sample_description_index {
            return Err(Error::SampleDescriptionIndexMismatch {
                track_id,
                stated_sample_description_index: properties.sample_description_index,
                established_sample_description_index: held.sample_description_index,
            });
        }
        let data = tracks.entry(track_id).or_default().place(sample)?;
        held.sample_count = held.sample_count.saturating_add(1);

        Ok(data)
    }
}

/// What the samples of one chunk share, and how many of them it holds
#[derive(Clone, Copy, Debug)]
struct HeldSamples {
    track_id: u32,
    sample_description_index: u32,
    sample_count: u64,
}

impl SampleTableWriter {
    /// Creates a writer for the sample tables of `movie`, waiting for the first chunk
    #[must_use]
    pub fn new(movie: &MovieBox) -> Self {
        Self {
            trak: movie.trak().to_vec(),
            tracks: BTreeMap::new(),
            data: VecDeque::new(),
            tables: BTreeMap::new(),
            state: State::Between,
        }
    }

    /// Opens a chunk starting at `chunk_offset` in the file, which the samples handed over next lie in
    ///
    /// # Errors
    ///
    /// * [`ChunkStillOpen`](crate::Error::ChunkStillOpen): the chunk
    ///   opened before it was not closed by [`finish_chunk`](Self::finish_chunk).
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_chunk(&mut self, chunk_offset: u64) -> Result<(), Error> {
        self.writing()?;
        if matches!(self.state, State::Chunk(_)) {
            return Err(self.fail(Error::ChunkStillOpen));
        }
        self.state = State::Chunk(OpenChunk {
            chunk_offset,
            held: None,
        });

        Ok(())
    }

    /// Closes the chunk that is open: it holds the samples it was handed, and belongs to their track
    ///
    /// # Errors
    ///
    /// * [`NoChunkOpen`](crate::Error::NoChunkOpen): no chunk was
    ///   open to close.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish_chunk(&mut self) -> Result<(), Error> {
        self.writing()?;
        let State::Chunk(OpenChunk { chunk_offset, held }) = self.state else {
            return Err(self.fail(Error::NoChunkOpen));
        };
        if let Some(held) = held {
            let track = self.tracks.entry(held.track_id).or_default();
            track
                .chunks
                .push((held.sample_count, held.sample_description_index));
            track.chunk_offsets.push(chunk_offset);
        }
        self.state = State::Between;

        Ok(())
    }

    /// Takes a sample, places it at the end of the chunk that is open, and queues its bytes for [`poll_data`](Self::poll_data)
    ///
    /// # Errors
    ///
    /// * [`NoChunkOpen`](crate::Error::NoChunkOpen): no chunk was
    ///   opened to carry it.
    /// * [`UnknownTrackId`](crate::Error::UnknownTrackId): the movie
    ///   declares no track the sample belongs to.
    /// * [`UnknownSampleDescriptionIndex`](crate::Error::UnknownSampleDescriptionIndex):
    ///   the track has no `stsd` entry describing the sample.
    /// * The failures of [`SampleEntry::try_from`](isobmff_boxes::SampleEntry),
    ///   carried on [`Box`](crate::Error::Box): that entry does not read
    ///   as a sample entry, with `stsd` added to the containers.
    /// * [`UnknownDataReferenceIndex`](crate::Error::UnknownDataReferenceIndex):
    ///   that entry names a `dref` entry the track has none of.
    /// * [`ExternalDataReference`](crate::Error::ExternalDataReference):
    ///   the `dref` entry names a resource other than the file itself.
    /// * [`TrackIdMismatch`](crate::Error::TrackIdMismatch): the
    ///   sample belongs to another track than the chunk holds.
    /// * [`SampleDescriptionIndexMismatch`](crate::Error::SampleDescriptionIndexMismatch):
    ///   the sample is described by another `stsd` entry than the chunk holds.
    /// * [`DecodeTimeMismatch`](crate::Error::DecodeTimeMismatch):
    ///   the sample does not start where the one before it in its track ends.
    /// * [`DecodeTimeOverflow`](crate::Error::DecodeTimeOverflow):
    ///   the decode times of its track run past what 64 bits carry.
    /// * [`SampleSizeOutOfRange`](crate::Error::SampleSizeOutOfRange):
    ///   the sample is longer than the 32 bits an `stsz` entry states its
    ///   length in.
    /// * [`CompositionTimeOffsetOutOfRange`](crate::Error::CompositionTimeOffsetOutOfRange):
    ///   the sample states a composition time offset outside what 32 signed
    ///   bits hold.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<(), Error> {
        self.writing()?;
        let State::Chunk(chunk) = &mut self.state else {
            return Err(self.fail(Error::NoChunkOpen));
        };
        let data = chunk
            .place(sample, &self.trak, &mut self.tracks)
            .map_err(|failure| self.fail(failure))?;
        self.data.push_back(data);

        Ok(())
    }

    /// Hands over the bytes of the next sample, in the order the samples were handed over
    ///
    /// The writer holds the bytes of a sample until they are handed over here,
    /// and reports `None` once they are used up.
    pub fn poll_data(&mut self) -> Option<Vec<u8>> {
        self.data.pop_front()
    }

    /// Declares the samples over, and builds the sample tables of every track for [`poll_sample_tables`](Self::poll_sample_tables)
    ///
    /// # Errors
    ///
    /// * [`ChunkStillOpen`](crate::Error::ChunkStillOpen): a chunk
    ///   was left open.
    /// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange), carried on
    ///   [`Box`](crate::Error::Box): a chunk holds more samples than
    ///   an `stsc` entry counts, or is numbered past what one reaches.
    /// * [`AlreadyFinished`](crate::Error::AlreadyFinished): the
    ///   samples were already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.writing()?;
        if matches!(self.state, State::Chunk(_)) {
            return Err(self.fail(Error::ChunkStillOpen));
        }
        self.state = State::Finished;

        self.tables = mem::take(&mut self.tracks)
            .into_iter()
            .map(|(track_id, track)| Ok((track_id, track.into_tables(track_id)?)))
            .collect::<Result<_, _>>()
            .map_err(|failure| self.fail(failure))?;

        Ok(())
    }

    /// Hands over the sample tables of the next track, in the order of the track IDs, once the samples were declared over
    ///
    /// Reports `None` until [`finish`](Self::finish) has built them, and once
    /// they are used up. A track no sample was handed over to has none.
    pub fn poll_sample_tables(&mut self) -> Option<(u32, SampleTables)> {
        self.tables.pop_first()
    }

    /// Returns `Ok` while the writer still takes samples
    const fn writing(&self) -> Result<(), Error> {
        match self.state {
            State::Between | State::Chunk(_) => Ok(()),
            State::Finished => Err(Error::AlreadyFinished),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::iter;
    use core::num::NonZeroU32;

    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkLargeOffsetEntry, ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets,
        SampleFlags, SampleSizeBox, SampleSizeEntries, SampleSizeEntry, SampleSizes,
        SampleToChunkBox, SampleToChunkEntry, TimeToSampleBox, TimeToSampleEntry,
    };

    use isobmff_test_support::{movie_declaring, track};

    use super::{SampleTableWriter, SampleTables};
    use crate::error::Error;
    use crate::sample::{Sample, SampleProperties};

    /// Writer for the sample tables of a movie of tracks 1 and 2
    pub(super) fn writer() -> SampleTableWriter {
        SampleTableWriter::new(&movie_declaring(vec![track(1), track(2)]))
    }

    /// Sample of `track_id` at `decode_time` lasting 1024 units, carrying `data`
    pub(super) fn sample(track_id: u32, decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(
            SampleProperties {
                track_id,
                decode_time,
                sample_duration: 1_024,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::ZERO,
                sample_description_index: 1,
            },
            data.to_vec(),
        )
    }

    /// Lays `chunks` out, each `(chunk_offset, samples)`, and hands back the tables
    pub(super) fn laid_out(chunks: Vec<(u64, Vec<Sample>)>) -> BTreeMap<u32, SampleTables> {
        let mut writer = writer();

        for (chunk_offset, samples) in chunks {
            writer.begin_chunk(chunk_offset).unwrap();
            for sample in samples {
                writer.handle_sample(sample).unwrap();
            }
            writer.finish_chunk().unwrap();
        }
        writer.finish().unwrap();

        iter::from_fn(|| writer.poll_sample_tables()).collect()
    }

    #[test]
    fn the_bytes_of_the_samples_are_handed_over_in_the_order_they_came() {
        let mut writer = writer();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.handle_sample(sample(1, 1_024, b"BB")).unwrap();

        assert_eq!(
            [writer.poll_data(), writer.poll_data(), writer.poll_data()],
            [Some(b"AAAA".to_vec()), Some(b"BB".to_vec()), None]
        );
    }

    #[test]
    fn the_tables_are_handed_over_in_the_order_of_the_track_ids() {
        let mut writer = writer();

        for (chunk_offset, track_id) in [(1_000, 2), (2_000, 1)] {
            writer.begin_chunk(chunk_offset).unwrap();
            writer.handle_sample(sample(track_id, 0, b"AAAA")).unwrap();
            writer.finish_chunk().unwrap();
        }
        writer.finish().unwrap();

        assert_eq!(
            iter::from_fn(|| writer.poll_sample_tables())
                .map(|(track_id, _tables)| track_id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn a_chunk_begun_while_one_is_still_open_fails_the_writer() {
        let mut writer = writer();

        writer.begin_chunk(1_000).unwrap();

        assert_eq!(writer.begin_chunk(2_000), Err(Error::ChunkStillOpen));
        assert_eq!(writer.finish_chunk(), Err(Error::ChunkStillOpen));
    }

    #[test]
    fn samples_declared_over_while_a_chunk_is_still_open_fail_the_writer() {
        let mut writer = writer();

        writer.begin_chunk(1_000).unwrap();

        assert_eq!(writer.finish(), Err(Error::ChunkStillOpen));
        assert_eq!(writer.poll_sample_tables(), None);
    }

    #[test]
    fn a_chunk_closed_while_none_is_open_fails_the_writer() {
        let mut writer = writer();

        assert_eq!(writer.finish_chunk(), Err(Error::NoChunkOpen));
        assert_eq!(writer.begin_chunk(1_000), Err(Error::NoChunkOpen));
    }

    #[test]
    fn a_track_gets_the_four_tables_its_samples_were_laid_out_in() {
        let tables = laid_out(vec![
            (1_000, vec![sample(1, 0, b"AAAA"), sample(1, 1_024, b"BB")]),
            (2_000, vec![sample(1, 2_048, b"CCCC")]),
        ]);

        assert_eq!(
            tables,
            BTreeMap::from([(
                1,
                SampleTables {
                    stts: TimeToSampleBox::new(vec![TimeToSampleEntry::new(3, 1_024)]),
                    stsc: SampleToChunkBox::new(vec![
                        SampleToChunkEntry::new(1, 2, 1),
                        SampleToChunkEntry::new(2, 1, 1),
                    ]),
                    sample_sizes: SampleSizes::Stsz(SampleSizeBox::new(
                        SampleSizeEntries::PerSample(vec![
                            SampleSizeEntry::new(4),
                            SampleSizeEntry::new(2),
                            SampleSizeEntry::new(4),
                        ])
                    )),
                    chunk_offsets: ChunkOffsets::Stco(ChunkOffsetBox::new(vec![
                        ChunkOffsetEntry::new(1_000),
                        ChunkOffsetEntry::new(2_000),
                    ])),
                    ctts: None,
                    stss: None,
                    sdtp: None,
                    padb: None,
                    stdp: None,
                }
            )])
        );
    }

    #[test]
    fn each_chunk_goes_on_the_tables_of_the_track_its_samples_belong_to() {
        let tables = laid_out(vec![
            (1_000, vec![sample(1, 0, b"AAAA")]),
            (
                2_000,
                vec![sample(2, 0, b"BBBB"), sample(2, 1_024, b"CCCC")],
            ),
            (3_000, vec![sample(1, 1_024, b"DDDD")]),
        ]);

        assert_eq!(
            tables,
            BTreeMap::from([
                (
                    1,
                    SampleTables {
                        stts: TimeToSampleBox::new(vec![TimeToSampleEntry::new(2, 1_024)]),
                        stsc: SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 1, 1)]),
                        sample_sizes: SampleSizes::Stsz(SampleSizeBox::new(
                            SampleSizeEntries::Uniform {
                                sample_size: NonZeroU32::new(4).unwrap(),
                                sample_count: 2,
                            }
                        )),
                        chunk_offsets: ChunkOffsets::Stco(ChunkOffsetBox::new(vec![
                            ChunkOffsetEntry::new(1_000),
                            ChunkOffsetEntry::new(3_000),
                        ])),
                        ctts: None,
                        stss: None,
                        sdtp: None,
                        padb: None,
                        stdp: None,
                    }
                ),
                (
                    2,
                    SampleTables {
                        stts: TimeToSampleBox::new(vec![TimeToSampleEntry::new(2, 1_024)]),
                        stsc: SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 2, 1)]),
                        sample_sizes: SampleSizes::Stsz(SampleSizeBox::new(
                            SampleSizeEntries::Uniform {
                                sample_size: NonZeroU32::new(4).unwrap(),
                                sample_count: 2,
                            }
                        )),
                        chunk_offsets: ChunkOffsets::Stco(ChunkOffsetBox::new(vec![
                            ChunkOffsetEntry::new(2_000),
                        ])),
                        ctts: None,
                        stss: None,
                        sdtp: None,
                        padb: None,
                        stdp: None,
                    }
                ),
            ])
        );
    }

    #[test]
    fn a_chunk_no_sample_was_handed_over_to_is_not_recorded() {
        let tables = laid_out(vec![
            (1_000, vec![]),
            (2_000, vec![sample(1, 0, b"AAAA")]),
            (3_000, vec![]),
        ]);

        assert_eq!(
            tables,
            BTreeMap::from([(
                1,
                SampleTables {
                    stts: TimeToSampleBox::new(vec![TimeToSampleEntry::new(1, 1_024)]),
                    stsc: SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 1, 1)]),
                    sample_sizes: SampleSizes::Stsz(SampleSizeBox::new(
                        SampleSizeEntries::Uniform {
                            sample_size: NonZeroU32::new(4).unwrap(),
                            sample_count: 1,
                        }
                    )),
                    chunk_offsets: ChunkOffsets::Stco(ChunkOffsetBox::new(vec![
                        ChunkOffsetEntry::new(2_000),
                    ])),
                    ctts: None,
                    stss: None,
                    sdtp: None,
                    padb: None,
                    stdp: None,
                }
            )])
        );
    }

    #[test]
    fn no_samples_at_all_lay_out_no_track() {
        assert_eq!(laid_out(vec![]), BTreeMap::new());
    }

    #[test]
    fn a_sample_handed_over_while_no_chunk_is_open_is_refused() {
        let mut writer = writer();

        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Err(Error::NoChunkOpen)
        );
    }

    #[test]
    fn a_sample_of_another_track_than_the_chunk_holds_is_refused() {
        let mut writer = writer();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(sample(2, 0, b"BBBB")),
            Err(Error::TrackIdMismatch {
                stated_track_id: 2,
                established_track_id: 1
            })
        );
    }

    #[test]
    fn samples_of_one_chunk_described_by_two_entries_are_refused() {
        let described_by_the_second = Sample::new(
            SampleProperties {
                track_id: 1,
                decode_time: 1_024,
                sample_duration: 1_024,
                sample_composition_time_offset: 0,
                sample_flags: SampleFlags::ZERO,
                sample_description_index: 2,
            },
            b"BBBB".to_vec(),
        );
        let mut writer = writer();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(described_by_the_second),
            Err(Error::SampleDescriptionIndexMismatch {
                track_id: 1,
                stated_sample_description_index: 2,
                established_sample_description_index: 1
            })
        );
    }

    #[test]
    fn a_chunk_opened_past_what_32_bits_reach_is_placed_in_a_co64() {
        let tables = laid_out(vec![(1 << 32, vec![sample(1, 0, b"AAAA")])]);

        assert_eq!(
            tables,
            BTreeMap::from([(
                1,
                SampleTables {
                    stts: TimeToSampleBox::new(vec![TimeToSampleEntry::new(1, 1_024)]),
                    stsc: SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 1, 1)]),
                    sample_sizes: SampleSizes::Stsz(SampleSizeBox::new(
                        SampleSizeEntries::Uniform {
                            sample_size: NonZeroU32::new(4).unwrap(),
                            sample_count: 1,
                        }
                    )),
                    chunk_offsets: ChunkOffsets::Co64(ChunkLargeOffsetBox::new(vec![
                        ChunkLargeOffsetEntry::new(1 << 32),
                    ])),
                    ctts: None,
                    stss: None,
                    sdtp: None,
                    padb: None,
                    stdp: None,
                }
            )])
        );
    }

    #[test]
    fn anything_handed_over_after_the_samples_were_declared_over_is_refused() {
        let mut writer = writer();

        writer.finish().unwrap();

        assert_eq!(writer.begin_chunk(1_000), Err(Error::AlreadyFinished));
        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Err(Error::AlreadyFinished)
        );
        assert_eq!(writer.finish_chunk(), Err(Error::AlreadyFinished));
        assert_eq!(writer.finish(), Err(Error::AlreadyFinished));
    }

    #[test]
    fn a_failure_is_reported_again_for_every_call_after_it() {
        let mut writer = writer();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.handle_sample(sample(1, 512, b"BBBB")).unwrap_err();

        assert_eq!(
            writer.handle_sample(sample(1, 1_024, b"CCCC")),
            Err(Error::DecodeTimeMismatch {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024
            })
        );
        assert_eq!(
            writer.begin_chunk(2_000),
            Err(Error::DecodeTimeMismatch {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024
            })
        );
        assert_eq!(
            writer.finish(),
            Err(Error::DecodeTimeMismatch {
                track_id: 1,
                stated_decode_time: 512,
                reached_decode_time: 1_024
            })
        );
    }

    #[test]
    fn a_sample_the_movie_does_not_resolve_is_refused_where_it_is_handed_over() {
        let refused = |sample: Sample| {
            let mut writer = writer();
            writer.begin_chunk(1_000).unwrap();

            writer.handle_sample(sample)
        };

        assert_eq!(
            refused(sample(999, 0, b"AAAA")),
            Err(Error::UnknownTrackId { track_id: 999 })
        );
        assert_eq!(
            refused(Sample::new(
                SampleProperties {
                    track_id: 1,
                    decode_time: 0,
                    sample_duration: 1_024,
                    sample_composition_time_offset: 0,
                    sample_flags: SampleFlags::ZERO,
                    sample_description_index: 2
                },
                b"AAAA".to_vec()
            )),
            Err(Error::UnknownSampleDescriptionIndex {
                track_id: 1,
                sample_description_index: 2
            })
        );
    }
}
