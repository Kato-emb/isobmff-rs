//! [`IndexedSegmentMuxer`], which writes a movie in segments, each indexed by a segment index

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::num::NonZeroU16;
use core::slice;

use isobmff_boxes::{FileTypeBox, MovieBox, SegmentTypeBox};

use super::output::{append_encoded, to_bytes};
use super::segment_index::{Subsegment, free_space, segment_index, segment_index_len};
use super::{Error, FragmentedMovieFileMuxer, MovieParameters, MuxerConfig, TrackParameters};
use crate::SampleProperties;

/// Which track an [`IndexedSegmentMuxer`] indexes, and how many subsegments each of its media segments holds at most
///
/// The reference stream of a segment index, ISO/IEC 14496-12 §8.14.3.1, is
/// the caller's to choose: the specification says only that it is normally
/// a track not every sample of which is coded independently, such as video,
/// and the muxer does not choose one.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SegmentIndexParameters {
    /// `track_ID` of the reference track, whose presentation times the
    /// segment index gives
    pub reference_track_id: u32,
    /// The most subsegments a media segment holds, for which the muxer
    /// reserves the room of the segment index before the first
    pub subsegment_count: NonZeroU16,
}

impl SegmentIndexParameters {
    /// Creates the parameters of a segment index of the track `reference_track_id`, of at most `subsegment_count` subsegments a media segment
    #[must_use]
    pub const fn new(reference_track_id: u32, subsegment_count: NonZeroU16) -> Self {
        Self {
            reference_track_id,
            subsegment_count,
        }
    }
}

/// The `sidx` a caller writes over the `free` first written for it, once an [`IndexedSegmentMuxer`] has ended a media segment
///
/// The [`SegmentIndexBox`](isobmff_boxes::SegmentIndexBox) (`sidx`),
/// ISO/IEC 14496-12 §8.14.3, of the segment, and a `free` after it where the
/// segment holds fewer subsegments than room was reserved for: the bytes are
/// as long as the `free` first written.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SegmentIndex {
    /// Offset of the `free` the bytes are written over, in the run of bytes
    /// the muxer returns from its first, the first byte of the `ftyp`
    pub offset: u64,
    /// The bytes of the `sidx`
    pub bytes: Vec<u8>,
}

/// The track an [`IndexedSegmentMuxer`] indexes, before and once it is added
#[derive(Debug, Clone)]
enum Reference {
    /// The track is yet to be added; the parameters of the movie build the
    /// `moov` of the track once it is
    Pending(Box<MovieParameters>),
    // Why hold a `moov` of the track: the fragmented muxer gives up the
    // parameters of its tracks once it writes the `moov`, and the
    // presentation of each subsegment needs the edit list and the timescales
    // of the track.
    Added(Box<MovieBox>),
}

/// The media segment being written, which holds no subsegment until its first is written
#[derive(Debug, Clone, Default)]
struct Segment {
    // Why hold where the `free` lies rather than find it again: the
    // fragments after it are held or polled once it is written.
    /// Offset of the `free` reserved for its `sidx`
    offset: u64,
    subsegments: Vec<Subsegment>,
}

/// Writer of a movie in media segments each indexed by a `sidx`, after an initialization segment
///
/// The initialization segment is the `ftyp` and a `moov` that declares the
/// tracks and no sample, and each media segment an `styp`, a `sidx` and the
/// subsegments it indexes: the Indexed Media Segment of ISO/IEC 23009-1
/// (MPEG-DASH), of the segments of ISO/IEC 14496-12 §8.14. Each media segment
/// opens with a [`SegmentTypeBox`] (`styp`), §8.14.2, and a
/// [`SegmentIndexBox`](isobmff_boxes::SegmentIndexBox) (`sidx`), §8.14.3,
/// of version 1, that indexes the track
/// [`SegmentIndexParameters::reference_track_id`] names, and holds up to
/// [`SegmentIndexParameters::subsegment_count`] subsegments, each one movie
/// fragment as [`FragmentedMovieFileMuxer`] writes it. The times of the
/// `sidx` are the presentation times of the reference track, its edit list
/// applied, §8.14.3.1, in the timescale of its media.
///
/// The muxer is driven as a [`SegmentMuxer`](super::SegmentMuxer) is, with
/// [`finish_subsegment`](Self::finish_subsegment) to end a subsegment within
/// the media segment. The bytes follow one another in one run: since a
/// `sidx` comes before the subsegments it indexes, the muxer writes a `free`
/// of its length before the first, and returns the `sidx` from
/// [`finish_segment`](Self::finish_segment) and [`finish`](Self::finish) for
/// the caller to write over it, at an offset in the run. A caller that keeps
/// each segment apart takes from it the offset its segment starts at.
///
/// ```
/// use std::num::NonZeroU16;
///
/// use isobmff::demux::Event;
/// use isobmff::mux::{MovieParameters, SegmentIndexParameters, TrackParameters};
/// use isobmff::structure_data::subsegments;
/// use isobmff::{Demuxer, DemuxerConfig, IndexedSegmentMuxer, MuxerConfig, SampleProperties};
/// use isobmff_boxes::{FileTypeBox, SampleFlags, SegmentTypeBox};
/// use isobmff_core::{BoxDecode, BoxHeader, BoxType, FourCc};
///
/// // A movie of one track, whose sample entry is that of a file, presented as it is composed
/// let file: &[u8] = include_bytes!("../../tests/fixtures/segment.mp4");
/// let (file_type, rest) = FileTypeBox::decode(file)?;
/// let (moov, _) = isobmff_boxes::MovieBox::decode(rest)?;
/// let track_id = moov.trak[0].tkhd.track_id;
/// let mut segment_type = SegmentTypeBox::default();
/// segment_type.major_brand = FourCc::new(*b"msix");
/// let index = SegmentIndexParameters::new(track_id, NonZeroU16::new(4).ok_or("no room")?);
/// let mut muxer = IndexedSegmentMuxer::new(MovieParameters::from(&moov), MuxerConfig::default(), file_type, segment_type, index)?;
/// let mut track = TrackParameters::from(&moov.trak[0]);
/// track.edts = None;
/// muxer.add_track(track)?;
///
/// // One media segment of two subsegments of one sample each
/// let sample = |decoding_time| SampleProperties {
///     track_id,
///     decoding_time,
///     sample_duration: 1024,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// let mut written = Vec::new();
/// muxer.handle_sample(sample(0), b"abc")?;
/// muxer.finish_subsegment()?;
/// muxer.handle_sample(sample(1024), b"de")?;
/// let sidx = muxer.finish()?.ok_or("no segment")?;
/// muxer.poll_output(&mut written);
///
/// // Write the `sidx` over the `free` reserved for it
/// let start = usize::try_from(sidx.offset)?;
/// written.get_mut(start..start + sidx.bytes.len()).ok_or("no free")?.copy_from_slice(&sidx.bytes);
///
/// // The `sidx` reads back, each subsegment at a `moof` of the bytes written
/// let mut demuxer = Demuxer::new(DemuxerConfig::default());
/// let (mut subsegment_starts, mut fragment_starts) = (Vec::new(), Vec::new());
/// loop {
///     while let Some(event) = demuxer.poll_event() {
///         match event {
///             Event::SegmentIndex { anchor, sidx } => subsegment_starts.extend(
///                 subsegments(&sidx, anchor)?.map(|subsegment| (subsegment.offset, subsegment.earliest_presentation_time)),
///             ),
///             _ => {}
///         }
///     }
///     let Some(required) = demuxer.required_input() else { break };
///     match written.get(usize::try_from(required.position)?..) {
///         Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
///         _ => demuxer.finish(),
///     }
/// }
/// let mut position = 0;
/// while let Some((header, _)) = written.get(position..).and_then(|rest| BoxHeader::decode(rest).ok()) {
///     if header.box_type() == BoxType::compact(*b"moof") {
///         fragment_starts.push(u64::try_from(position)?);
///     }
///     position += usize::try_from(header.size().total_bytes().ok_or("to the end")?)?;
/// }
/// assert_eq!(subsegment_starts, [(fragment_starts[0], 0), (fragment_starts[1], 1024)]);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct IndexedSegmentMuxer {
    muxer: FragmentedMovieFileMuxer,
    /// The bytes each media segment opens with: the `styp`, and the `free`
    /// reserved for the `sidx`, the last of them
    opening: Vec<u8>,
    index: SegmentIndexParameters,
    reference: Reference,
    // Why count what was polled: the offset of a `sidx` is that of the
    // `free` reserved for it in the whole run of bytes, which the bytes the
    // muxer holds do not give once some are polled.
    polled: u64,
    segment: Segment,
    // Why hold the samples of the reference track: the presentation of the
    // subsegment, and its stream access point, are of them, and the
    // fragmented muxer does not lend them.
    samples: Vec<SampleProperties>,
    // Why hold where the segment before ends rather than its subsegments:
    // the next starts there, and nothing else of them is needed once its
    // `sidx` is returned.
    previous_end: Option<u64>,
}

impl IndexedSegmentMuxer {
    /// Creates a muxer of the movie `movie` describes, whose initialization segment opens with `file_type`, each media segment with `segment_type` and a segment index as `index` sets it, and holds the `ftyp`
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::new`], and [`Error::Encode`] where the
    /// `styp` does not write.
    pub fn new(
        movie: MovieParameters,
        config: MuxerConfig,
        file_type: FileTypeBox,
        segment_type: SegmentTypeBox,
        index: SegmentIndexParameters,
    ) -> Result<Self, Error> {
        let mut opening = to_bytes(&segment_type)?;
        let reserved = segment_index_len(index.subsegment_count.get());
        append_encoded(&free_space(reserved), &mut opening)?;
        Ok(Self {
            reference: Reference::Pending(Box::new(movie.clone())),
            muxer: FragmentedMovieFileMuxer::new(movie, config, file_type)?,
            opening,
            index,
            polled: 0,
            segment: Segment::default(),
            samples: Vec::new(),
            previous_end: None,
        })
    }

    /// Adds the track `track` describes, before the first sample
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::add_track`].
    pub fn add_track(&mut self, track: TrackParameters) -> Result<(), Error> {
        let reference = match &self.reference {
            Reference::Pending(movie) if track.track_id == self.index.reference_track_id => Some(
                Box::new(movie.to_fragmented_movie_box(slice::from_ref(&track))),
            ),
            Reference::Pending(_) | Reference::Added(_) => None,
        };
        self.muxer.add_track(track)?;
        if let Some(movie) = reference {
            self.reference = Reference::Added(movie);
        }
        Ok(())
    }

    /// Takes the next sample of the subsegment, with its bytes, and holds them until the subsegment ends; the first sample writes the initialization segment first
    ///
    /// # Errors
    /// * [`Error::UnknownTrack`]: no track of the sample's `track_ID`, or of
    ///   [`SegmentIndexParameters::reference_track_id`], was added.
    /// * [`Error::SubsegmentCount`]: the sample would open a subsegment of a
    ///   media segment that holds as many as
    ///   [`SegmentIndexParameters::subsegment_count`]; the caller ends the
    ///   segment, and hands the sample over again, the first of the next.
    /// * As [`FragmentedMovieFileMuxer::handle_sample`].
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        bytes: &[u8],
    ) -> Result<(), Error> {
        if let Reference::Pending(_) = self.reference {
            return Err(Error::UnknownTrack {
                track_id: self.index.reference_track_id,
            });
        }
        // Why refuse as the segment fills rather than when the subsegment
        // ends: a segment that holds as many subsegments as its `sidx` counts
        // takes no sample, so none is held once it is full, and the caller
        // can end it.
        let subsegment_count = self.index.subsegment_count.get();
        if self.segment.subsegments.len() >= usize::from(subsegment_count) {
            return Err(Error::SubsegmentCount { subsegment_count });
        }
        self.muxer.handle_sample(properties, bytes)?;
        if properties.track_id == self.index.reference_track_id {
            self.samples.push(properties);
        }
        Ok(())
    }

    /// Ends the subsegment, and holds its `moof` and its `mdat` for [`poll_output`](Self::poll_output), after the `styp` and the `free` reserved for the `sidx` where it is the first of its media segment
    ///
    /// Where the subsegment holds no sample, nothing is written.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::NoReferenceSample`]: the subsegment presents no sample of
    ///   the reference track.
    /// * [`Error::ReferencedSize`]: the `moof` and the `mdat` take more bytes
    ///   than a `referenced_size` counts.
    /// * [`Error::SubsegmentTime`]: the subsegment starts no later than the
    ///   one before it in the media segment, or, as the first of a media
    ///   segment, not where the segment before it ends.
    /// * [`Error::SubsegmentDuration`]: the subsegment before it in the media
    ///   segment lasts longer than a `subsegment_duration` counts.
    /// * [`Error::Timeline`]: the edit list of the reference track does not
    ///   fit its samples.
    /// * [`Error::Encode`]: the `styp` or the `moof` does not write.
    pub fn finish_subsegment(&mut self) -> Result<(), Error> {
        match self.next_subsegment()? {
            Some(next) => self.write_subsegment(next),
            None => self.muxer.write_fragment(&[]),
        }
    }

    /// Ends the media segment, its last subsegment as [`finish_subsegment`](Self::finish_subsegment) does, and returns the `sidx` that indexes it, or `None` where the segment holds no subsegment
    ///
    /// The `sidx` gives the last subsegment the duration up to where the
    /// presentation of the reference track ends in it: the first subsegment
    /// of the next media segment is refused, with [`Error::SubsegmentTime`],
    /// where it does not start there.
    ///
    /// # Errors
    /// As [`finish_subsegment`](Self::finish_subsegment), and
    /// [`Error::SubsegmentDuration`] where the last subsegment lasts longer
    /// than a `subsegment_duration` counts.
    pub fn finish_segment(&mut self) -> Result<Option<SegmentIndex>, Error> {
        let next = self.next_subsegment()?;
        if self.segment.subsegments.is_empty() && next.is_none() {
            self.muxer.write_fragment(&[])?;
            return Ok(None);
        }
        let track_id = self.index.reference_track_id;
        let Reference::Added(movie) = &self.reference else {
            return Err(Error::UnknownTrack { track_id });
        };
        // Why build the `sidx` before the last subsegment is written: a
        // `sidx` that does not write leaves the muxer as it was.
        let bytes = segment_index(
            movie,
            track_id,
            self.index.subsegment_count.get(),
            self.segment.subsegments.iter().chain(next.as_ref()),
        )?;
        if let Some(next) = next {
            self.write_subsegment(next)?;
        }
        let segment = core::mem::take(&mut self.segment);
        self.previous_end = segment.subsegments.last().map(|last| last.end);
        Ok(Some(SegmentIndex {
            offset: segment.offset,
            bytes,
        }))
    }

    /// Appends the bytes the muxer holds to `out`, and holds none
    pub fn poll_output(&mut self, out: &mut Vec<u8>) {
        let start = out.len();
        self.muxer.poll_output(out);
        let appended = u64::try_from(out.len().saturating_sub(start)).unwrap_or(u64::MAX);
        self.polled = self.polled.saturating_add(appended);
    }

    /// Ends the last media segment, as [`finish_segment`](Self::finish_segment) does, or writes the initialization segment where no sample was taken, and returns the `sidx` of the segment
    ///
    /// The muxer then takes nothing more: it ends the file, unlike the
    /// `finish` of a writer of structure-data, which ends one unit and goes on.
    ///
    /// # Errors
    /// As [`finish_segment`](Self::finish_segment), and
    /// [`Error::Encode`] where the `moov` does not write.
    pub fn finish(&mut self) -> Result<Option<SegmentIndex>, Error> {
        let index = self.finish_segment()?;
        self.muxer.finish_after(&[])?;
        Ok(index)
    }

    /// Returns the subsegment the samples taken make, or `None` where there are none, checked against the subsegments before it
    fn next_subsegment(&self) -> Result<Option<Subsegment>, Error> {
        let Some(length) = self.muxer.fragment_len() else {
            return Ok(None);
        };
        let track_id = self.index.reference_track_id;
        let Reference::Added(movie) = &self.reference else {
            return Err(Error::UnknownTrack { track_id });
        };
        let next = Subsegment::new(movie, track_id, &self.samples, length)?;
        let found = next.earliest_presentation_time;
        match (self.segment.subsegments.last(), self.previous_end) {
            (Some(last), _) if found <= last.earliest_presentation_time => {
                Err(Error::SubsegmentTime {
                    expected: last.earliest_presentation_time,
                    found,
                })
            }
            (Some(last), _) => {
                let duration = found.saturating_sub(last.earliest_presentation_time);
                match u32::try_from(duration) {
                    Ok(_) => Ok(Some(next)),
                    Err(_) => Err(Error::SubsegmentDuration { duration }),
                }
            }
            (None, Some(end)) if end != found => Err(Error::SubsegmentTime {
                expected: end,
                found,
            }),
            (None, _) => Ok(Some(next)),
        }
    }

    /// Writes the subsegment `next` the samples taken make, after the `styp` and the `free` reserved for the `sidx` where it opens its media segment
    fn write_subsegment(&mut self, next: Subsegment) -> Result<(), Error> {
        let opens = self.segment.subsegments.is_empty();
        let before: &[u8] = if opens { &self.opening } else { &[] };
        let position = self
            .polled
            .saturating_add(u64::try_from(self.muxer.held_len()).unwrap_or(u64::MAX));
        self.muxer.write_fragment(before)?;
        if opens {
            let reserved = segment_index_len(self.index.subsegment_count.get());
            let opening = u64::try_from(self.opening.len()).unwrap_or(u64::MAX);
            self.segment.offset = position.saturating_add(opening.saturating_sub(reserved));
        }
        self.samples.clear();
        self.segment.subsegments.push(next);
        Ok(())
    }
}
