//! [`Demuxer`], which reads a movie file or a segment from the bytes a caller reads

mod file_level;

use isobmff_boxes::{FileTypeBox, MovieBox, MovieFragmentRandomAccessBox, SegmentTypeBox};
use isobmff_core::BoxHeader;

use crate::byte_range::ByteRange;
use crate::demuxer::file_level::{Handling, Value};
use crate::error::DemuxError;
use crate::framing::{BoxEvent, BoxEvents, BoxReader};
use crate::limits::DemuxLimits;
use crate::structure_data::{SampleLocations, SegmentIndex, StructureDataReader};

/// Reader of a movie file or a segment: the boxes at file level, read into their values, and the samples they declare
///
/// The demuxer does no I/O. It names the bytes it wants next as
/// [`wanted_input`](Self::wanted_input), takes them with the file offset they
/// start at in [`handle_input`](Self::handle_input), and reports what they
/// hold as [`DemuxEvent`]s. Bytes may arrive cut anywhere.
///
/// The boxes of structure-data (`moov`, `moof`, `sidx`, `mfra`) and the
/// brands (`ftyp`, `styp`) are gathered and read into their values, which the
/// demuxer keeps for its accessors; a `moov` or a `moof` is reported with the
/// [`SampleLocation`](crate::SampleLocation)s it declares. Each media data
/// box is reported by where it lies, and waits for the caller to
/// [`skip`](Self::skip) it or [`stream`](Self::stream) it. Any other box is
/// skipped (ISO/IEC 14496-12 §4.2.2).
///
/// The demuxer holds no rule on the order of the boxes: a `moof` before the
/// `moov` fails as the structure-data it declares does not resolve.
///
/// A failure is returned by the call that reads it, and leaves the demuxer
/// failed: it wants no more input, and every call after it returns the same
/// failure. The exceptions refuse one call and leave the demuxer as it was,
/// as [`DemuxError`] sets out.
///
/// The crate documentation shows the loops of a caller that can seek, of one
/// that cannot, and of one that resumes at a later box.
#[derive(Debug)]
pub struct Demuxer {
    boxes: BoxReader,
    structure_data: StructureDataReader,
    payload_limit_bytes: u64,
    file_type: Option<FileTypeBox>,
    segment_type: Option<SegmentTypeBox>,
}

impl Demuxer {
    /// Creates a demuxer that wants the file from its first byte, and reads it up to the default [`DemuxLimits`]
    #[must_use]
    pub const fn new() -> Self {
        Self::with_limits(DemuxLimits::new())
    }

    /// Creates a demuxer that wants the file from its first byte, and reads it up to `limits`
    #[must_use]
    pub const fn with_limits(limits: DemuxLimits) -> Self {
        Self {
            boxes: BoxReader::new(),
            structure_data: StructureDataReader::new(limits.structure_data()),
            payload_limit_bytes: limits.payload_bytes(),
            file_type: None,
            segment_type: None,
        }
    }

    /// Returns the bytes the demuxer wants next, or `None` once it wants no more
    ///
    /// The wanted bytes are those of [`BoxReader::wanted_input`]: once the
    /// demuxer skips a box, it wants the file offset past it.
    #[must_use]
    pub fn wanted_input(&self) -> Option<ByteRange> {
        self.boxes.wanted_input()
    }

    /// Takes `input`, the bytes of the file from file offset `offset`, and returns what they hold
    ///
    /// `input` is taken as [`BoxReader::handle_input`] takes it: from the
    /// offset [`wanted_input`](Self::wanted_input) names, which it must start
    /// at or before, and an input that ends before that offset holds no
    /// events. The events borrow `input` alone, so the caller answers a
    /// [`DemuxEvent::MediaData`] on the demuxer while it goes through them.
    /// They stop where the reader of the boxes stops, so the caller passes the
    /// rest of its input again from the offset wanted next.
    ///
    /// A failure is returned by the call that reads it, in place of the events
    /// before it.
    ///
    /// # Errors
    ///
    /// * [`UnwantedInput`](DemuxError::UnwantedInput),
    ///   [`UndecidedBox`](DemuxError::UndecidedBox): `input` starts past the
    ///   wanted offset, or a [`DemuxEvent::MediaData`] is not answered yet. The
    ///   demuxer is left as it was.
    /// * The failures of [`BoxReader::handle_input`], and of
    ///   [`BoxReader::gather`] with the limit of the payload of a box.
    /// * [`Box`](DemuxError::Box): a box gathered does not read as its value.
    /// * The failures of the [`StructureDataReader`] the boxes of
    ///   structure-data are handed to.
    /// * The failure the demuxer failed with before.
    pub fn handle_input<'a>(
        &mut self,
        offset: u64,
        input: &'a [u8],
    ) -> Result<DemuxEvents<'a>, DemuxError> {
        // Why not `offset`: input may start before the wanted offset, and a
        // payload, which comes only first among the events of a call, starts
        // where the reader wanted its input.
        let payload_offset = self.boxes.wanted_input().map_or(offset, ByteRange::offset);
        let box_events = self.boxes.handle_input(offset, input)?;
        self.demux(box_events, payload_offset)
    }

    /// Ends the file at the offset the demuxer wants, and returns the events of the box it ends
    ///
    /// A box whose size runs to the end of the file ends here, so the events go
    /// where those of [`handle_input`](Self::handle_input) go: a `moov` that
    /// runs to the end of the file is reported here. The demuxer then wants no
    /// more input.
    ///
    /// # Errors
    ///
    /// * The failures of [`BoxReader::finish`], and those of reading a box
    ///   into its value as for [`handle_input`](Self::handle_input).
    /// * The failure the demuxer failed with before.
    pub fn finish(&mut self) -> Result<DemuxEvents<'static>, DemuxError> {
        let box_events = self.boxes.finish()?;
        // Why not the wanted offset: `finish` takes no input, so its events hold
        // no payload to place.
        self.demux(box_events, 0)
    }

    /// Skips the media data box just reported: the demuxer next wants the file offset just past it
    ///
    /// # Errors
    ///
    /// * The failures of [`BoxReader::skip`]:
    ///   [`NoOpenBox`](DemuxError::NoOpenBox) when no
    ///   [`DemuxEvent::MediaData`] waits for an answer.
    /// * The failure the demuxer failed with before.
    pub fn skip(&mut self) -> Result<(), DemuxError> {
        self.boxes.skip()
    }

    /// Streams the payload of the media data box just reported as [`DemuxEvent::MediaDataPayload`]s
    ///
    /// # Errors
    ///
    /// * The failures of [`BoxReader::stream`]:
    ///   [`NoOpenBox`](DemuxError::NoOpenBox) when no
    ///   [`DemuxEvent::MediaData`] waits for an answer.
    /// * The failure the demuxer failed with before.
    pub fn stream(&mut self) -> Result<(), DemuxError> {
        self.boxes.stream()
    }

    /// Resumes reading at file offset `offset`, where a box after the `moov` starts, as after a seek
    ///
    /// The demuxer forgets the decoding time each track has reached, so a
    /// `moof` read after it resolves only where its `tfdt` gives the decoding
    /// time (§8.8.12). The movie, the segment indexes, the movie fragment
    /// random access box and the brands are kept, so a `moov` read again fails
    /// with [`DuplicateBox`](DemuxError::DuplicateBox). A
    /// [`MediaDataReader`](crate::media_data::MediaDataReader) the caller fed
    /// is for the caller to [`clear`](crate::media_data::MediaDataReader::clear).
    ///
    /// # Errors
    ///
    /// * The failure the demuxer failed with before, which no resuming
    ///   undoes.
    pub fn resume_at(&mut self, offset: u64) -> Result<(), DemuxError> {
        if let Some(failure) = self.boxes.failure() {
            return Err(failure);
        }
        self.boxes = BoxReader::starting_at(offset);
        self.structure_data.resume();
        Ok(())
    }

    /// Returns the `ftyp` read last, §5.2
    ///
    /// A file without one is to be read as one of the brand `mp41` (§5.2.1).
    #[must_use]
    pub const fn file_type(&self) -> Option<&FileTypeBox> {
        self.file_type.as_ref()
    }

    /// Returns the `styp` read last, the one of the segment being read, §8.14.2
    #[must_use]
    pub const fn segment_type(&self) -> Option<&SegmentTypeBox> {
        self.segment_type.as_ref()
    }

    /// Returns the movie, once its `moov` is read
    #[must_use]
    pub const fn movie(&self) -> Option<&MovieBox> {
        self.structure_data.movie()
    }

    /// Returns the segment indexes of the `sidx`es read, in the order they were read
    ///
    /// A segment index read again, as it is after resuming before its `sidx`,
    /// is kept once.
    #[must_use]
    pub fn segment_indexes(&self) -> &[SegmentIndex] {
        self.structure_data.segment_indexes()
    }

    /// Returns the `mfra` read last, §8.8.9
    #[must_use]
    pub const fn movie_fragment_random_access(&self) -> Option<&MovieFragmentRandomAccessBox> {
        self.structure_data.movie_fragment_random_access()
    }

    /// Makes the events of the reader of the boxes into those of the demuxer, a payload among them at file offset `payload_offset`
    fn demux<'a>(
        &mut self,
        box_events: BoxEvents<'a>,
        payload_offset: u64,
    ) -> Result<DemuxEvents<'a>, DemuxError> {
        let mut events = DemuxEvents::default();
        for box_event in box_events {
            let event = match box_event {
                BoxEvent::Header(header) => self.open(header),
                BoxEvent::Payload(bytes) => Ok(Some(DemuxEvent::MediaDataPayload {
                    offset: payload_offset,
                    bytes,
                })),
                BoxEvent::End => self.close(),
            };
            match event {
                Ok(event) => events.push(event),
                Err(failure) => {
                    self.boxes.fail_with(failure);
                    return Err(failure);
                }
            }
        }
        // Why not return the events first, as the reader of the boxes does: it
        // then wants no more input, so a caller that reads while input is
        // wanted would end without the failure.
        self.boxes.failure().map_or(Ok(events), Err)
    }

    /// Answers the header of a box at file level as its type says, and returns the event it is reported with
    fn open(&mut self, header: BoxHeader) -> Result<Option<DemuxEvent<'static>>, DemuxError> {
        let byte_range = self.boxes.open_box().ok_or(DemuxError::NoOpenBox)?;
        match file_level::handling(header.box_type()) {
            Handling::Gather => {
                self.boxes.gather(self.payload_limit_bytes)?;
                Ok(None)
            }
            Handling::Report => Ok(Some(DemuxEvent::MediaData { header, byte_range })),
            Handling::Skip => {
                self.boxes.skip()?;
                Ok(Some(DemuxEvent::Other { header, byte_range }))
            }
        }
    }

    /// Reads the box just gathered, if any, into its value, and returns the event it is reported with
    fn close(&mut self) -> Result<Option<DemuxEvent<'static>>, DemuxError> {
        let (Some(raw_box), Some(byte_range)) =
            (self.boxes.gathered_box(), self.boxes.gathered_box_range())
        else {
            return Ok(None);
        };
        let Some(value) = file_level::read(raw_box)? else {
            return Ok(None);
        };
        let event = match value {
            Value::FileType(ftyp) => {
                self.file_type = Some(ftyp);
                DemuxEvent::FileType
            }
            Value::SegmentType(styp) => {
                self.segment_type = Some(styp);
                DemuxEvent::SegmentType
            }
            Value::Movie(moov) => DemuxEvent::Movie {
                samples: self.structure_data.handle_movie(moov)?,
            },
            Value::MovieFragment(moof) => DemuxEvent::MovieFragment {
                samples: self
                    .structure_data
                    .handle_movie_fragment(&moof, byte_range.offset())?,
            },
            Value::SegmentIndex(sidx) => {
                let anchor = byte_range
                    .offset()
                    .saturating_add(byte_range.length().unwrap_or_default());
                self.structure_data.handle_segment_index(&sidx, anchor)?;
                DemuxEvent::SegmentIndex
            }
            Value::MovieFragmentRandomAccess(mfra) => {
                self.structure_data
                    .handle_movie_fragment_random_access(mfra);
                DemuxEvent::MovieFragmentRandomAccess
            }
        };

        Ok(Some(event))
    }
}

impl Default for Demuxer {
    fn default() -> Self {
        Self::new()
    }
}

/// What the bytes of a file hold, as a [`Demuxer`] reports it
///
/// An event says what was read; the values read are the demuxer's to return,
/// so an event borrows the input it was read from and never the demuxer.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum DemuxEvent<'a> {
    /// `ftyp` read, §5.2: [`Demuxer::file_type`] returns it
    FileType,
    /// `styp` read, §8.14.2: [`Demuxer::segment_type`] returns it
    SegmentType,
    /// `moov` read, §8.2.1: [`Demuxer::movie`] returns it
    #[non_exhaustive]
    Movie {
        /// Locations of the samples its sample tables declare, in the order their media data lies
        samples: SampleLocations,
    },
    /// `moof` read, §8.8.4
    #[non_exhaustive]
    MovieFragment {
        /// Locations of the samples its track fragment runs declare, in the order their media data lies
        samples: SampleLocations,
    },
    /// `sidx` read, §8.14.3: [`Demuxer::segment_indexes`] returns its index
    SegmentIndex,
    /// `mfra` read, §8.8.9: [`Demuxer::movie_fragment_random_access`] returns it
    MovieFragmentRandomAccess,
    /// Header of a media data box, `mdat` (§8.1.1) or `imda` (§8.1.4), which waits for [`Demuxer::skip`] or [`Demuxer::stream`]
    #[non_exhaustive]
    MediaData {
        /// Header of the box
        header: BoxHeader,
        /// Where the whole box lies, its length `None` when it runs to the end of the file
        byte_range: ByteRange,
    },
    /// Piece of the payload of the media data box being streamed
    #[non_exhaustive]
    MediaDataPayload {
        /// File offset of the first byte of the piece
        offset: u64,
        /// Bytes of the piece, borrowed from the input
        bytes: &'a [u8],
    },
    /// Any other box, skipped, §4.2.2
    #[non_exhaustive]
    Other {
        /// Header of the box
        header: BoxHeader,
        /// Where the whole box lies, its length `None` when it runs to the end of the file
        byte_range: ByteRange,
    },
}

/// Events the bytes given to a [`Demuxer`] hold, in file order
///
/// Returned by [`Demuxer::handle_input`] and [`Demuxer::finish`]. They borrow
/// the input, never the demuxer.
#[derive(Clone, Debug, Default)]
#[must_use = "the events report the movie and its samples, and a media data box waits for an answer"]
pub struct DemuxEvents<'a> {
    // Why not a `Vec`: the events of the reader of the boxes in one call are
    // at most a payload, the end of its box and one header, and only the
    // payload and the header make an event here, so a call allocates nothing.
    events: [Option<DemuxEvent<'a>>; 2],
}

impl<'a> DemuxEvents<'a> {
    fn push(&mut self, event: Option<DemuxEvent<'a>>) {
        let Some(event) = event else {
            return;
        };
        let slot = self.events.iter_mut().find(|slot| slot.is_none());
        debug_assert!(slot.is_some(), "more events than a call can make");
        if let Some(slot) = slot {
            *slot = Some(event);
        }
    }
}

impl<'a> Iterator for DemuxEvents<'a> {
    type Item = DemuxEvent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.events.iter_mut().find_map(Option::take)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::ops::Range;

    use isobmff_boxes::{
        FileTypeBox, MovieExtendsBox, MovieFragmentBox, SegmentIndexBox, SegmentIndexReference,
        SegmentTypeBox, TrackExtendsBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentBox,
        TrackRunBox, TrackRunSamples,
    };
    use isobmff_core::{BoxDefinition, BoxEncode, BoxHeader, BoxType, FourCc};

    use super::{DemuxEvent, Demuxer};
    use crate::byte_range::ByteRange;
    use crate::error::DemuxError;
    use crate::framing::to_u64;
    use crate::limits::DemuxLimits;
    use crate::structure_data::fixtures::track;

    /// What a caller saw of an event, the samples by track, decoding time and byte range
    #[derive(PartialEq, Debug)]
    enum Seen {
        FileType,
        SegmentType,
        Movie(Vec<(u32, u64, Range<u64>)>),
        MovieFragment(Vec<(u32, u64, Range<u64>)>),
        SegmentIndex,
        MovieFragmentRandomAccess,
        MediaData(ByteRange),
        /// Payload of a media data box from a file offset, its pieces joined
        Payload(u64, Vec<u8>),
        Other(BoxType, ByteRange),
    }

    /// What a caller saw reading a file
    #[derive(Debug)]
    struct Read {
        seen: Vec<Seen>,
        /// File offsets the demuxer wanted input from
        wanted: Vec<u64>,
    }

    /// Reads `file` from wherever `demuxer` wants it, `piece_len` bytes at a time, streaming each media data box if `stream`, else skipping it
    fn read(
        demuxer: &mut Demuxer,
        file: &[u8],
        piece_len: usize,
        stream: bool,
    ) -> Result<Read, DemuxError> {
        let mut read = Read {
            seen: Vec::new(),
            wanted: Vec::new(),
        };
        while let Some(wanted) = demuxer.wanted_input() {
            read.wanted.push(wanted.offset());
            let rest = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap_or_default();
            let input = rest.get(..piece_len.min(rest.len())).unwrap();
            if input.is_empty() {
                break;
            }
            for event in demuxer.handle_input(wanted.offset(), input)? {
                if let DemuxEvent::MediaData { .. } = event {
                    if stream {
                        demuxer.stream()?;
                    } else {
                        demuxer.skip()?;
                    }
                }
                see(&mut read.seen, event);
            }
        }
        for event in demuxer.finish()? {
            see(&mut read.seen, event);
        }

        Ok(read)
    }

    fn see(seen: &mut Vec<Seen>, event: DemuxEvent<'_>) {
        let placed = |samples: super::SampleLocations| {
            samples
                .map(|location| {
                    (
                        location.properties().track_id,
                        location.properties().decoding_time,
                        location.byte_range(),
                    )
                })
                .collect()
        };
        let event = match event {
            DemuxEvent::FileType => Seen::FileType,
            DemuxEvent::SegmentType => Seen::SegmentType,
            DemuxEvent::Movie { samples } => Seen::Movie(placed(samples)),
            DemuxEvent::MovieFragment { samples } => Seen::MovieFragment(placed(samples)),
            DemuxEvent::SegmentIndex => Seen::SegmentIndex,
            DemuxEvent::MovieFragmentRandomAccess => Seen::MovieFragmentRandomAccess,
            DemuxEvent::MediaData { byte_range, .. } => Seen::MediaData(byte_range),
            DemuxEvent::MediaDataPayload { offset, bytes } => {
                if let Some(Seen::Payload(start, joined)) = seen.last_mut()
                    && start.saturating_add(to_u64(joined.len())) == offset
                {
                    joined.extend_from_slice(bytes);
                    return;
                }
                Seen::Payload(offset, bytes.to_vec())
            }
            DemuxEvent::Other { header, byte_range } => Seen::Other(header.box_type(), byte_range),
        };
        seen.push(event);
    }

    /// Returns the movie of `tracks`, ready to encode
    fn movie(tracks: impl IntoIterator<Item = isobmff_boxes::TrackBox>) -> isobmff_boxes::MovieBox {
        let mut movie = crate::structure_data::fixtures::movie(tracks);
        movie.mvhd.next_track_id = 2;
        movie
    }

    /// Returns `value` encoded as a box
    fn encoded(value: &(impl BoxEncode + BoxDefinition)) -> Vec<u8> {
        let mut bytes = vec![0; usize::try_from(value.encoded_len()).unwrap()];
        value.encode(&mut bytes).unwrap();
        bytes
    }

    /// Returns the box of `box_type` and `payload`
    fn raw_box(box_type: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let header =
            BoxHeader::with_payload_len(BoxType::compact(box_type), to_u64(payload.len())).unwrap();
        let mut bytes = vec![0; usize::try_from(header.encoded_len()).unwrap()];
        header.encode(&mut bytes).unwrap();
        [bytes.as_slice(), payload].concat()
    }

    fn file_type() -> Vec<u8> {
        let mut ftyp = FileTypeBox::default();
        ftyp.major_brand = FourCc::new(*b"isom");
        encoded(&ftyp)
    }

    /// Returns a movie file of an `ftyp`, an `mdat` of six bytes, a `free`, and the `moov` of track 1 of two samples in that `mdat`
    fn movie_at_end() -> (Vec<u8>, Range<u64>) {
        let ftyp = file_type();
        let mdat = raw_box(*b"mdat", b"ABCDEF");
        let payload = to_u64(ftyp.len()).saturating_add(8)
            ..to_u64(ftyp.len()).saturating_add(to_u64(mdat.len()));
        let free = raw_box(*b"free", b"AAAA");
        let moov = encoded(&movie([track(1, 10, &[2, 4], &[(1, 2)], &[payload.start])]));
        ([ftyp, mdat, free, moov].concat(), payload)
    }

    #[test]
    fn a_movie_at_the_end_reads_without_a_byte_of_its_media_data() {
        let (file, payload) = movie_at_end();
        let start = payload.start;
        for piece_len in [1, 7, usize::MAX] {
            let read = read(&mut Demuxer::new(), &file, piece_len, false).unwrap();

            assert_eq!(
                read.seen,
                [
                    Seen::FileType,
                    Seen::MediaData(ByteRange::new(start.saturating_sub(8), Some(14))),
                    Seen::Other(
                        BoxType::compact(*b"free"),
                        ByteRange::new(payload.end, Some(12))
                    ),
                    Seen::Movie(vec![
                        (1, 0, start..start.saturating_add(2)),
                        (1, 10, start.saturating_add(2)..payload.end),
                    ]),
                ],
                "in pieces of {piece_len}"
            );
            assert!(
                read.wanted.iter().all(|offset| !payload.contains(offset)),
                "in pieces of {piece_len}, the media data is wanted: {:?}",
                read.wanted
            );
        }
    }

    #[test]
    fn a_media_data_box_streams_as_pieces_at_their_file_offsets() {
        let (file, payload) = movie_at_end();
        for piece_len in [1, 7, usize::MAX] {
            let read = read(&mut Demuxer::new(), &file, piece_len, true).unwrap();

            assert_eq!(
                read.seen.get(2),
                Some(&Seen::Payload(payload.start, b"ABCDEF".to_vec())),
                "in pieces of {piece_len}"
            );
        }
    }

    #[test]
    fn a_media_data_box_not_answered_refuses_input_and_leaves_the_demuxer_as_it_was() {
        let (file, _) = movie_at_end();
        let mut demuxer = Demuxer::new();
        let mut offset = 0;
        let events = loop {
            let input = file.get(usize::try_from(offset).unwrap()..).unwrap();
            let events: Vec<_> = demuxer.handle_input(offset, input).unwrap().collect();
            if !events.is_empty() && !matches!(events.last(), Some(DemuxEvent::FileType)) {
                break events;
            }
            offset = demuxer.wanted_input().unwrap().offset();
        };
        assert!(matches!(events.as_slice(), [DemuxEvent::MediaData { .. }]));
        let wanted = demuxer.wanted_input();

        let offset = wanted.unwrap().offset();
        assert_eq!(
            demuxer
                .handle_input(
                    offset,
                    file.get(usize::try_from(offset).unwrap()..).unwrap()
                )
                .map(Iterator::count),
            Err(DemuxError::UndecidedBox {
                box_type: BoxType::compact(*b"mdat")
            })
        );
        assert_eq!(demuxer.wanted_input(), wanted);
        assert_eq!(demuxer.skip(), Ok(()));
    }

    /// Returns a movie of track 1, no sample in its `moov`, and a `trex` of samples of 100 ticks and 2 bytes
    fn fragmented_movie() -> Vec<u8> {
        let mut movie = movie([track(1, 10, &[], &[], &[])]);
        let mut trex = TrackExtendsBox::default();
        trex.track_id = 1;
        trex.default_sample_description_index = 1;
        trex.default_sample_duration = 100;
        trex.default_sample_size = 2;
        let mut mvex = MovieExtendsBox::default();
        mvex.trex = vec![trex];
        movie.mvex = Some(mvex);
        encoded(&movie)
    }

    /// Returns a `moof` of two samples of track 1, decoded from `tfdt` if given, followed by the `mdat` of their four bytes
    fn fragment(sequence_number: u32, tfdt: Option<u64>) -> Vec<u8> {
        let mut trun = TrackRunBox::default();
        trun.data_offset = Some(0);
        trun.samples = TrackRunSamples::EmptyRows { sample_count: 2 };
        let mut traf = TrackFragmentBox::default();
        traf.tfhd.track_id = 1;
        traf.tfhd.default_base_is_moof = true;
        traf.tfdt = tfdt.map(|base_media_decode_time| {
            let mut tfdt = TrackFragmentBaseMediaDecodeTimeBox::default();
            tfdt.version = 1;
            tfdt.base_media_decode_time = base_media_decode_time;
            tfdt
        });
        traf.trun = vec![trun];
        let mut moof = MovieFragmentBox::default();
        moof.mfhd.sequence_number = sequence_number;
        moof.traf = vec![traf];
        let data_offset = to_u64(encoded(&moof).len()).saturating_add(8);
        if let Some(trun) = moof.traf.first_mut().and_then(|traf| traf.trun.first_mut()) {
            trun.data_offset = Some(i32::try_from(data_offset).unwrap());
        }
        [encoded(&moof), raw_box(*b"mdat", b"WXYZ")].concat()
    }

    /// Returns the samples of `fragment`, made by [`fragment`], at file offset `offset`, that start at decoding time `decoding_time`
    fn fragment_samples(fragment: &[u8], offset: u64, decoding_time: u64) -> Seen {
        let data = offset
            .saturating_add(to_u64(fragment.len()))
            .saturating_sub(4);
        Seen::MovieFragment(vec![
            (1, decoding_time, data..data.saturating_add(2)),
            (
                1,
                decoding_time.saturating_add(100),
                data.saturating_add(2)..data.saturating_add(4),
            ),
        ])
    }

    #[test]
    fn the_fragments_of_a_movie_carry_its_decoding_time_on() {
        let head = [file_type(), fragmented_movie()].concat();
        let file = [head.clone(), fragment(1, None), fragment(2, None)].concat();
        let second = to_u64(head.len()).saturating_add(to_u64(fragment(1, None).len()));
        let read = read(&mut Demuxer::new(), &file, 7, false).unwrap();

        let fragments: Vec<_> = read
            .seen
            .into_iter()
            .filter(|seen| matches!(seen, Seen::MovieFragment(_)))
            .collect();
        assert_eq!(
            fragments,
            [
                fragment_samples(&fragment(1, None), to_u64(head.len()), 0),
                fragment_samples(&fragment(2, None), second, 200)
            ]
        );
    }

    #[test]
    fn concatenated_segments_report_each_brand_and_index_and_carry_the_decoding_time_on() {
        let head = [file_type(), fragmented_movie()].concat();
        let segment = |sequence_number| {
            let mut styp = SegmentTypeBox::default();
            styp.major_brand = FourCc::new(*b"msdh");
            let mut sidx = SegmentIndexBox::default();
            sidx.reference_id = 1;
            sidx.timescale = 1000;
            let mut reference = SegmentIndexReference::default();
            reference.referenced_size = u32::try_from(fragment(0, None).len()).unwrap();
            reference.subsegment_duration = 200;
            sidx.references = vec![reference];
            [
                encoded(&styp),
                encoded(&sidx),
                fragment(sequence_number, None),
            ]
            .concat()
        };
        let first = [head.clone(), segment(0)].concat();
        let file = [first.clone(), segment(1)].concat();
        let mut demuxer = Demuxer::new();
        let read = read(&mut demuxer, &file, usize::MAX, false).unwrap();

        let fragment_offset = |segment_offset: u64| {
            segment_offset
                .saturating_add(to_u64(segment(0).len()))
                .saturating_sub(to_u64(fragment(0, None).len()))
        };
        let media_data = |segment_offset: u64| {
            let end = segment_offset.saturating_add(to_u64(segment(0).len()));
            Seen::MediaData(ByteRange::new(end.saturating_sub(12), Some(12)))
        };
        assert_eq!(
            read.seen,
            [
                Seen::FileType,
                Seen::Movie(vec![]),
                Seen::SegmentType,
                Seen::SegmentIndex,
                fragment_samples(&fragment(0, None), fragment_offset(to_u64(head.len())), 0),
                media_data(to_u64(head.len())),
                Seen::SegmentType,
                Seen::SegmentIndex,
                fragment_samples(
                    &fragment(1, None),
                    fragment_offset(to_u64(first.len())),
                    200
                ),
                media_data(to_u64(first.len())),
            ]
        );
        assert_eq!(demuxer.segment_indexes().len(), 2);
        assert_eq!(
            demuxer.segment_type().map(|styp| styp.major_brand),
            Some(FourCc::new(*b"msdh"))
        );
    }

    #[test]
    fn a_fragment_resumed_at_resolves_only_from_its_tfdt() {
        let head = [file_type(), fragmented_movie()].concat();
        let second = to_u64(head.len()).saturating_add(to_u64(fragment(1, None).len()));
        let file = [head.clone(), fragment(1, None), fragment(2, Some(5000))].concat();
        let mut demuxer = Demuxer::new();
        read(&mut demuxer, &file, usize::MAX, false).unwrap();

        demuxer.resume_at(second).unwrap();
        let resumed = read(&mut demuxer, &file, usize::MAX, false).unwrap();
        assert_eq!(
            resumed.seen.first(),
            Some(&fragment_samples(&fragment(2, Some(5000)), second, 5000))
        );

        demuxer.resume_at(to_u64(head.len())).unwrap();
        assert_eq!(
            read(&mut demuxer, &file, usize::MAX, false).map(|read| read.seen),
            Err(DemuxError::MissingDecodingTime { track_id: 1 })
        );
        assert_eq!(demuxer.wanted_input(), None);
        assert!(demuxer.movie().is_some());
        assert_eq!(
            demuxer.resume_at(second),
            Err(DemuxError::MissingDecodingTime { track_id: 1 })
        );
    }

    #[test]
    fn a_movie_that_runs_to_the_end_of_the_file_is_reported_by_finish() {
        let (file, _) = movie_at_end();
        let mut file = file;
        let moov_offset = file
            .len()
            .saturating_sub(encoded(&movie([track(1, 10, &[2, 4], &[(1, 2)], &[0])])).len());
        file.get_mut(moov_offset..moov_offset.saturating_add(4))
            .unwrap()
            .copy_from_slice(&[0; 4]);

        let (_, payload) = movie_at_end();
        let read = read(&mut Demuxer::new(), &file, 7, false).unwrap();
        assert_eq!(
            read.seen.last(),
            Some(&Seen::Movie(vec![
                (1, 0, payload.start..payload.start.saturating_add(2)),
                (1, 10, payload.start.saturating_add(2)..payload.end),
            ]))
        );
    }

    #[test]
    fn a_box_whose_payload_is_past_the_limit_fails_the_demuxer() {
        let (file, _) = movie_at_end();
        let mut demuxer = Demuxer::with_limits(DemuxLimits::new().with_payload_bytes(16));

        let failure = read(&mut demuxer, &file, usize::MAX, false).map(|read| read.seen);
        assert!(
            matches!(failure, Err(DemuxError::PayloadLimitExceeded { box_type, limit_bytes: 16, .. }) if box_type == isobmff_boxes::MovieBox::BOX_TYPE),
            "{failure:?}"
        );
        assert_eq!(demuxer.wanted_input(), None);
        assert_eq!(demuxer.finish().map(Iterator::count), failure.map(|_| 0));
    }

    #[test]
    fn a_failure_is_returned_by_the_call_that_reads_it_even_with_no_event_before_it() {
        let file = [
            file_type(),
            raw_box(*b"free", b"AAAA"),
            b"\0\0\0\x02free".to_vec(),
        ]
        .concat();
        let mut demuxer = Demuxer::new();
        let mut outcomes = Vec::new();
        while let Some(wanted) = demuxer.wanted_input() {
            let input = file
                .get(usize::try_from(wanted.offset()).unwrap()..)
                .unwrap();
            let outcome = demuxer
                .handle_input(wanted.offset(), input)
                .map(Iterator::count);
            outcomes.push(outcome);
        }

        assert_eq!(
            outcomes.last(),
            Some(&Err(DemuxError::Box {
                error: isobmff_core::Error::size_below_header(8, 2)
            }))
        );
    }

    #[test]
    fn a_box_of_an_extended_type_is_reported_as_any_other() {
        let mut uuid = b"\0\0\0\x1cuuid".to_vec();
        uuid.extend_from_slice(&[0xab; 16]);
        uuid.extend_from_slice(b"AAAA");
        let file = [file_type(), uuid].concat();

        let read = read(&mut Demuxer::new(), &file, usize::MAX, false).unwrap();
        assert_eq!(
            read.seen,
            [
                Seen::FileType,
                Seen::Other(BoxType::extended([0xab; 16]), ByteRange::new(16, Some(28))),
            ]
        );
    }

    #[test]
    fn a_movie_read_again_after_resuming_before_it_fails_as_a_duplicate() {
        let (file, _) = movie_at_end();
        let mut demuxer = Demuxer::new();
        read(&mut demuxer, &file, usize::MAX, false).unwrap();

        demuxer.resume_at(0).unwrap();
        assert_eq!(
            read(&mut demuxer, &file, usize::MAX, false).map(|read| read.seen),
            Err(DemuxError::DuplicateBox {
                box_type: isobmff_boxes::MovieBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn a_fragment_before_its_movie_fails_as_missing_the_movie() {
        let file = [file_type(), fragment(1, None), fragmented_movie()].concat();

        assert_eq!(
            read(&mut Demuxer::new(), &file, usize::MAX, false).map(|read| read.seen),
            Err(DemuxError::MissingMovie)
        );
    }
}
