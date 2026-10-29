//! [`Demuxer`], which reads a movie file or a segment from the bytes a caller reads

mod file_level;

use alloc::boxed::Box;

use isobmff_boxes::{
    FileTypeBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox, SegmentTypeBox,
};
use isobmff_core::BoxHeader;

use crate::byte_range::ByteRange;
use crate::demuxer::file_level::{Handling, Value};
use crate::error::DemuxError;
use crate::framing::{BoxEvent, BoxReader};
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
/// [`SampleLocation`](crate::SampleLocation)s it declares. Every other box,
/// the media data boxes among them, is skipped and reported by where it lies
/// (ISO/IEC 14496-12 §4.2.2): the bytes of the samples are the caller's to
/// read at their locations, or to gather with a
/// [`MediaDataReader`](crate::media_data::MediaDataReader).
///
/// The demuxer holds no rule on the order of the boxes: a `moof` before the
/// `moov` fails as the structure-data it declares does not resolve.
///
/// A failure is returned by the call that reads it, and leaves the demuxer
/// failed: it wants no more input, and every call after it returns the same
/// failure. Input that starts past the offset the demuxer wants is refused
/// with [`UnwantedInput`](DemuxError::UnwantedInput), and leaves the demuxer
/// as it was.
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
    /// `input` is taken, and the event returned, as [`BoxReader::handle_input`]
    /// takes and returns them: one event a call, the caller passing the same
    /// input again while events come. A box gathered into a value is read past
    /// within the call, and reported once it is read.
    ///
    /// A failure leaves the demuxer failed: it wants no more input, and every
    /// call after it returns the same failure.
    ///
    /// # Errors
    ///
    /// * [`UnwantedInput`](DemuxError::UnwantedInput): `input` starts past the
    ///   wanted offset. The demuxer is left as it was.
    /// * The failures of [`BoxReader::handle_input`], and of
    ///   [`BoxReader::gather`] with the limit of the payload of a box.
    /// * [`Box`](DemuxError::Box): a box gathered does not read as its value.
    /// * The failures of the [`StructureDataReader`] the boxes of
    ///   structure-data are handed to.
    /// * The failure the demuxer failed with before.
    pub fn handle_input(
        &mut self,
        offset: u64,
        input: &[u8],
    ) -> Result<Option<DemuxEvent>, DemuxError> {
        loop {
            let Some(box_event) = self.boxes.handle_input(offset, input)? else {
                return Ok(None);
            };
            if let Some(event) = self.demux(box_event)? {
                return Ok(Some(event));
            }
        }
    }

    /// Ends the file at the offset the demuxer wants, and returns the event of the box it ends
    ///
    /// A box whose size runs to the end of the file ends here, so the event
    /// goes where those of [`handle_input`](Self::handle_input) go: a `moov`
    /// that runs to the end of the file is reported here. The demuxer then
    /// wants no more input.
    ///
    /// # Errors
    ///
    /// * The failures of [`BoxReader::finish`], and those of reading a box
    ///   into its value as for [`handle_input`](Self::handle_input).
    /// * The failure the demuxer failed with before.
    pub fn finish(&mut self) -> Result<Option<DemuxEvent>, DemuxError> {
        let Some(box_event) = self.boxes.finish()? else {
            return Ok(None);
        };
        self.demux(box_event)
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

    /// Returns the `styp` read last, §8.14.2
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

    /// Makes an event of the reader of the boxes into the event of the demuxer, if it makes one
    fn demux(&mut self, box_event: BoxEvent<'_>) -> Result<Option<DemuxEvent>, DemuxError> {
        let event = match box_event {
            BoxEvent::Header(header) => self.open(header),
            // Why not `unreachable!`: a payload comes only after `stream`, which
            // the demuxer never calls, and library code does not panic.
            BoxEvent::Payload(_) => Ok(None),
            BoxEvent::End => self.close(),
        };
        event.inspect_err(|&failure| self.boxes.fail_with(failure))
    }

    /// Answers the header of a box at file level as its type says, and returns the event it is reported with
    fn open(&mut self, header: BoxHeader) -> Result<Option<DemuxEvent>, DemuxError> {
        let byte_range = self.boxes.open_box().ok_or(DemuxError::NoOpenBox)?;
        let event = match file_level::handling(header.box_type()) {
            Handling::Gather => {
                self.boxes.gather(self.payload_limit_bytes)?;
                return Ok(None);
            }
            Handling::MediaData => DemuxEvent::MediaData { header, byte_range },
            Handling::Other => DemuxEvent::Other { header, byte_range },
        };
        self.boxes.skip()?;
        Ok(Some(event))
    }

    /// Reads the box just gathered, if any, into its value, and returns the event it is reported with
    fn close(&mut self) -> Result<Option<DemuxEvent>, DemuxError> {
        let (Some(raw_box), Some(range)) =
            (self.boxes.gathered_box(), self.boxes.gathered_box_range())
        else {
            return Ok(None);
        };
        let byte_range = ByteRange::new(range.start, Some(range.end.saturating_sub(range.start)));
        let Some(value) = file_level::read(raw_box)? else {
            return Ok(None);
        };
        let event = match value {
            Value::FileType(ftyp) => {
                self.file_type = Some(ftyp);
                DemuxEvent::FileType { byte_range }
            }
            Value::SegmentType(styp) => {
                self.segment_type = Some(styp);
                DemuxEvent::SegmentType { byte_range }
            }
            Value::Movie(moov) => DemuxEvent::Movie {
                byte_range,
                samples: self.structure_data.handle_movie(moov)?,
            },
            Value::MovieFragment(moof) => DemuxEvent::MovieFragment {
                samples: self
                    .structure_data
                    .handle_movie_fragment(&moof, range.start)?,
                movie_fragment: Box::new(moof),
                byte_range,
            },
            Value::SegmentIndex(sidx) => {
                self.structure_data.handle_segment_index(&sidx, range)?;
                DemuxEvent::SegmentIndex { byte_range }
            }
            Value::MovieFragmentRandomAccess(mfra) => {
                self.structure_data
                    .handle_movie_fragment_random_access(mfra);
                DemuxEvent::MovieFragmentRandomAccess { byte_range }
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
/// An event says what was read and where it lies. The values the demuxer
/// keeps are its to return; a `moof`, which it does not keep, comes with the
/// event. An event borrows neither the input nor the demuxer.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum DemuxEvent {
    /// `ftyp` read, §5.2: [`Demuxer::file_type`] returns it
    #[non_exhaustive]
    FileType {
        /// Where the whole box lies
        byte_range: ByteRange,
    },
    /// `styp` read, §8.14.2: [`Demuxer::segment_type`] returns it
    #[non_exhaustive]
    SegmentType {
        /// Where the whole box lies
        byte_range: ByteRange,
    },
    /// `moov` read, §8.2.1: [`Demuxer::movie`] returns it
    #[non_exhaustive]
    Movie {
        /// Where the whole box lies
        byte_range: ByteRange,
        /// Locations of the samples its sample tables declare, in the order their media data lies
        samples: SampleLocations,
    },
    /// `moof` read, §8.8.4
    #[non_exhaustive]
    MovieFragment {
        /// The `moof`
        movie_fragment: Box<MovieFragmentBox>,
        /// Where the whole box lies
        byte_range: ByteRange,
        /// Locations of the samples its track fragment runs declare, in the order their media data lies
        samples: SampleLocations,
    },
    /// `sidx` read, §8.14.3: its index is the one of [`Demuxer::segment_indexes`] that starts where this does
    #[non_exhaustive]
    SegmentIndex {
        /// Where the whole box lies
        byte_range: ByteRange,
    },
    /// `mfra` read, §8.8.9: [`Demuxer::movie_fragment_random_access`] returns it
    #[non_exhaustive]
    MovieFragmentRandomAccess {
        /// Where the whole box lies
        byte_range: ByteRange,
    },
    /// Media data box, `mdat` (§8.1.1) or `imda` (§8.1.4), skipped
    #[non_exhaustive]
    MediaData {
        /// Header of the box
        header: BoxHeader,
        /// Where the whole box lies, its length `None` when it runs to the end of the file
        byte_range: ByteRange,
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
        Other(BoxType, ByteRange),
    }

    /// What a caller saw reading a file
    #[derive(Debug)]
    struct Read {
        seen: Vec<Seen>,
        /// File offsets the demuxer wanted input from
        wanted: Vec<u64>,
    }

    /// Reads `file` from wherever `demuxer` wants it, `piece_len` bytes at a time
    fn read(demuxer: &mut Demuxer, file: &[u8], piece_len: usize) -> Result<Read, DemuxError> {
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
            while let Some(event) = demuxer.handle_input(wanted.offset(), input)? {
                see(&mut read.seen, event);
            }
        }
        if let Some(event) = demuxer.finish()? {
            see(&mut read.seen, event);
        }

        Ok(read)
    }

    fn see(seen: &mut Vec<Seen>, event: DemuxEvent) {
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
            DemuxEvent::FileType { .. } => Seen::FileType,
            DemuxEvent::SegmentType { .. } => Seen::SegmentType,
            DemuxEvent::Movie { samples, .. } => Seen::Movie(placed(samples)),
            DemuxEvent::MovieFragment { samples, .. } => Seen::MovieFragment(placed(samples)),
            DemuxEvent::SegmentIndex { .. } => Seen::SegmentIndex,
            DemuxEvent::MovieFragmentRandomAccess { .. } => Seen::MovieFragmentRandomAccess,
            DemuxEvent::MediaData { byte_range, .. } => Seen::MediaData(byte_range),
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
            let read = read(&mut Demuxer::new(), &file, piece_len).unwrap();

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
        let read = read(&mut Demuxer::new(), &file, 7).unwrap();

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
        let read = read(&mut demuxer, &file, usize::MAX).unwrap();

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
    fn the_boxes_read_into_their_values_are_reported_where_they_lie() {
        let (ftyp_length, moov_length) =
            (to_u64(file_type().len()), to_u64(fragmented_movie().len()));
        let moof_offset = ftyp_length.saturating_add(moov_length);
        let mdat_length = to_u64(raw_box(*b"mdat", b"WXYZ").len());
        let moof_length = to_u64(fragment(7, None).len()).saturating_sub(mdat_length);
        let file = [file_type(), fragmented_movie(), fragment(7, None)].concat();
        let mut demuxer = Demuxer::new();
        let mut reported = Vec::new();
        while let Some(event) = demuxer.handle_input(0, &file).unwrap() {
            match event {
                DemuxEvent::FileType { byte_range, .. } | DemuxEvent::Movie { byte_range, .. } => {
                    reported.push((byte_range, None));
                }
                DemuxEvent::MovieFragment {
                    movie_fragment,
                    byte_range,
                    ..
                } => reported.push((byte_range, Some(movie_fragment.mfhd.sequence_number))),
                DemuxEvent::SegmentType { .. }
                | DemuxEvent::SegmentIndex { .. }
                | DemuxEvent::MovieFragmentRandomAccess { .. }
                | DemuxEvent::MediaData { .. }
                | DemuxEvent::Other { .. } => {}
            }
        }

        assert_eq!(
            reported,
            [
                (ByteRange::new(0, Some(ftyp_length)), None),
                (ByteRange::new(ftyp_length, Some(moov_length)), None),
                (ByteRange::new(moof_offset, Some(moof_length)), Some(7)),
            ]
        );
    }

    #[test]
    fn a_segment_index_read_again_is_reported_where_its_sidx_lies_and_kept_once() {
        let sidx = |earliest_presentation_time| {
            let mut sidx = SegmentIndexBox::default();
            sidx.reference_id = 1;
            sidx.timescale = 1000;
            sidx.earliest_presentation_time = earliest_presentation_time;
            encoded(&sidx)
        };
        let head = [file_type(), fragmented_movie()].concat();
        let first = to_u64(head.len());
        let file = [head, sidx(0), sidx(200)].concat();
        let mut demuxer = Demuxer::new();
        read(&mut demuxer, &file, usize::MAX).unwrap();

        demuxer.resume_at(first).unwrap();
        let reported = loop {
            if let DemuxEvent::SegmentIndex { byte_range, .. } =
                demuxer.handle_input(0, &file).unwrap().unwrap()
            {
                break byte_range;
            }
        };
        assert_eq!(reported, ByteRange::new(first, Some(to_u64(sidx(0).len()))));
        let starts: Vec<_> = demuxer
            .segment_indexes()
            .iter()
            .map(|index| index.byte_range().start)
            .collect();
        assert_eq!(starts, [first, first.saturating_add(to_u64(sidx(0).len()))]);
    }

    #[test]
    fn a_fragment_resumed_at_resolves_only_from_its_tfdt() {
        let head = [file_type(), fragmented_movie()].concat();
        let second = to_u64(head.len()).saturating_add(to_u64(fragment(1, None).len()));
        let file = [head.clone(), fragment(1, None), fragment(2, Some(5000))].concat();
        let mut demuxer = Demuxer::new();
        read(&mut demuxer, &file, usize::MAX).unwrap();

        demuxer.resume_at(second).unwrap();
        let resumed = read(&mut demuxer, &file, usize::MAX).unwrap();
        assert_eq!(
            resumed.seen.first(),
            Some(&fragment_samples(&fragment(2, Some(5000)), second, 5000))
        );

        demuxer.resume_at(to_u64(head.len())).unwrap();
        assert_eq!(
            read(&mut demuxer, &file, usize::MAX).map(|read| read.seen),
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
        let read = read(&mut Demuxer::new(), &file, 7).unwrap();
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

        let failure = read(&mut demuxer, &file, usize::MAX).map(|read| read.seen);
        assert!(
            matches!(failure, Err(DemuxError::PayloadLimitExceeded { box_type, limit_bytes: 16, .. }) if box_type == isobmff_boxes::MovieBox::BOX_TYPE),
            "{failure:?}"
        );
        assert_eq!(demuxer.wanted_input(), None);
        assert_eq!(demuxer.finish().err(), failure.err());
    }

    #[test]
    fn a_header_that_fails_after_a_skipped_box_fails_the_call_that_reads_it() {
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
                .map(|event| event.is_some());
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
    fn the_events_before_a_failure_arrive_however_the_input_is_cut() {
        let file = [
            file_type(),
            raw_box(*b"mdat", b"AB"),
            b"\0\0\0\x02free".to_vec(),
        ]
        .concat();
        let failure = DemuxError::Box {
            error: isobmff_core::Error::size_below_header(8, 2),
        };
        let mdat = ByteRange::new(to_u64(file_type().len()), Some(10));
        for piece_len in [1, 2, 8, usize::MAX] {
            let mut demuxer = Demuxer::new();
            let mut seen = Vec::new();
            let ending = loop {
                let Some(wanted) = demuxer.wanted_input() else {
                    break Ok(());
                };
                let rest = file
                    .get(usize::try_from(wanted.offset()).unwrap()..)
                    .unwrap();
                match demuxer.handle_input(wanted.offset(), rest.get(..piece_len).unwrap_or(rest)) {
                    Ok(Some(event)) => see(&mut seen, event),
                    Ok(None) => {}
                    Err(failure) => break Err(failure),
                }
            };

            assert_eq!(
                (seen, ending),
                (vec![Seen::FileType, Seen::MediaData(mdat)], Err(failure)),
                "in pieces of {piece_len}"
            );
        }
    }

    #[test]
    fn a_box_of_an_extended_type_is_reported_as_any_other() {
        let mut uuid = b"\0\0\0\x1cuuid".to_vec();
        uuid.extend_from_slice(&[0xab; 16]);
        uuid.extend_from_slice(b"AAAA");
        let file = [file_type(), uuid].concat();

        let read = read(&mut Demuxer::new(), &file, usize::MAX).unwrap();
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
        read(&mut demuxer, &file, usize::MAX).unwrap();

        demuxer.resume_at(0).unwrap();
        assert_eq!(
            read(&mut demuxer, &file, usize::MAX).map(|read| read.seen),
            Err(DemuxError::DuplicateBox {
                box_type: isobmff_boxes::MovieBox::BOX_TYPE
            })
        );
    }

    #[test]
    fn a_fragment_before_its_movie_fails_as_missing_the_movie() {
        let file = [file_type(), fragment(1, None), fragmented_movie()].concat();

        assert_eq!(
            read(&mut Demuxer::new(), &file, usize::MAX).map(|read| read.seen),
            Err(DemuxError::MissingMovie)
        );
    }
}
