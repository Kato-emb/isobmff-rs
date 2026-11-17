//! [`SegmentMuxer`], which writes a movie whose samples lie in movie fragments in segments, each indexed by a segment index where asked

use alloc::boxed::Box;
use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, SegmentTypeBox};

use super::output::to_bytes;
use super::segment_index::{Index, SegmentIndex, SegmentIndexParameters};
use super::{Error, FragmentedMovieFileMuxer, MovieParameters, MuxerConfig, TrackParameters};
use crate::SampleProperties;

/// Writer of a movie in segments: an initialization segment, the `ftyp` and a `moov` that declares the tracks and no sample, and media segments of movie fragments, each indexed by a `sidx` where asked
///
/// The segments of ISO/IEC 14496-12 §8.14. Each media segment opens with a
/// [`SegmentTypeBox`] (`styp`), §8.14.2, and holds one or more movie
/// fragments as [`FragmentedMovieFileMuxer`] writes them:
/// [`finish_fragment`](Self::finish_fragment) ends one within the segment,
/// such as a CMAF chunk, and [`finish_segment`](Self::finish_segment) ends
/// the last and the segment.
///
/// Given [`SegmentIndexParameters`], each media segment is the Indexed Media
/// Segment of ISO/IEC 23009-1 (MPEG-DASH): after its `styp` comes a
/// [`SegmentIndexBox`](isobmff_boxes::SegmentIndexBox) (`sidx`), §8.14.3,
/// of version 1, that indexes the track
/// [`SegmentIndexParameters::reference_track_id`] names, each movie fragment
/// one subsegment, up to [`SegmentIndexParameters::subsegment_count`] of
/// them. The times of the `sidx` are the presentation times of the reference
/// track, its edit list applied, §8.14.3.1, in the timescale of its media.
/// Since a `sidx` comes before the subsegments it indexes, the muxer writes a
/// `free` of its length before the first, and returns the `sidx` from
/// [`finish_segment`](Self::finish_segment) and [`finish`](Self::finish) for
/// the caller to write over it, at an offset in the run of bytes.
///
/// The muxer is driven as a [`FragmentedMovieFileMuxer`] is. The segments
/// follow one another in one run of bytes, which reads as one fragmented
/// movie file; a caller that keeps each segment apart polls after the first
/// sample, for the initialization segment, and after each
/// [`finish_segment`](Self::finish_segment) and [`finish`](Self::finish), for
/// a media segment, and takes from the offset of a `sidx` the offset its
/// segment starts at. Where a fragment and a segment end is the caller's to
/// choose.
///
/// Segments of one movie fragment each:
///
/// ```
/// use isobmff::mux::{MovieParameters, TrackParameters};
/// use isobmff::{MuxerConfig, SampleProperties, SegmentMuxer};
/// use isobmff_boxes::{FileTypeBox, SampleFlags, SegmentTypeBox};
/// use isobmff_core::{BoxDecode, FourCc};
///
/// // A movie of one track, whose sample entry is that of a file
/// let file: &[u8] = include_bytes!("../../tests/fixtures/segment.mp4");
/// let (file_type, rest) = FileTypeBox::decode(file)?;
/// let (moov, _) = isobmff_boxes::MovieBox::decode(rest)?;
/// let mut segment_type = SegmentTypeBox::default();
/// segment_type.major_brand = FourCc::new(*b"msdh");
/// let mut muxer = SegmentMuxer::new(MovieParameters::from(&moov), MuxerConfig::default(), file_type, segment_type, None)?;
/// muxer.add_track(TrackParameters::from(&moov.trak[0]))?;
///
/// // Write two segments of one sample each, keeping each segment apart
/// let sample = |decoding_time| SampleProperties {
///     track_id: moov.trak[0].tkhd.track_id,
///     decoding_time,
///     sample_duration: 1024,
///     sample_composition_time_offset: 0,
///     sample_flags: SampleFlags::default(),
///     sample_description_index: 1,
/// };
/// let (mut initialization, mut first, mut second) = (Vec::new(), Vec::new(), Vec::new());
/// muxer.handle_sample(sample(0), b"abc")?;
/// muxer.poll_output(&mut initialization);
/// muxer.finish_segment()?;
/// muxer.poll_output(&mut first);
/// muxer.handle_sample(sample(1024), b"de")?;
/// muxer.finish()?;
/// muxer.poll_output(&mut second);
///
/// // The initialization segment opens with the `ftyp`, and each media segment with the `styp`
/// let box_type = |segment: &[u8]| segment.get(4..8).map(<[u8]>::to_vec);
/// assert_eq!(box_type(&initialization), Some(b"ftyp".to_vec()));
/// assert_eq!(box_type(&first), Some(b"styp".to_vec()));
/// assert_eq!(box_type(&second), Some(b"styp".to_vec()));
/// assert!(first.ends_with(b"abc") && second.ends_with(b"de"));
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
///
/// A media segment indexed by a `sidx`, of two subsegments:
///
/// ```
/// use std::num::NonZeroU16;
///
/// use isobmff::demux::Event;
/// use isobmff::mux::{MovieParameters, SegmentIndexParameters, TrackParameters};
/// use isobmff::structure_data::subsegments;
/// use isobmff::{Demuxer, DemuxerConfig, MuxerConfig, SampleProperties, SegmentMuxer};
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
/// let mut muxer = SegmentMuxer::new(MovieParameters::from(&moov), MuxerConfig::default(), file_type, segment_type, Some(index))?;
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
/// muxer.finish_fragment()?;
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
pub struct SegmentMuxer {
    muxer: FragmentedMovieFileMuxer,
    /// The bytes each media segment opens with: the `styp`, and the `free`
    /// reserved for the `sidx`, the last of them, where segments are indexed
    opening: Vec<u8>,
    segments: Segments,
}

/// What a [`SegmentMuxer`] holds of the media segment being written
#[derive(Debug, Clone)]
enum Segments {
    /// Segments not indexed
    Plain {
        // Why hold whether the segment is open rather than ask the fragmented
        // muxer: a fragment ended within the segment leaves it nothing that
        // tells the next fragment goes on in the same segment.
        /// Whether a fragment of the segment was written, after which the
        /// next is written without the `styp`
        opened: bool,
    },
    /// Segments indexed by a `sidx`
    Indexed(Box<Index>),
}

impl SegmentMuxer {
    /// Creates a muxer of the movie `movie` describes, whose initialization segment opens with `file_type` and each media segment with `segment_type`, and a segment index as `index` sets it where there is one, and holds the `ftyp`
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::new`], and [`Error::Encode`] where the
    /// `styp` does not write.
    pub fn new(
        movie: MovieParameters,
        config: MuxerConfig,
        file_type: FileTypeBox,
        segment_type: SegmentTypeBox,
        index: Option<SegmentIndexParameters>,
    ) -> Result<Self, Error> {
        let mut opening = to_bytes(&segment_type)?;
        let segments = match index {
            None => Segments::Plain { opened: false },
            Some(parameters) => {
                Segments::Indexed(Box::new(Index::new(parameters, &movie, &mut opening)?))
            }
        };
        Ok(Self {
            muxer: FragmentedMovieFileMuxer::new(movie, config, file_type)?,
            opening,
            segments,
        })
    }

    /// Adds the track `track` describes, before the first sample
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::add_track`].
    pub fn add_track(&mut self, track: TrackParameters) -> Result<(), Error> {
        match &mut self.segments {
            Segments::Plain { .. } => self.muxer.add_track(track),
            Segments::Indexed(index) => index.add_track(&mut self.muxer, track),
        }
    }

    /// Takes the next sample of the fragment, with its bytes, and holds them until the fragment ends; the first sample writes the initialization segment first
    ///
    /// # Errors
    /// * [`Error::UnknownTrack`]: no track of the sample's `track_ID`, or, where
    ///   segments are indexed, of
    ///   [`SegmentIndexParameters::reference_track_id`], was added.
    /// * [`Error::SubsegmentCount`]: segments are indexed, and the sample
    ///   would open a subsegment of a media segment that holds as many as
    ///   [`SegmentIndexParameters::subsegment_count`]; the caller ends the
    ///   segment, and hands the sample over again, the first of the next.
    /// * As [`FragmentedMovieFileMuxer::handle_sample`].
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let Segments::Indexed(index) = &mut self.segments else {
            return self.muxer.handle_sample(properties, bytes);
        };
        index.check_sample()?;
        self.muxer.handle_sample(properties, bytes)?;
        index.take_sample(properties);
        Ok(())
    }

    /// Ends the movie fragment within the media segment, and holds its `moof` and its `mdat` for [`poll_output`](Self::poll_output), after the `styp`, and the `free` reserved for the `sidx`, where it is the first of its segment
    ///
    /// Where segments are indexed, the fragment is one subsegment. Where it
    /// holds no sample, nothing is written.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Encode`]: the `moof` does not write.
    ///
    /// Where segments are indexed:
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
    pub fn finish_fragment(&mut self) -> Result<(), Error> {
        match &mut self.segments {
            Segments::Plain { opened } => {
                let held = self.muxer.has_fragment();
                self.muxer
                    .write_fragment(if *opened { &[] } else { &self.opening })?;
                *opened |= held;
                Ok(())
            }
            Segments::Indexed(index) => index.finish_fragment(&mut self.muxer, &self.opening),
        }
    }

    /// Ends the media segment, its last fragment as [`finish_fragment`](Self::finish_fragment) does, and returns the `sidx` that indexes it, or `None` where segments are not indexed or the segment holds no subsegment
    ///
    /// The `sidx` gives the last subsegment the duration up to where the
    /// presentation of the reference track ends in it: the first subsegment
    /// of the next media segment is refused, with [`Error::SubsegmentTime`],
    /// where it does not start there.
    ///
    /// # Errors
    /// As [`finish_fragment`](Self::finish_fragment), and
    /// [`Error::SubsegmentDuration`] where the last subsegment lasts longer
    /// than a `subsegment_duration` counts.
    pub fn finish_segment(&mut self) -> Result<Option<SegmentIndex>, Error> {
        match &mut self.segments {
            Segments::Plain { .. } => {
                self.finish_fragment()?;
                self.segments = Segments::Plain { opened: false };
                Ok(None)
            }
            Segments::Indexed(index) => index.finish_segment(&mut self.muxer, &self.opening),
        }
    }

    /// Appends the bytes the muxer holds to `out`, and holds none
    pub fn poll_output(&mut self, out: &mut Vec<u8>) {
        let start = out.len();
        self.muxer.poll_output(out);
        if let Segments::Indexed(index) = &mut self.segments {
            index.polled(out.len().saturating_sub(start));
        }
    }

    /// Ends the last media segment, as [`finish_segment`](Self::finish_segment) does, or writes the initialization segment where no sample was taken, and returns the `sidx` of the segment where there is one
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
}
