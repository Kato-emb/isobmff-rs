//! [`SegmentMuxer`], which writes a movie whose samples lie in movie fragments in segments

use alloc::vec::Vec;

use isobmff_boxes::{FileTypeBox, SegmentTypeBox};

use super::{Error, FragmentedMovieFileMuxer, MovieParameters, MuxerConfig, TrackParameters};
use crate::SampleProperties;

/// Writer of a movie in segments: an initialization segment, the `ftyp` and a `moov` that declares the tracks and no sample, and media segments, each an `styp` and a movie fragment
///
/// The segments of ISO/IEC 14496-12 §8.14, each media segment opening with a
/// [`SegmentTypeBox`] (`styp`), §8.14.2, and holding one movie fragment as
/// [`FragmentedMovieFileMuxer`] writes it.
///
/// The muxer is driven as a [`FragmentedMovieFileMuxer`] is, with
/// [`finish_segment`](Self::finish_segment) in place of `finish_fragment`.
/// The segments follow one another in one run of bytes, which reads as one
/// fragmented movie file; a caller that keeps each segment apart polls after
/// the first sample, for the initialization segment, and after each
/// [`finish_segment`](Self::finish_segment) and [`finish`](Self::finish), for
/// a media segment. Where a segment ends is the caller's to choose; the muxer
/// writes no segment index.
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
/// let mut muxer = SegmentMuxer::new(MovieParameters::from(&moov), MuxerConfig::default(), file_type, segment_type)?;
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
#[derive(Debug, Clone)]
pub struct SegmentMuxer {
    muxer: FragmentedMovieFileMuxer,
    segment_type: SegmentTypeBox,
}

impl SegmentMuxer {
    /// Creates a muxer of the movie `movie` describes, whose initialization segment opens with `file_type` and each media segment with `segment_type`, and holds the `ftyp`
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::new`].
    pub fn new(
        movie: MovieParameters,
        config: MuxerConfig,
        file_type: FileTypeBox,
        segment_type: SegmentTypeBox,
    ) -> Result<Self, Error> {
        Ok(Self {
            muxer: FragmentedMovieFileMuxer::new(movie, config, file_type)?,
            segment_type,
        })
    }

    /// Adds the track `track` describes, before the first sample
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::add_track`].
    pub fn add_track(&mut self, track: TrackParameters) -> Result<(), Error> {
        self.muxer.add_track(track)
    }

    /// Takes the next sample of the segment, with its bytes, and holds them until the segment ends; the first sample writes the initialization segment first
    ///
    /// # Errors
    /// As [`FragmentedMovieFileMuxer::handle_sample`].
    pub fn handle_sample(
        &mut self,
        properties: SampleProperties,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.muxer.handle_sample(properties, bytes)
    }

    /// Ends the media segment, and holds its `styp`, its `moof` and its `mdat` for [`poll_output`](Self::poll_output)
    ///
    /// Where the segment holds no sample, nothing is written.
    ///
    /// # Errors
    /// * [`Error::Finished`]: the muxer has finished.
    /// * [`Error::Encode`]: the `styp` or the `moof` does not write.
    pub fn finish_segment(&mut self) -> Result<(), Error> {
        self.muxer.write_fragment(Some(&self.segment_type))
    }

    /// Appends the bytes the muxer holds to `out`, and holds none
    pub fn poll_output(&mut self, out: &mut Vec<u8>) {
        self.muxer.poll_output(out);
    }

    /// Ends the last media segment, or writes the initialization segment where no sample was taken, and holds it for [`poll_output`](Self::poll_output)
    ///
    /// The muxer then takes nothing more: it ends the file, unlike the
    /// `finish` of a writer of structure-data, which ends one unit and goes on.
    ///
    /// # Errors
    /// As [`finish_segment`](Self::finish_segment), and
    /// [`Error::Encode`] where the `moov` does not write.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.muxer.finish_after(Some(&self.segment_type))
    }
}
