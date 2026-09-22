//! [`FragmentedStructure`] and [`FragmentedDisposition`], the order of the top-level boxes of a fragmented movie file, ISO/IEC 14496-12 Annex A.8

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox, MovieFragmentBox};
use isobmff_core::{BoxDefinition, BoxType};

use crate::Error;

/// Holds the structure of a fragmented movie file, one top-level box at a time
///
/// A fragmented movie file is laid out as ISO/IEC 14496-12 Annex A.8 has it:
/// the brands it declares itself readable as, the movie its fragments
/// continue, then one movie fragment after another with the media data each
/// of them addresses. This machine holds that order. Handed the type of each
/// top-level box as it comes, it answers with the [`FragmentedDisposition`]
/// of that box — read whole into a value, passed on as media data, or passed
/// over — and fails on a box the order does not place there. It reads no box
/// itself: what is done with a disposition stays with the caller.
///
/// # Contract
///
/// * The `ftyp` comes first, as early as §4.3 asks: a file carrying none reads
///   all the same, as §4.3 allows, but one carrying it after any other box —
///   a second `ftyp` among them — is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
/// * The `moov` comes once, and before any fragment: a second is
///   [`DuplicateBox`](crate::ErrorKind::DuplicateBox), and a `moof`
///   arriving before it is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder). A file
///   declared over without one is
///   [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox).
/// * The `mdat` comes after a fragment: one arriving before any `moof` is
///   [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder). How many
///   follow a fragment is not counted.
/// * Every other box is passed over, wherever it lies.
/// * An `Err` leaves the structure failed for good,
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again.
/// * [`finish`](Self::finish) declares the file over. A header handed over
///   then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished).
#[derive(Clone, Copy, Debug)]
pub(crate) struct FragmentedStructure {
    state: State,
}

/// What the structure of a fragmented movie file makes of a top-level box
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum FragmentedDisposition {
    /// Box is read whole into a [`FileTypeBox`]
    FileType,
    /// Box is read whole into a [`MovieBox`]
    Movie,
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
    /// Told the file is over, and taking no more headers
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(Error),
}

/// How far into the order of a fragmented movie file the boxes so far reach
#[derive(Clone, Copy, Debug)]
enum Position {
    /// Before any box, where the `ftyp` may still come
    Start,
    /// After a box, waiting for the `moov`
    Opened,
    /// After the `moov`, waiting for the first fragment
    Declared,
    /// After a fragment, where media data may follow
    Fragmenting,
}

impl FragmentedStructure {
    /// Creates a structure waiting at the start of a fragmented movie file
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
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): an
    ///   `ftyp` after another box, a `moof` before the `moov`, or an `mdat`
    ///   before any `moof`.
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): a second
    ///   `moov`.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub(crate) fn handle_box_type(
        &mut self,
        box_type: BoxType,
    ) -> Result<FragmentedDisposition, Error> {
        let position = match self.state {
            State::Reading(position) => position,
            State::Finished => return Err(Error::already_finished()),
            State::Failed(failure) => return Err(failure),
        };

        let (reached, disposition) =
            place(position, box_type).map_err(|failure| self.fail(failure))?;
        self.state = State::Reading(reached);

        Ok(disposition)
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the file carried no `moov`.
    /// * [`AlreadyFinished`](crate::ErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub(crate) fn finish(&mut self) -> Result<(), Error> {
        match self.state {
            State::Reading(Position::Declared | Position::Fragmenting) => {
                self.state = State::Finished;

                Ok(())
            }
            State::Reading(Position::Start | Position::Opened) => {
                Err(self.fail(Error::missing_mandatory_box(MovieBox::BOX_TYPE)))
            }
            State::Finished => Err(Error::already_finished()),
            State::Failed(failure) => Err(failure),
        }
    }

    /// Fails the structure for good, and hands the failure back to report
    const fn fail(&mut self, failure: Error) -> Error {
        self.state = State::Failed(failure);

        failure
    }
}

/// Places the box `box_type` names at `position`, and returns where the file stands past it
const fn place(
    position: Position,
    box_type: BoxType,
) -> Result<(Position, FragmentedDisposition), Error> {
    match (box_type, position) {
        (FileTypeBox::BOX_TYPE, Position::Start) => {
            Ok((Position::Opened, FragmentedDisposition::FileType))
        }
        (FileTypeBox::BOX_TYPE, Position::Opened | Position::Declared | Position::Fragmenting)
        | (MovieFragmentBox::BOX_TYPE, Position::Start | Position::Opened)
        | (MediaDataBox::BOX_TYPE, Position::Start | Position::Opened | Position::Declared) => {
            Err(Error::box_out_of_order(box_type))
        }
        (MovieBox::BOX_TYPE, Position::Start | Position::Opened) => {
            Ok((Position::Declared, FragmentedDisposition::Movie))
        }
        (MovieBox::BOX_TYPE, Position::Declared | Position::Fragmenting) => {
            Err(Error::duplicate_box(box_type))
        }
        (MovieFragmentBox::BOX_TYPE, Position::Declared | Position::Fragmenting) => {
            Ok((Position::Fragmenting, FragmentedDisposition::MovieFragment))
        }
        (MediaDataBox::BOX_TYPE, Position::Fragmenting) => {
            Ok((Position::Fragmenting, FragmentedDisposition::MediaData))
        }
        (_other, Position::Start) => Ok((Position::Opened, FragmentedDisposition::Skip)),
        (_other, Position::Opened | Position::Declared | Position::Fragmenting) => {
            Ok((position, FragmentedDisposition::Skip))
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::BoxType;

    use super::{Error, FragmentedDisposition, FragmentedStructure};

    /// The dispositions of the boxes named, in order, stopping at the first failure
    fn dispositions_of(fourccs: &[&[u8; 4]]) -> Result<Vec<FragmentedDisposition>, Error> {
        let mut structure = FragmentedStructure::new();

        fourccs
            .iter()
            .map(|fourcc| structure.handle_box_type(BoxType::compact(**fourcc)))
            .collect()
    }

    #[test]
    fn the_boxes_of_a_fragmented_movie_file_are_read_passed_on_or_passed_over_in_turn() {
        assert_eq!(
            dispositions_of(&[
                b"ftyp", b"free", b"moov", b"free", b"moof", b"mdat", b"moof", b"mdat", b"mdat",
                b"mfra",
            ]),
            Ok(vec![
                FragmentedDisposition::FileType,
                FragmentedDisposition::Skip,
                FragmentedDisposition::Movie,
                FragmentedDisposition::Skip,
                FragmentedDisposition::MovieFragment,
                FragmentedDisposition::MediaData,
                FragmentedDisposition::MovieFragment,
                FragmentedDisposition::MediaData,
                FragmentedDisposition::MediaData,
                FragmentedDisposition::Skip,
            ])
        );
    }

    #[test]
    fn a_file_declaring_no_brands_is_read_all_the_same() {
        assert_eq!(
            dispositions_of(&[b"moov", b"moof", b"mdat"]),
            Ok(vec![
                FragmentedDisposition::Movie,
                FragmentedDisposition::MovieFragment,
                FragmentedDisposition::MediaData,
            ])
        );
    }

    #[test]
    fn brands_declared_after_another_box_are_out_of_order() {
        let out_of_order = Err(Error::box_out_of_order(BoxType::compact(*b"ftyp")));

        assert_eq!(dispositions_of(&[b"free", b"ftyp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"moov", b"ftyp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"ftyp", b"ftyp"]), out_of_order);
    }

    #[test]
    fn a_second_movie_is_rejected() {
        assert_eq!(
            dispositions_of(&[b"ftyp", b"moov", b"moof", b"moov"]),
            Err(Error::duplicate_box(BoxType::compact(*b"moov")))
        );
    }

    #[test]
    fn a_fragment_arriving_before_the_movie_is_out_of_order() {
        assert_eq!(
            dispositions_of(&[b"ftyp", b"moof"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"moof")))
        );
    }

    #[test]
    fn media_data_arriving_before_any_fragment_is_out_of_order() {
        assert_eq!(
            dispositions_of(&[b"ftyp", b"moov", b"mdat"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"mdat")))
        );
        assert_eq!(
            dispositions_of(&[b"mdat"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"mdat")))
        );
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        let mut structure = FragmentedStructure::new();

        structure
            .handle_box_type(BoxType::compact(*b"ftyp"))
            .unwrap();

        assert_eq!(
            structure.finish(),
            Err(Error::missing_mandatory_box(BoxType::compact(*b"moov")))
        );
    }

    #[test]
    fn a_file_of_the_movie_alone_is_a_fragmented_movie_file() {
        let mut structure = FragmentedStructure::new();

        structure
            .handle_box_type(BoxType::compact(*b"moov"))
            .unwrap();

        assert_eq!(structure.finish(), Ok(()));
    }

    #[test]
    fn a_failed_structure_reports_the_same_failure_for_every_call_after_it() {
        let mut structure = FragmentedStructure::new();
        let failure = Error::box_out_of_order(BoxType::compact(*b"moof"));

        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"moof")),
            Err(failure)
        );
        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"moov")),
            Err(failure)
        );
        assert_eq!(structure.finish(), Err(failure));
    }

    #[test]
    fn a_header_handed_over_after_finishing_is_rejected() {
        let mut structure = FragmentedStructure::new();

        structure
            .handle_box_type(BoxType::compact(*b"moov"))
            .unwrap();
        structure.finish().unwrap();

        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"moof")),
            Err(Error::already_finished())
        );
        assert_eq!(structure.finish(), Err(Error::already_finished()));
    }
}
