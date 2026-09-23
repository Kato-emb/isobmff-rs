//! [`TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use isobmff_core::{
    AnyBox, BoxDecode, BoxDefinition, BoxEncode, BoxType, ChildBoxes, Error, FieldReader,
    FieldWriter, FourCC, FullBoxFlags, LanguageCode, Mp4EpochSeconds, NullTerminatedString,
    OtherBoxes, U16F16, boxes,
};

use crate::chunk_offset::{ChunkOffsetBox, ChunkOffsets};
use crate::data_entry::{DataEntry, DataEntryUrlBox};
use crate::dinf::DataInformationBox;
use crate::dref::DataReferenceBox;
use crate::hdlr::HandlerBox;
use crate::mdhd::MediaHeaderBox;
use crate::mdia::MediaBox;
use crate::minf::{MediaInformationBox, MediaInformationHeader};
use crate::sample_size::{SampleSizeBox, SampleSizeEntries, SampleSizes};
use crate::stbl::SampleTableBox;
use crate::stsc::SampleToChunkBox;
use crate::stsd::SampleDescriptionBox;
use crate::stts::TimeToSampleBox;
use crate::tkhd::TrackHeaderBox;
use crate::vmhd::VideoMediaHeaderBox;

/// Flags of the `tkhd` of a video track: enabled, in the movie and in the preview
const VIDEO_TRACK_HEADER_FLAGS: FullBoxFlags = match FullBoxFlags::new(0x7) {
    Some(flags) => flags,
    // Why not unwrap: 0x7 is within the 24 bits the field carries, so the flags
    // always build, and a degenerate value stands in for the panic the lints
    // forbid.
    None => FullBoxFlags::ZERO,
};

/// Name the `hdlr` of a video track carries
const VIDEO_HANDLER_NAME: &str = "VideoHandler";

/// Box that holds everything declaring one track
///
/// [`TrackBox`] (`trak`), ISO/IEC 14496-12 §8.3.1. Both children the spec marks
/// mandatory are promoted to fields. An `edts`, which maps the track's media
/// onto the movie's timeline, has no fields yet and is kept in
/// [`other_boxes`](Self::other_boxes) — so the times this crate reports are the
/// media's own, with no edit list applied.
#[doc(alias = "trak")]
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct TrackBox {
    tkhd: TrackHeaderBox,
    mdia: MediaBox,
    other_boxes: OtherBoxes,
}

impl TrackBox {
    /// Creates the box from the two declarations the spec requires
    #[must_use]
    pub const fn new(tkhd: TrackHeaderBox, mdia: MediaBox) -> Self {
        Self {
            tkhd,
            mdia,
            other_boxes: OtherBoxes::new(),
        }
    }

    /// Creates the box of a video track from the declarations a video track varies
    ///
    /// [`new`](Self::new) states every field; `new_video` states the ones a
    /// video track varies and fills the rest with the values of a track
    /// continued in fragments, whose samples the movie fragments carry:
    ///
    /// * `tkhd` (ISO/IEC 14496-12 §8.3.2.3): the flags `track_enabled`,
    ///   `track_in_movie` and `track_in_preview`, a `creation_time`,
    ///   `modification_time` and `duration` of 0,
    ///   and the template values of [`TrackHeaderBox::new`].
    /// * `mdhd` (§8.4.2): a `creation_time`, `modification_time` and
    ///   `duration` of 0, and the language `und`.
    /// * `hdlr` (§8.4.3): the handler type `vide`, named `VideoHandler`.
    /// * `minf` (§8.4.4): a `vmhd` (§12.1.2) of template values, a `dref`
    ///   (§8.7.2) whose one entry places the media data in this file, and an
    ///   `stbl` (§8.5.1) whose `stsd` holds `sample_entry` alone and whose `stts`,
    ///   `stsc`, `stsz` and `stco` are empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use isobmff_boxes::TrackBox;
    /// use isobmff_core::{AnyBox, BoxDecode, BoxEncode, BoxType};
    ///
    /// // The sample entry of the coding, read through the first data reference
    /// let entry = AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0, 0, 0, 0, 0, 0, 0, 1]);
    ///
    /// // A 1920 by 1080 video track on a 90 kHz timescale
    /// let track = TrackBox::new_video(1, 90_000, 1920, 1080, entry);
    /// assert_eq!(track.mdia().mdhd().timescale(), 90_000);
    ///
    /// // The whole box reads back as the value that wrote it
    /// let mut buffer = vec![0; usize::try_from(track.encoded_len()).unwrap()];
    /// track.encode(&mut buffer).unwrap();
    /// assert_eq!(TrackBox::decode(&buffer).unwrap(), (track, b"".as_slice()));
    /// ```
    #[must_use]
    pub fn new_video(
        track_id: u32,
        timescale: u32,
        width: u16,
        height: u16,
        sample_entry: AnyBox,
    ) -> Self {
        let epoch = Mp4EpochSeconds::from_seconds(0);
        let tkhd = TrackHeaderBox::new(
            VIDEO_TRACK_HEADER_FLAGS,
            epoch,
            epoch,
            track_id,
            0,
            U16F16::from_integer(width),
            U16F16::from_integer(height),
        );
        let stbl = SampleTableBox::new(
            SampleDescriptionBox::new(vec![sample_entry]),
            TimeToSampleBox::new(Vec::new()),
            SampleToChunkBox::new(Vec::new()),
            SampleSizes::Stsz(SampleSizeBox::new(SampleSizeEntries::PerSample(Vec::new()))),
            ChunkOffsets::Stco(ChunkOffsetBox::new(Vec::new())),
        );
        let minf = MediaInformationBox::new(
            MediaInformationHeader::Video(VideoMediaHeaderBox::new()),
            DataInformationBox::new(DataReferenceBox::new(vec![DataEntry::Url(
                DataEntryUrlBox::new(None),
            )])),
            stbl,
        );
        let mdia = MediaBox::new(
            MediaHeaderBox::new(epoch, epoch, timescale, 0, LanguageCode::UND),
            HandlerBox::new(
                FourCC::new(*b"vide"),
                // Why not unwrap: the name holds no NUL, so the string always
                // builds, and the empty name stands in for the panic the lints
                // forbid.
                NullTerminatedString::new(String::from(VIDEO_HANDLER_NAME)).unwrap_or_default(),
            ),
            minf,
        );

        Self::new(tkhd, mdia)
    }

    /// Returns the declarations the track applies as a whole
    #[must_use]
    pub const fn tkhd(&self) -> &TrackHeaderBox {
        &self.tkhd
    }

    /// Returns everything declaring the media the track carries
    #[must_use]
    pub const fn mdia(&self) -> &MediaBox {
        &self.mdia
    }

    /// Returns the media of the track, to be changed in place
    #[must_use]
    pub(crate) const fn mdia_mut(&mut self) -> &mut MediaBox {
        &mut self.mdia
    }

    /// Returns the children no field of this box claims, in the order they came
    #[must_use]
    pub fn other_boxes(&self) -> &[AnyBox] {
        self.other_boxes.as_slice()
    }
}

impl BoxDefinition for TrackBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"trak");
}

impl BoxDecode for TrackBox {
    /// # Errors
    ///
    /// * The failures of [`boxes`]: a child does not frame as a box.
    /// * [`MissingMandatoryBox`](isobmff_core::ErrorKind::MissingMandatoryBox): no `tkhd` or
    ///   `mdia`.
    /// * [`DuplicateBox`](isobmff_core::ErrorKind::DuplicateBox): more than one of either.
    /// * Whatever the child reports, on the [`containers`](Error::containers) path: one of them
    ///   does not decode.
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let mut tkhd_boxes = ChildBoxes::new();
        let mut mdia_boxes = ChildBoxes::new();
        let mut other_boxes = OtherBoxes::new();

        for child in boxes(reader.take_remainder()) {
            let child = child?;
            let box_type = child.header().box_type();

            if box_type == TrackHeaderBox::BOX_TYPE {
                tkhd_boxes.push(child);
            } else if box_type == MediaBox::BOX_TYPE {
                mdia_boxes.push(child);
            } else {
                other_boxes.keep(child);
            }
        }

        Ok(Self {
            tkhd: tkhd_boxes.exactly_one()?,
            mdia: mdia_boxes.exactly_one()?,
            other_boxes,
        })
    }
}

impl BoxEncode for TrackBox {
    fn payload_len(&self) -> u64 {
        let others = self
            .other_boxes
            .as_slice()
            .iter()
            .fold(0_u64, |total, other| {
                total.saturating_add(other.encoded_len())
            });

        self.tkhd
            .encoded_len()
            .saturating_add(self.mdia.encoded_len())
            .saturating_add(others)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let mut rest = self.tkhd.encode(writer.take_remainder())?;
        rest = self.mdia.encode(rest)?;
        for other in self.other_boxes.as_slice() {
            rest = other.encode(rest)?;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{
        AnyBox, BoxDecode, BoxEncode, BoxType, Error, FourCC, FullBoxFlags, LanguageCode,
        Mp4EpochSeconds, NullTerminatedString, U16F16,
    };

    use super::TrackBox;
    use crate::dinf::tests::data_information;
    use crate::hdlr::HandlerBox;
    use crate::mdhd::MediaHeaderBox;
    use crate::mdia::MediaBox;
    use crate::minf::MediaInformationHeader;
    use crate::minf::tests::media_information;
    use crate::tkhd::TrackHeaderBox;
    use crate::vmhd::tests::video_media_header;

    /// Track box of a video track, with every mandatory child in place
    pub(crate) fn track() -> TrackBox {
        TrackBox::new(
            TrackHeaderBox::new(
                FullBoxFlags::new(0x3).unwrap(),
                Mp4EpochSeconds::from_seconds(0),
                Mp4EpochSeconds::from_seconds(0),
                1,
                90_000,
                U16F16::from_integer(1920),
                U16F16::from_integer(1080),
            ),
            MediaBox::new(
                MediaHeaderBox::new(
                    Mp4EpochSeconds::from_seconds(0),
                    Mp4EpochSeconds::from_seconds(0),
                    90_000,
                    90_000,
                    LanguageCode::UND,
                ),
                HandlerBox::new(
                    FourCC::new(*b"vide"),
                    NullTerminatedString::new(String::from("VideoHandler")).unwrap(),
                ),
                media_information(),
            ),
        )
    }

    /// Sample entry of an `avc1` coding, read through the first data reference
    fn sample_entry() -> AnyBox {
        AnyBox::from_raw_bytes(BoxType::compact(*b"avc1"), vec![0, 0, 0, 0, 0, 0, 0, 1])
    }

    /// Video track declaring `track_id`, its samples left to the fragments
    pub(crate) fn video_track(track_id: u32) -> TrackBox {
        TrackBox::new_video(track_id, 90_000, 1920, 1080, sample_entry())
    }

    /// Writes the payload of the box and returns the bytes it occupies
    fn encoded_payload(track: &TrackBox) -> Vec<u8> {
        let mut buffer = vec![0; usize::try_from(track.payload_len()).unwrap()];
        track.encode_payload(&mut buffer).unwrap();

        buffer
    }

    #[test]
    fn a_box_reads_back_as_the_value_that_wrote_it() {
        let payload = encoded_payload(&track());

        assert_eq!(TrackBox::decode_payload(&payload).unwrap(), track());
    }

    #[test]
    fn a_box_holding_only_a_track_header_is_rejected() {
        let whole = encoded_payload(&track());
        let track_header_len = usize::try_from(track().tkhd().payload_len()).unwrap() + 8;

        assert_eq!(
            TrackBox::decode_payload(whole.get(..track_header_len).unwrap()),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"mdia")))
        );
    }

    #[test]
    fn a_video_track_states_what_it_varies_and_fills_the_rest_for_fragments() {
        let track = video_track(1);
        let epoch = Mp4EpochSeconds::from_seconds(0);
        let minf = track.mdia().minf();
        let stbl = minf.stbl();

        assert_eq!(
            TrackBox::decode_payload(&encoded_payload(&track)).unwrap(),
            track
        );
        assert_eq!(
            track.tkhd(),
            &TrackHeaderBox::new(
                FullBoxFlags::new(0x7).unwrap(),
                epoch,
                epoch,
                1,
                0,
                U16F16::from_integer(1920),
                U16F16::from_integer(1080),
            )
        );
        assert_eq!(
            track.mdia().mdhd(),
            &MediaHeaderBox::new(epoch, epoch, 90_000, 0, LanguageCode::UND)
        );
        assert_eq!(
            track.mdia().hdlr(),
            &HandlerBox::new(
                FourCC::new(*b"vide"),
                NullTerminatedString::new(String::from("VideoHandler")).unwrap(),
            )
        );
        assert_eq!(
            minf.media_information_header(),
            Some(&MediaInformationHeader::Video(video_media_header()))
        );
        assert_eq!(minf.dinf(), &data_information());
        assert_eq!(stbl.stsd().entries(), [sample_entry()]);
        assert!(stbl.stts().entries().is_empty());
        assert!(stbl.stsc().entries().is_empty());
        assert_eq!(stbl.sample_sizes().sizes().count(), 0);
        assert_eq!(stbl.chunk_offsets().offsets().count(), 0);
    }
}
