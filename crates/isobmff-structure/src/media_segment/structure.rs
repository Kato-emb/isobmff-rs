//! [`MediaSegmentStructure`] and [`MediaSegmentDisposition`], the order of the top-level boxes of a media segment, ISO/IEC 14496-12 §8.16

use isobmff_boxes::{MediaDataBox, MovieFragmentBox, SegmentIndexBox, SegmentTypeBox};
use isobmff_core::{BoxDefinition, BoxType};

use crate::Error;

/// Holds the structure of a media segment, one top-level box at a time
///
/// A media segment carries a portion of a presentation for delivery apart from
/// the movie that declares it (ISO/IEC 14496-12 §8.16.1): the brands it
/// declares itself readable as, then one movie fragment after another with the
/// media data each of them addresses (§3.1.18). Segments concatenated into one
/// stream are read as one, each `styp` where it lies. This machine holds that
/// order. Handed the type of each top-level box as it comes, it answers with
/// the [`MediaSegmentDisposition`] of that box — read whole into a value,
/// passed on as media data, or passed over — and fails on a box the order does
/// not place there. It reads no box itself: what is done with a disposition
/// stays with the caller.
///
/// # Contract
///
/// * A `styp` and a `sidx` are read into values, and an `mdat` is passed on as
///   media data, wherever they lie, none moving the order on more than a box
///   passed over does. A segment carrying no `styp` reads all the same, and no
///   `styp` marks a boundary the order turns on (§8.16.2 lets one not first in
///   its file be ignored). How many `mdat`s there are is not counted.
/// * The `moof` comes any number of times, and at least once: a segment
///   declared over without one is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
/// * Every other box is passed over, wherever it lies — a `moov` among them,
///   since the movie a segment continues is held apart from it.
/// * [`resume`](Self::resume) restarts the order part-way into the segment,
///   at a box an index points at: the next box is a `moof` or a `sidx`,
///   placed as it would be where the boxes before the resume left the order,
///   and any other is [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
/// * An `Err` changes nothing: the structure stands where it stood before the
///   call.
/// * [`finish`](Self::finish) checks that the boxes so far form a whole
///   segment, and changes nothing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MediaSegmentStructure {
    position: Position,
    /// Whether the next box is one an index points at, after a [`resume`](Self::resume)
    resuming: bool,
}

/// What the structure of a media segment makes of a top-level box
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum MediaSegmentDisposition {
    /// Box is read whole into a [`SegmentTypeBox`]
    SegmentType,
    /// Box is read whole into a [`MovieFragmentBox`]
    MovieFragment,
    /// Box is read whole into a [`SegmentIndexBox`]
    SegmentIndex,
    /// Payload of the box is media data, passed on as it arrives
    MediaData,
    /// Box is passed over, payload and all
    Skip,
}

/// How far into the order of a media segment the boxes so far reach
#[derive(Clone, Copy, Debug)]
enum Position {
    /// Before any box
    Start,
    /// After a box, before any fragment
    Opened,
    /// After a fragment
    Fragmenting,
}

impl MediaSegmentStructure {
    /// Creates a structure waiting at the start of a media segment
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            position: Position::Start,
            resuming: false,
        }
    }

    /// Returns whether the structure is reading at the start, before any box
    #[must_use]
    pub(crate) const fn is_at_start(&self) -> bool {
        !self.resuming && matches!(self.position, Position::Start)
    }

    /// Takes the type of the next top-level box, and returns what to do with that box
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): a box
    ///   other than a `moof` or a `sidx` straight after a
    ///   [`resume`](Self::resume).
    pub(crate) fn handle_box_type(
        &mut self,
        box_type: BoxType,
    ) -> Result<MediaSegmentDisposition, Error> {
        if self.resuming
            && !matches!(
                box_type,
                MovieFragmentBox::BOX_TYPE | SegmentIndexBox::BOX_TYPE
            )
        {
            return Err(Error::box_out_of_order(box_type));
        }

        let (reached, disposition) = place(self.position, box_type);
        self.position = reached;
        self.resuming = false;

        Ok(disposition)
    }

    /// Restarts the order part-way into the segment, where the next box is one an index points at
    pub(crate) const fn resume(&mut self) {
        self.resuming = true;
    }

    /// Checks that the boxes so far form a whole segment
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the segment carried no `moof`.
    pub(crate) const fn finish(&self) -> Result<(), Error> {
        match self.position {
            Position::Fragmenting => Ok(()),
            Position::Start | Position::Opened => {
                Err(Error::missing_mandatory_box(MovieFragmentBox::BOX_TYPE))
            }
        }
    }
}

/// Places the box `box_type` names at `position`, and returns where the segment stands past it
const fn place(position: Position, box_type: BoxType) -> (Position, MediaSegmentDisposition) {
    match (box_type, position) {
        (MovieFragmentBox::BOX_TYPE, _any) => (
            Position::Fragmenting,
            MediaSegmentDisposition::MovieFragment,
        ),
        (SegmentTypeBox::BOX_TYPE, _any) => {
            (passed_over(position), MediaSegmentDisposition::SegmentType)
        }
        (MediaDataBox::BOX_TYPE, _any) => {
            (passed_over(position), MediaSegmentDisposition::MediaData)
        }
        (SegmentIndexBox::BOX_TYPE, _any) => {
            (passed_over(position), MediaSegmentDisposition::SegmentIndex)
        }
        (_other, _any) => (passed_over(position), MediaSegmentDisposition::Skip),
    }
}

/// Returns where the segment stands past a box other than a `moof`, standing at `position`
///
/// Any box closes the start of the segment.
const fn passed_over(position: Position) -> Position {
    match position {
        Position::Start => Position::Opened,
        Position::Opened | Position::Fragmenting => position,
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::BoxType;

    use super::{Error, MediaSegmentDisposition, MediaSegmentStructure};

    /// The dispositions of the boxes named, in order, stopping at the first failure
    fn dispositions_of(fourccs: &[&[u8; 4]]) -> Result<Vec<MediaSegmentDisposition>, Error> {
        let mut structure = MediaSegmentStructure::new();

        fourccs
            .iter()
            .map(|fourcc| structure.handle_box_type(BoxType::compact(**fourcc)))
            .collect()
    }

    /// The dispositions of the boxes named, in order, after `before` and a resume, stopping at the first failure
    fn dispositions_resuming_after(
        before: &[&[u8; 4]],
        fourccs: &[&[u8; 4]],
    ) -> Result<Vec<MediaSegmentDisposition>, Error> {
        let mut structure = MediaSegmentStructure::new();
        for fourcc in before {
            structure.handle_box_type(BoxType::compact(**fourcc))?;
        }

        structure.resume();

        fourccs
            .iter()
            .map(|fourcc| structure.handle_box_type(BoxType::compact(**fourcc)))
            .collect()
    }

    #[test]
    fn a_resumed_segment_goes_on_from_a_fragment_or_an_index() {
        assert_eq!(
            dispositions_resuming_after(&[b"styp", b"moof", b"mdat"], &[b"moof", b"mdat"]),
            Ok(vec![
                MediaSegmentDisposition::MovieFragment,
                MediaSegmentDisposition::MediaData,
            ])
        );
        assert_eq!(
            dispositions_resuming_after(&[b"styp"], &[b"sidx", b"moof"]),
            Ok(vec![
                MediaSegmentDisposition::SegmentIndex,
                MediaSegmentDisposition::MovieFragment,
            ])
        );
    }

    #[test]
    fn a_resumed_segment_starting_on_a_box_no_index_points_at_is_out_of_order() {
        assert_eq!(
            dispositions_resuming_after(&[b"styp", b"moof"], &[b"mdat"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"mdat")))
        );
        assert_eq!(
            dispositions_resuming_after(&[b"styp", b"moof"], &[b"styp"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"styp")))
        );
    }

    #[test]
    fn a_segment_resumed_after_its_check_passed_goes_on_from_where_it_stood() {
        let mut structure = MediaSegmentStructure::new();
        structure
            .handle_box_type(BoxType::compact(*b"moof"))
            .unwrap();
        structure.finish().unwrap();

        structure.resume();

        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"sidx")),
            Ok(MediaSegmentDisposition::SegmentIndex)
        );
        assert_eq!(structure.finish(), Ok(()));
    }

    #[test]
    fn the_boxes_of_a_media_segment_are_read_passed_on_or_passed_over_in_turn() {
        assert_eq!(
            dispositions_of(&[
                b"styp", b"sidx", b"moof", b"mdat", b"moov", b"moof", b"mdat", b"mdat", b"mfra",
            ]),
            Ok(vec![
                MediaSegmentDisposition::SegmentType,
                MediaSegmentDisposition::SegmentIndex,
                MediaSegmentDisposition::MovieFragment,
                MediaSegmentDisposition::MediaData,
                MediaSegmentDisposition::Skip,
                MediaSegmentDisposition::MovieFragment,
                MediaSegmentDisposition::MediaData,
                MediaSegmentDisposition::MediaData,
                MediaSegmentDisposition::Skip,
            ])
        );
    }

    #[test]
    fn a_segment_declaring_no_brands_is_read_all_the_same() {
        assert_eq!(
            dispositions_of(&[b"moof", b"mdat"]),
            Ok(vec![
                MediaSegmentDisposition::MovieFragment,
                MediaSegmentDisposition::MediaData,
            ])
        );
    }

    #[test]
    fn brands_and_media_data_are_read_wherever_they_lie() {
        assert_eq!(
            [
                dispositions_of(&[b"styp", b"moof", b"mdat", b"styp", b"moof", b"mdat"]),
                dispositions_of(&[b"styp", b"mdat", b"moof"]),
                dispositions_of(&[b"moof", b"mdat", b"styp"]),
            ],
            [
                Ok(vec![
                    MediaSegmentDisposition::SegmentType,
                    MediaSegmentDisposition::MovieFragment,
                    MediaSegmentDisposition::MediaData,
                    MediaSegmentDisposition::SegmentType,
                    MediaSegmentDisposition::MovieFragment,
                    MediaSegmentDisposition::MediaData,
                ]),
                Ok(vec![
                    MediaSegmentDisposition::SegmentType,
                    MediaSegmentDisposition::MediaData,
                    MediaSegmentDisposition::MovieFragment,
                ]),
                Ok(vec![
                    MediaSegmentDisposition::MovieFragment,
                    MediaSegmentDisposition::MediaData,
                    MediaSegmentDisposition::SegmentType,
                ]),
            ]
        );
    }

    #[test]
    fn a_segment_declared_over_without_a_fragment_is_rejected() {
        let mut structure = MediaSegmentStructure::new();

        structure
            .handle_box_type(BoxType::compact(*b"styp"))
            .unwrap();
        structure
            .handle_box_type(BoxType::compact(*b"mdat"))
            .unwrap();

        assert_eq!(
            structure.finish(),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"moof")))
        );
    }

    #[test]
    fn a_segment_of_a_fragment_alone_is_a_media_segment() {
        let mut structure = MediaSegmentStructure::new();

        structure
            .handle_box_type(BoxType::compact(*b"moof"))
            .unwrap();

        assert_eq!(structure.finish(), Ok(()));
    }
}
