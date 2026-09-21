//! [`MediaSegmentStructure`] and [`MediaSegmentDisposition`], the order of the top-level boxes of a media segment, ISO/IEC 14496-12 §8.16

use isobmff_boxes::{MediaDataBox, MovieFragmentBox, SegmentTypeBox};
use isobmff_core::{BoxDefinition, BoxType};

use crate::StructureError;

/// Holds the structure of a media segment, one top-level box at a time
///
/// A media segment carries a portion of a presentation for delivery apart
/// from the movie that declares it (ISO/IEC 14496-12 §8.16.1): the brands it
/// declares itself readable as, then one movie fragment after another with
/// the media data each of them addresses (§3.1.18). This machine holds that
/// order. Handed the type of each top-level box as it comes, it answers with
/// the [`MediaSegmentDisposition`] of that box — read whole into a value,
/// passed on as media data, or passed over — and fails on a box the order
/// does not place there. It reads no box itself: what is done with a
/// disposition stays with the caller.
///
/// # Contract
///
/// * The `styp` comes first, as §8.16.2 asks: a segment carrying none reads
///   all the same, but one carrying it after any other box — a second `styp`
///   among them, as the segments of a presentation concatenated into one
///   file carry — is
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder).
/// * The `moof` comes any number of times, and at least once: a segment
///   declared over without one is
///   [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox).
/// * The `mdat` comes after a fragment: one arriving before any `moof` is
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder). How many
///   follow a fragment is not counted.
/// * Every other box is passed over, wherever it lies — a `sidx` among them,
///   whose index is not read, and a `moov`, since the movie a segment
///   continues is held apart from it.
/// * An `Err` leaves the structure failed for good,
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again.
/// * [`finish`](Self::finish) declares the segment over. A header handed
///   over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MediaSegmentStructure {
    state: State,
}

/// What the structure of a media segment makes of a top-level box
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum MediaSegmentDisposition {
    /// Box is read whole into a [`SegmentTypeBox`]
    SegmentType,
    /// Box is read whole into a [`MovieFragmentBox`]
    MovieFragment,
    /// Payload of the box is media data, passed on as it arrives
    MediaData,
    /// Box is passed over, payload and all
    Skip,
}

/// Where the structure stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking headers, standing where the boxes so far have brought it
    Reading(Position),
    /// Told the segment is over, and taking no more headers
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(StructureError),
}

/// How far into the order of a media segment the boxes so far reach
#[derive(Clone, Copy, Debug)]
enum Position {
    /// Before any box, where the `styp` may still come
    Start,
    /// After a box, waiting for the first fragment
    Opened,
    /// After a fragment, where media data may follow
    Fragmenting,
}

impl MediaSegmentStructure {
    /// Creates a structure waiting at the start of a media segment
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            state: State::Reading(Position::Start),
        }
    }

    /// Takes the type of the next top-level box, and returns what to do with that box
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder): a
    ///   `styp` after another box, or an `mdat` before any `moof`.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub(crate) fn handle_box_type(
        &mut self,
        box_type: BoxType,
    ) -> Result<MediaSegmentDisposition, StructureError> {
        let position = match self.state {
            State::Reading(position) => position,
            State::Finished => return Err(StructureError::already_finished()),
            State::Failed(failure) => return Err(failure),
        };

        let (reached, disposition) =
            place(position, box_type).map_err(|failure| self.fail(failure))?;
        self.state = State::Reading(reached);

        Ok(disposition)
    }

    /// Declares the segment over
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox):
    ///   the segment carried no `moof`.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   segment was already declared over.
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub(crate) fn finish(&mut self) -> Result<(), StructureError> {
        match self.state {
            State::Reading(Position::Fragmenting) => {
                self.state = State::Finished;

                Ok(())
            }
            State::Reading(Position::Start | Position::Opened) => Err(self.fail(
                StructureError::missing_mandatory_box(MovieFragmentBox::BOX_TYPE),
            )),
            State::Finished => Err(StructureError::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the structure for good, and hands the failure back to report
    const fn fail(&mut self, failure: StructureError) -> StructureError {
        self.state = State::Failed(failure);

        failure
    }
}

/// Places the box `box_type` names at `position`, and returns where the segment stands past it
const fn place(
    position: Position,
    box_type: BoxType,
) -> Result<(Position, MediaSegmentDisposition), StructureError> {
    match (box_type, position) {
        (SegmentTypeBox::BOX_TYPE, Position::Start) => {
            Ok((Position::Opened, MediaSegmentDisposition::SegmentType))
        }
        (SegmentTypeBox::BOX_TYPE, Position::Opened | Position::Fragmenting)
        | (MediaDataBox::BOX_TYPE, Position::Start | Position::Opened) => {
            Err(StructureError::box_out_of_order(box_type))
        }
        (
            MovieFragmentBox::BOX_TYPE,
            Position::Start | Position::Opened | Position::Fragmenting,
        ) => Ok((
            Position::Fragmenting,
            MediaSegmentDisposition::MovieFragment,
        )),
        (MediaDataBox::BOX_TYPE, Position::Fragmenting) => {
            Ok((Position::Fragmenting, MediaSegmentDisposition::MediaData))
        }
        (_other, Position::Start) => Ok((Position::Opened, MediaSegmentDisposition::Skip)),
        (_other, Position::Opened | Position::Fragmenting) => {
            Ok((position, MediaSegmentDisposition::Skip))
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::BoxType;

    use super::{MediaSegmentDisposition, MediaSegmentStructure, StructureError};

    /// The dispositions of the boxes named, in order, stopping at the first failure
    fn dispositions_of(
        fourccs: &[&[u8; 4]],
    ) -> Result<Vec<MediaSegmentDisposition>, StructureError> {
        let mut structure = MediaSegmentStructure::new();

        fourccs
            .iter()
            .map(|fourcc| structure.handle_box_type(BoxType::compact(**fourcc)))
            .collect()
    }

    #[test]
    fn the_boxes_of_a_media_segment_are_read_passed_on_or_passed_over_in_turn() {
        assert_eq!(
            dispositions_of(&[
                b"styp", b"sidx", b"moof", b"mdat", b"moov", b"moof", b"mdat", b"mdat", b"mfra",
            ]),
            Ok(vec![
                MediaSegmentDisposition::SegmentType,
                MediaSegmentDisposition::Skip,
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
    fn brands_declared_after_another_box_are_out_of_order() {
        let out_of_order = Err(StructureError::box_out_of_order(BoxType::compact(*b"styp")));

        assert_eq!(dispositions_of(&[b"free", b"styp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"moof", b"styp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"styp", b"styp"]), out_of_order);
    }

    #[test]
    fn media_data_arriving_before_any_fragment_is_out_of_order() {
        assert_eq!(
            dispositions_of(&[b"styp", b"mdat"]),
            Err(StructureError::box_out_of_order(BoxType::compact(*b"mdat")))
        );
        assert_eq!(
            dispositions_of(&[b"mdat"]),
            Err(StructureError::box_out_of_order(BoxType::compact(*b"mdat")))
        );
    }

    #[test]
    fn a_segment_declared_over_without_a_fragment_is_rejected() {
        let mut structure = MediaSegmentStructure::new();

        structure
            .handle_box_type(BoxType::compact(*b"styp"))
            .unwrap();

        assert_eq!(
            structure.finish(),
            Err(StructureError::missing_mandatory_box(BoxType::compact(
                *b"moof"
            )))
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

    #[test]
    fn a_failed_structure_reports_the_same_failure_for_every_call_after_it() {
        let mut structure = MediaSegmentStructure::new();
        let failure = StructureError::box_out_of_order(BoxType::compact(*b"mdat"));

        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"mdat")),
            Err(failure)
        );
        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"moof")),
            Err(failure)
        );
        assert_eq!(structure.finish(), Err(failure));
    }

    #[test]
    fn a_header_handed_over_after_finishing_is_rejected() {
        let mut structure = MediaSegmentStructure::new();

        structure
            .handle_box_type(BoxType::compact(*b"moof"))
            .unwrap();
        structure.finish().unwrap();

        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"moof")),
            Err(StructureError::already_finished())
        );
        assert_eq!(structure.finish(), Err(StructureError::already_finished()));
    }
}
