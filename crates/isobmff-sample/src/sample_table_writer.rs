//! [`SampleTableWriter`], the samples of a presentation laid out as the sample tables of a movie, ISO/IEC 14496-12 §8.5.1 and §8.7

mod open_track;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::mem;

use isobmff_boxes::{
    ChunkOffsets, CompositionOffsetBox, DegradationPriorityBox, PaddingBitsBox,
    SampleDependencyTypeBox, SampleDescriptionBox, SampleSizeBox, SampleTableBox, SampleToChunkBox,
    SyncSampleBox, TimeToSampleBox,
};

use crate::error::Error;
use crate::sample::Sample;
use crate::sample_table_writer::open_track::OpenTrack;

/// Lays the samples of a presentation out as the sample tables of a movie
///
/// The writer takes the samples chunk by chunk — a chunk being a contiguous
/// set of samples of one track (ISO/IEC 14496-12 §3.1.2) — and hands the bytes
/// of each straight back, to be laid down where the caller opened the chunk.
/// What it keeps is what the sample tables of a track state about them: the
/// decode timeline (`stts`, §8.6.1.2), the chunks the samples lie in (`stsc`,
/// §8.7.4), their sizes (`stsz`, §8.7.3) and where each chunk starts (`stco`
/// or `co64`, §8.7.5), and the optional tables stating their composition time
/// offsets (`ctts`, §8.6.1.3) and the fields of their `sample_flags` (`sdtp`,
/// `padb`, `stss` and `stdp`, §8.8.3.1), which [`finish`](Self::finish) hands
/// back per track as [`SampleTables`]. The `stsd` of each track, and the movie
/// the tables go into, stay with the caller.
///
/// # Layout
///
/// * A chunk is opened by [`begin_chunk`](Self::begin_chunk) stating where in
///   the file its first byte lies, and holds the samples handed over until the
///   next chunk is opened or the samples are declared over. The samples of a
///   chunk lie one after another from there, and the chunk belongs to the
///   track of its first sample.
/// * The tables of a track count its chunks in the order they were opened and
///   its samples in the order they were handed over, so the chunks of a track
///   have to be opened at rising offsets for
///   [`sample_table::sample_extents`](crate::sample_table::sample_extents) to
///   hand the samples back in that order.
/// * Each table is stated the way its box chooses from the values laid down:
///   [`SampleSizeBox::from_sizes`], [`TimeToSampleBox::from_deltas`],
///   [`SampleToChunkBox::from_chunks`] and [`ChunkOffsets::from_offsets`].
/// * An optional table is left out when every sample of the track states what
///   its absence does: the `ctts` when every offset is zero, the `stss` when
///   every sample is a sync sample, and the `sdtp`, the `padb` and the `stdp`
///   when every field they state is zero. A track whose samples are none of
///   them sync samples carries an `stss` listing none.
///
/// # Contract
///
/// * Handing a sample over while no chunk is open is
///   [`NoChunkOpen`](crate::ErrorKind::NoChunkOpen), and one of another
///   track than the chunk holds is
///   [`TrackIdMismatch`](crate::ErrorKind::TrackIdMismatch). A chunk no
///   sample was handed over to is not recorded.
/// * The decode timeline of a track starts at zero and states how long each
///   sample lasts, never when it is decoded, so a sample of a track has to
///   start where the one before it ends — the first at zero — which is
///   otherwise
///   [`DecodeTimeMismatch`](crate::ErrorKind::DecodeTimeMismatch).
/// * The samples of a chunk are all described by one `stsd` entry, which the
///   run of chunks states for them (§8.7.4): a chunk mixing two is
///   [`SampleDescriptionIndexMismatch`](crate::ErrorKind::SampleDescriptionIndexMismatch).
/// * A sample stating a composition time offset no version of a `ctts`
///   writes is
///   [`CompositionTimeOffsetOutOfRange`](crate::ErrorKind::CompositionTimeOffsetOutOfRange),
///   and one setting a reserved bit of its `sample_flags`, which no table
///   states,
///   [`UnsupportedSampleFlags`](crate::ErrorKind::UnsupportedSampleFlags).
/// * A chunk holding more samples than an `stsc` entry counts, or numbered
///   past what one reaches, is reported by [`finish`](Self::finish), where the
///   tables are built: the failure of the box, carried on
///   [`Box`](crate::ErrorKind::Box). So is a track stating a negative
///   composition time offset and one past [`i32::MAX`], which no one version
///   of a `ctts` writes both of:
///   [`CompositionTimeOffsetOutOfRange`](crate::ErrorKind::CompositionTimeOffsetOutOfRange).
/// * An `Err` leaves the writer failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside: every
///   later call reports that same failure again.
/// * [`finish`](Self::finish) declares the samples over and hands back the
///   tables. A chunk opened or a sample handed over then, or a second
///   [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff_boxes::{
///     ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets, SampleToChunkBox, SampleToChunkEntry,
/// };
/// use isobmff_sample::{Sample, SampleTableWriter};
///
/// let mut writer = SampleTableWriter::new();
///
/// // Two samples of track 1 in a chunk at 1000, then one of track 2 in a chunk at 1008
/// writer.begin_chunk(1_000)?;
/// assert_eq!(writer.handle_sample(Sample::new(1, 0, 1_024, 0, 0, 1, b"SAMP".to_vec()))?, b"SAMP");
/// assert_eq!(writer.handle_sample(Sample::new(1, 1_024, 1_024, 0, 0, 1, b"DATA".to_vec()))?, b"DATA");
/// writer.begin_chunk(1_008)?;
/// writer.handle_sample(Sample::new(2, 0, 512, 0, 0, 1, b"MORE".to_vec()))?;
///
/// // Each track gets the tables its samples were laid out in
/// let tables = writer.finish()?;
/// assert_eq!(
///     *tables[&1].stsc(),
///     SampleToChunkBox::new(vec![SampleToChunkEntry::new(1, 2, 1)])
/// );
/// assert_eq!(
///     *tables[&2].chunk_offsets(),
///     ChunkOffsets::Stco(ChunkOffsetBox::new(vec![ChunkOffsetEntry::new(1_008)]))
/// );
/// # Ok::<(), isobmff_sample::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct SampleTableWriter {
    tracks: BTreeMap<u32, OpenTrack>,
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
    stsz: SampleSizeBox,
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

    /// Returns how many bytes each sample occupies
    #[must_use]
    pub const fn stsz(&self) -> &SampleSizeBox {
        &self.stsz
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
        let mut stbl =
            SampleTableBox::new(stsd, self.stts, self.stsc, self.stsz, self.chunk_offsets);
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
    /// Waiting for the first chunk to be opened
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
    fn place(
        &mut self,
        sample: Sample,
        tracks: &mut BTreeMap<u32, OpenTrack>,
    ) -> Result<Vec<u8>, Error> {
        let track_id = sample.track_id();
        let held = self.held.get_or_insert(HeldSamples {
            track_id,
            sample_description_index: sample.sample_description_index(),
            sample_count: 0,
        });
        if held.track_id != track_id {
            return Err(Error::track_id_mismatch(track_id, held.track_id));
        }
        if held.sample_description_index != sample.sample_description_index() {
            return Err(Error::sample_description_index_mismatch(
                track_id,
                sample.sample_description_index(),
                held.sample_description_index,
            ));
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
    /// Creates a writer waiting for the first chunk
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tracks: BTreeMap::new(),
            state: State::Between,
        }
    }

    /// Opens a chunk starting at `chunk_offset` in the file, which the samples handed over next lie in
    ///
    /// The chunk open before it, if any, is closed: it holds the samples it
    /// was handed, and belongs to their track.
    ///
    /// # Errors
    ///
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn begin_chunk(&mut self, chunk_offset: u64) -> Result<(), Error> {
        self.writing()?;
        self.close_chunk();
        self.state = State::Chunk(OpenChunk {
            chunk_offset,
            held: None,
        });

        Ok(())
    }

    /// Takes a sample, places it at the end of the chunk that is open, and hands its bytes back
    ///
    /// # Errors
    ///
    /// * [`NoChunkOpen`](crate::ErrorKind::NoChunkOpen): no chunk was
    ///   opened to carry it.
    /// * [`TrackIdMismatch`](crate::ErrorKind::TrackIdMismatch): the
    ///   sample belongs to another track than the chunk holds.
    /// * [`SampleDescriptionIndexMismatch`](crate::ErrorKind::SampleDescriptionIndexMismatch):
    ///   the sample is described by another `stsd` entry than the chunk holds.
    /// * [`DecodeTimeMismatch`](crate::ErrorKind::DecodeTimeMismatch):
    ///   the sample does not start where the one before it in its track ends.
    /// * [`DecodeTimeOverflow`](crate::ErrorKind::DecodeTimeOverflow):
    ///   the decode times of its track run past what 64 bits carry.
    /// * [`SampleSizeOutOfRange`](crate::ErrorKind::SampleSizeOutOfRange):
    ///   the sample is longer than the 32 bits an `stsz` entry states its
    ///   length in.
    /// * [`CompositionTimeOffsetOutOfRange`](crate::ErrorKind::CompositionTimeOffsetOutOfRange):
    ///   the sample states a composition time offset neither version of a
    ///   `ctts` writes.
    /// * [`UnsupportedSampleFlags`](crate::ErrorKind::UnsupportedSampleFlags):
    ///   the sample sets a reserved bit of its `sample_flags`.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn handle_sample(&mut self, sample: Sample) -> Result<Vec<u8>, Error> {
        self.writing()?;
        let State::Chunk(chunk) = &mut self.state else {
            return Err(self.fail(Error::no_chunk_open()));
        };
        chunk
            .place(sample, &mut self.tracks)
            .map_err(|failure| self.fail(failure))
    }

    /// Declares the samples over, and hands back the sample tables of every track
    ///
    /// The chunk that is open, if any, is closed first.
    ///
    /// # Errors
    ///
    /// * [`OutOfRange`](isobmff_core::ErrorKind::OutOfRange), carried on
    ///   [`Box`](crate::ErrorKind::Box): a chunk holds more samples than
    ///   an `stsc` entry counts, or is numbered past what one reaches.
    /// * [`CompositionTimeOffsetOutOfRange`](crate::ErrorKind::CompositionTimeOffsetOutOfRange):
    ///   a track states a negative composition time offset and one past
    ///   [`i32::MAX`], which no one version of a `ctts` writes both of; the
    ///   failure names the widest.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   samples were already declared over.
    /// * The failure of a previous call, which the writer keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<BTreeMap<u32, SampleTables>, Error> {
        self.writing()?;
        self.close_chunk();
        self.state = State::Finished;

        mem::take(&mut self.tracks)
            .into_iter()
            .map(|(track_id, track)| Ok((track_id, track.into_tables(track_id)?)))
            .collect::<Result<_, _>>()
            .map_err(|failure| self.fail(failure))
    }

    /// Closes the chunk that is open, putting it on the tables of the track it belongs to
    fn close_chunk(&mut self) {
        if let State::Chunk(OpenChunk {
            chunk_offset,
            held: Some(held),
        }) = self.state
        {
            let track = self.tracks.entry(held.track_id).or_default();
            track
                .chunks
                .push((held.sample_count, held.sample_description_index));
            track.chunk_offsets.push(chunk_offset);
        }
    }

    /// Returns `Ok` while the writer still takes samples
    const fn writing(&self) -> Result<(), Error> {
        match self.state {
            State::Between | State::Chunk(_) => Ok(()),
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the writer for good, and hands the failure back to report
    fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

impl Default for SampleTableWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use isobmff_boxes::{
        ChunkLargeOffsetBox, ChunkLargeOffsetEntry, ChunkOffsetBox, ChunkOffsetEntry, ChunkOffsets,
        SampleSizeBox, SampleSizeEntry, SampleSizes, SampleToChunkBox, SampleToChunkEntry,
        TimeToSampleBox, TimeToSampleEntry,
    };

    use super::{SampleTableWriter, SampleTables};
    use crate::error::Error;
    use crate::sample::Sample;

    /// Sample of `track_id` at `decode_time` lasting 1024 units, carrying `data`
    pub(super) fn sample(track_id: u32, decode_time: u64, data: &[u8]) -> Sample {
        Sample::new(track_id, decode_time, 1_024, 0, 0, 1, data.to_vec())
    }

    /// Lays `chunks` out, each `(chunk_offset, samples)`, and hands back the tables
    pub(super) fn laid_out(chunks: Vec<(u64, Vec<Sample>)>) -> BTreeMap<u32, SampleTables> {
        let mut writer = SampleTableWriter::new();

        for (chunk_offset, samples) in chunks {
            writer.begin_chunk(chunk_offset).unwrap();
            for sample in samples {
                writer.handle_sample(sample).unwrap();
            }
        }

        writer.finish().unwrap()
    }

    #[test]
    fn the_bytes_of_a_sample_are_handed_straight_back() {
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();

        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Ok(b"AAAA".to_vec())
        );
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
                    stsz: SampleSizeBox::new(SampleSizes::PerSample(vec![
                        SampleSizeEntry::new(4),
                        SampleSizeEntry::new(2),
                        SampleSizeEntry::new(4),
                    ])),
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
                        stsz: SampleSizeBox::new(SampleSizes::Uniform {
                            sample_size: NonZeroU32::new(4).unwrap(),
                            sample_count: 2,
                        }),
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
                        stsz: SampleSizeBox::new(SampleSizes::Uniform {
                            sample_size: NonZeroU32::new(4).unwrap(),
                            sample_count: 2,
                        }),
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
                    stsz: SampleSizeBox::new(SampleSizes::Uniform {
                        sample_size: NonZeroU32::new(4).unwrap(),
                        sample_count: 1,
                    }),
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
        let mut writer = SampleTableWriter::new();

        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Err(Error::no_chunk_open())
        );
    }

    #[test]
    fn a_sample_of_another_track_than_the_chunk_holds_is_refused() {
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(sample(2, 0, b"BBBB")),
            Err(Error::track_id_mismatch(2, 1))
        );
    }

    #[test]
    fn samples_of_one_chunk_described_by_two_entries_are_refused() {
        let described_by_the_second = Sample::new(1, 1_024, 1_024, 0, 0, 2, b"BBBB".to_vec());
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();

        assert_eq!(
            writer.handle_sample(described_by_the_second),
            Err(Error::sample_description_index_mismatch(1, 2, 1))
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
                    stsz: SampleSizeBox::new(SampleSizes::Uniform {
                        sample_size: NonZeroU32::new(4).unwrap(),
                        sample_count: 1,
                    }),
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
        let mut writer = SampleTableWriter::new();

        writer.finish().unwrap();

        assert_eq!(writer.begin_chunk(1_000), Err(Error::already_finished()));
        assert_eq!(
            writer.handle_sample(sample(1, 0, b"AAAA")),
            Err(Error::already_finished())
        );
        assert_eq!(writer.finish(), Err(Error::already_finished()));
    }

    #[test]
    fn a_failure_is_reported_again_for_every_call_after_it() {
        let mut writer = SampleTableWriter::new();

        writer.begin_chunk(1_000).unwrap();
        writer.handle_sample(sample(1, 0, b"AAAA")).unwrap();
        writer.handle_sample(sample(1, 512, b"BBBB")).unwrap_err();

        assert_eq!(
            writer.handle_sample(sample(1, 1_024, b"CCCC")),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
        assert_eq!(
            writer.begin_chunk(2_000),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
        assert_eq!(
            writer.finish(),
            Err(Error::decode_time_mismatch(1, 512, 1_024))
        );
    }
}
