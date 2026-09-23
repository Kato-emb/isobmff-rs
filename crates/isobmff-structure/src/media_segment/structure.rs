//! [`MediaSegmentStructure`] and [`MediaSegmentDisposition`], the order of the top-level boxes of a media segment, ISO/IEC 14496-12 §8.16

use isobmff_boxes::{MediaDataBox, MovieFragmentBox, SegmentIndexBox, SegmentTypeBox};
use isobmff_core::{BoxDefinition, BoxType};

use crate::Error;

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
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
/// * The `moof` comes any number of times, and at least once: a segment
///   declared over without one is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
/// * The `mdat` comes after a fragment: one arriving before any `moof` is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder). How many
///   follow a fragment is not counted.
/// * A `sidx` is read into a value wherever it lies, and moves the order on as
///   a box passed over does.
/// * Every other box is passed over, wherever it lies — a `moov` among them,
///   since the movie a segment continues is held apart from it.
/// * [`resume`](Self::resume) restarts the order part-way into the segment,
///   at a box an index points at: the next box is a `moof` or a `sidx`,
///   placed as it would be where the boxes before the resume left the order,
///   and any other is [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
///   The structure resumes from the segment declared over as well, and a
///   failed one stays failed.
/// * An `Err` leaves the structure failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again.
/// * [`finish`](Self::finish) declares the segment over. A header handed
///   over then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
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
    /// Box is read whole into a [`SegmentIndexBox`]
    SegmentIndex,
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
    /// Restarted part-way into the segment, where the boxes before it had brought it, taking a box an index points at next
    Resuming(Position),
    /// Told the segment is over, where the boxes so far had brought it, and taking no more headers
    Finished(Position),
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
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
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): a
    ///   `styp` after another box, or an `mdat` before any `moof`.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub(crate) fn handle_box_type(
        &mut self,
        box_type: BoxType,
    ) -> Result<MediaSegmentDisposition, Error> {
        let placed = match (self.state, box_type) {
            (State::Reading(position), _)
            | (State::Resuming(position), MovieFragmentBox::BOX_TYPE | SegmentIndexBox::BOX_TYPE) => {
                place(position, box_type)
            }
            (State::Resuming(_position), _other) => Err(Error::box_out_of_order(box_type)),
            (State::Finished(_position), _any) => return Err(Error::already_finished()),
            (State::Failed(failure), _any) => return Err(failure),
        };

        let (reached, disposition) = placed.map_err(|failure| self.fail(failure))?;
        self.state = State::Reading(reached);

        Ok(disposition)
    }

    /// Restarts the order part-way into the segment, where the next box is one an index points at
    pub(crate) const fn resume(&mut self) {
        match self.state {
            State::Reading(position) | State::Resuming(position) | State::Finished(position) => {
                self.state = State::Resuming(position);
            }
            State::Failed(_failure) => {}
        }
    }

    /// Declares the segment over
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the segment carried no `moof`.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   segment was already declared over.
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub(crate) fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            State::Reading(position) | State::Resuming(position) => match position {
                Position::Fragmenting => {
                    self.state = State::Finished(position);

                    Ok(())
                }
                Position::Start | Position::Opened => {
                    Err(self.fail(Error::missing_mandatory_box(MovieFragmentBox::BOX_TYPE)))
                }
            },
            State::Finished(_position) => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the structure for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

/// Places the box `box_type` names at `position`, and returns where the segment stands past it
const fn place(
    position: Position,
    box_type: BoxType,
) -> Result<(Position, MediaSegmentDisposition), Error> {
    match (box_type, position) {
        (SegmentTypeBox::BOX_TYPE, Position::Start) => {
            Ok((Position::Opened, MediaSegmentDisposition::SegmentType))
        }
        (SegmentTypeBox::BOX_TYPE, Position::Opened | Position::Fragmenting)
        | (MediaDataBox::BOX_TYPE, Position::Start | Position::Opened) => {
            Err(Error::box_out_of_order(box_type))
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
        (SegmentIndexBox::BOX_TYPE, _any) => {
            Ok((passed_over(position), MediaSegmentDisposition::SegmentIndex))
        }
        (_other, _any) => Ok((passed_over(position), MediaSegmentDisposition::Skip)),
    }
}

/// Returns where the segment stands past a box the order is not built of, standing at `position`
///
/// Any box closes the start of the segment, past which the `styp` is out of
/// order.
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
    }

    #[test]
    fn a_segment_declared_over_resumes_and_is_declared_over_again() {
        let mut structure = MediaSegmentStructure::new();
        structure
            .handle_box_type(BoxType::compact(*b"moof"))
            .unwrap();
        structure.finish().unwrap();

        structure.resume();

        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"moof")),
            Ok(MediaSegmentDisposition::MovieFragment)
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
    fn brands_declared_after_another_box_are_out_of_order() {
        let out_of_order = Err(Error::box_out_of_order(BoxType::compact(*b"styp")));

        assert_eq!(dispositions_of(&[b"free", b"styp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"moof", b"styp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"styp", b"styp"]), out_of_order);
    }

    #[test]
    fn media_data_arriving_before_any_fragment_is_out_of_order() {
        assert_eq!(
            dispositions_of(&[b"styp", b"mdat"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"mdat")))
        );
        assert_eq!(
            dispositions_of(&[b"mdat"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"mdat")))
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

    #[test]
    fn a_failed_structure_reports_the_same_failure_for_every_call_after_it() {
        let mut structure = MediaSegmentStructure::new();
        let failure = Error::box_out_of_order(BoxType::compact(*b"mdat"));

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
            Err(Error::already_finished())
        );
        assert_eq!(structure.finish(), Err(Error::already_finished()));
    }
}
