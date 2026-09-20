//! [`FragmentedStructure`], the order of the top-level boxes of a fragmented movie file, ISO/IEC 14496-12 Annex A.8

use isobmff_boxes::{FileTypeBox, MediaDataBox, MovieBox, MovieFragmentBox};
use isobmff_core::{BoxDefinition, BoxHeader, BoxType};

use crate::{Disposition, StructureError};

/// Holds the structure of a fragmented movie file, one top-level box at a time
///
/// A fragmented movie file is laid out as ISO/IEC 14496-12 Annex A.8 has it:
/// the brands it declares itself readable as, the movie its fragments
/// continue, then one movie fragment after another with the media data each
/// of them addresses. This machine holds that order. Handed the header of each
/// top-level box as it comes, it answers with the [`Disposition`] of that box
/// — read whole into a value, passed on as media data, or passed over — and
/// fails on a box the order does not place there. It reads no box itself:
/// what is done with a disposition stays with the caller.
///
/// # Contract
///
/// * The `ftyp` comes first, as early as §4.3 asks: a file carrying none reads
///   all the same, as §4.3 allows, but one carrying it after any other box —
///   a second `ftyp` among them — is
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder).
/// * The `moov` comes once, and before any fragment: a second is
///   [`DuplicateBox`](crate::StructureErrorKind::DuplicateBox), and a `moof`
///   arriving before it is
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder). A file
///   declared over without one is
///   [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox).
/// * The `mdat` comes after a fragment: one arriving before any `moof` is
///   [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder). How many
///   follow a fragment is not counted.
/// * Every other box is passed over, wherever it lies.
/// * An `Err` leaves the structure failed for good,
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished) aside:
///   every later call reports that same failure again.
/// * [`finish`](Self::finish) declares the file over. A header handed over
///   then, or a second [`finish`](Self::finish), is
///   [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished).
///
/// # Examples
///
/// ```
/// use isobmff::{BoxHeader, BoxType, Disposition, FragmentedStructure};
///
/// let header = |fourcc: &[u8; 4]| BoxHeader::with_payload_len(BoxType::compact(*fourcc), 16).unwrap();
/// let mut structure = FragmentedStructure::new();
///
/// // The brands and the movie are read into values
/// assert_eq!(structure.handle_header(header(b"ftyp"))?, Disposition::FileType);
/// assert_eq!(structure.handle_header(header(b"moov"))?, Disposition::Movie);
///
/// // A fragment is read into a value, and the media data beside it passed on
/// assert_eq!(structure.handle_header(header(b"moof"))?, Disposition::MovieFragment);
/// assert_eq!(structure.handle_header(header(b"mdat"))?, Disposition::MediaData);
///
/// // Any other box is passed over
/// assert_eq!(structure.handle_header(header(b"free"))?, Disposition::Skip);
/// structure.finish()?;
/// # Ok::<(), isobmff::StructureError>(())
/// ```
#[derive(Clone, Copy, Debug)]
pub struct FragmentedStructure {
    state: State,
}

/// Where the structure stands between calls
#[derive(Clone, Copy, Debug)]
enum State {
    /// Taking headers, standing where the boxes so far have brought it
    Reading(Position),
    /// Told the file is over, and taking no more headers
    Finished,
    /// Failed, and reporting that same failure for every call after it
    Failed(StructureError),
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
    pub const fn new() -> Self {
        Self {
            state: State::Reading(Position::Start),
        }
    }

    /// Takes the header of the next top-level box, and returns what to do with that box
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::StructureErrorKind::BoxOutOfOrder): an
    ///   `ftyp` after another box, a `moof` before the `moov`, or an `mdat`
    ///   before any `moof`.
    /// * [`DuplicateBox`](crate::StructureErrorKind::DuplicateBox): a second
    ///   `moov`.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   file was declared over by [`finish`](Self::finish).
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub fn handle_header(&mut self, header: BoxHeader) -> Result<Disposition, StructureError> {
        let position = match self.state {
            State::Reading(position) => position,
            State::Finished => return Err(StructureError::already_finished()),
            State::Failed(failure) => return Err(failure),
        };

        let (reached, disposition) =
            place(position, header.box_type()).map_err(|failure| self.fail(failure))?;
        self.state = State::Reading(reached);

        Ok(disposition)
    }

    /// Declares the file over
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::StructureErrorKind::MissingMandatoryBox):
    ///   the file carried no `moov`.
    /// * [`AlreadyFinished`](crate::StructureErrorKind::AlreadyFinished): the
    ///   file was already declared over.
    /// * The failure of a previous call, which the structure keeps and reports
    ///   again for every call after it.
    pub fn finish(&mut self) -> Result<(), StructureError> {
        match self.state {
            State::Reading(Position::Declared | Position::Fragmenting) => {
                self.state = State::Finished;

                Ok(())
            }
            State::Reading(Position::Start | Position::Opened) => {
                Err(self.fail(StructureError::missing_mandatory_box(MovieBox::BOX_TYPE)))
            }
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

impl Default for FragmentedStructure {
    fn default() -> Self {
        Self::new()
    }
}

/// Places the box `box_type` names at `position`, and returns where the file stands past it
const fn place(
    position: Position,
    box_type: BoxType,
) -> Result<(Position, Disposition), StructureError> {
    match (box_type, position) {
        (FileTypeBox::BOX_TYPE, Position::Start) => Ok((Position::Opened, Disposition::FileType)),
        (FileTypeBox::BOX_TYPE, Position::Opened | Position::Declared | Position::Fragmenting)
        | (MovieFragmentBox::BOX_TYPE, Position::Start | Position::Opened)
        | (MediaDataBox::BOX_TYPE, Position::Start | Position::Opened | Position::Declared) => {
            Err(StructureError::box_out_of_order(box_type))
        }
        (MovieBox::BOX_TYPE, Position::Start | Position::Opened) => {
            Ok((Position::Declared, Disposition::Movie))
        }
        (MovieBox::BOX_TYPE, Position::Declared | Position::Fragmenting) => {
            Err(StructureError::duplicate_box(box_type))
        }
        (MovieFragmentBox::BOX_TYPE, Position::Declared | Position::Fragmenting) => {
            Ok((Position::Fragmenting, Disposition::MovieFragment))
        }
        (MediaDataBox::BOX_TYPE, Position::Fragmenting) => {
            Ok((Position::Fragmenting, Disposition::MediaData))
        }
        (_other, Position::Start) => Ok((Position::Opened, Disposition::Skip)),
        (_other, Position::Opened | Position::Declared | Position::Fragmenting) => {
            Ok((position, Disposition::Skip))
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use isobmff_core::{BoxHeader, BoxType};

    use super::{Disposition, FragmentedStructure, StructureError};

    /// Header of a top-level box of `fourcc`, whose payload is not looked at
    fn header(fourcc: &[u8; 4]) -> BoxHeader {
        BoxHeader::with_payload_len(BoxType::compact(*fourcc), 16).unwrap()
    }

    /// The dispositions of the boxes named, in order, stopping at the first failure
    fn dispositions_of(fourccs: &[&[u8; 4]]) -> Result<Vec<Disposition>, StructureError> {
        let mut structure = FragmentedStructure::new();

        fourccs
            .iter()
            .map(|fourcc| structure.handle_header(header(fourcc)))
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
                Disposition::FileType,
                Disposition::Skip,
                Disposition::Movie,
                Disposition::Skip,
                Disposition::MovieFragment,
                Disposition::MediaData,
                Disposition::MovieFragment,
                Disposition::MediaData,
                Disposition::MediaData,
                Disposition::Skip,
            ])
        );
    }

    #[test]
    fn a_file_declaring_no_brands_is_read_all_the_same() {
        assert_eq!(
            dispositions_of(&[b"moov", b"moof", b"mdat"]),
            Ok(vec![
                Disposition::Movie,
                Disposition::MovieFragment,
                Disposition::MediaData,
            ])
        );
    }

    #[test]
    fn brands_declared_after_another_box_are_out_of_order() {
        let out_of_order = Err(StructureError::box_out_of_order(BoxType::compact(*b"ftyp")));

        assert_eq!(dispositions_of(&[b"free", b"ftyp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"moov", b"ftyp"]), out_of_order);
        assert_eq!(dispositions_of(&[b"ftyp", b"ftyp"]), out_of_order);
    }

    #[test]
    fn a_second_movie_is_rejected() {
        assert_eq!(
            dispositions_of(&[b"ftyp", b"moov", b"moof", b"moov"]),
            Err(StructureError::duplicate_box(BoxType::compact(*b"moov")))
        );
    }

    #[test]
    fn a_fragment_arriving_before_the_movie_is_out_of_order() {
        assert_eq!(
            dispositions_of(&[b"ftyp", b"moof"]),
            Err(StructureError::box_out_of_order(BoxType::compact(*b"moof")))
        );
    }

    #[test]
    fn media_data_arriving_before_any_fragment_is_out_of_order() {
        assert_eq!(
            dispositions_of(&[b"ftyp", b"moov", b"mdat"]),
            Err(StructureError::box_out_of_order(BoxType::compact(*b"mdat")))
        );
        assert_eq!(
            dispositions_of(&[b"mdat"]),
            Err(StructureError::box_out_of_order(BoxType::compact(*b"mdat")))
        );
    }

    #[test]
    fn a_file_declared_over_without_a_movie_is_rejected() {
        let mut structure = FragmentedStructure::new();

        structure.handle_header(header(b"ftyp")).unwrap();

        assert_eq!(
            structure.finish(),
            Err(StructureError::missing_mandatory_box(BoxType::compact(
                *b"moov"
            )))
        );
    }

    #[test]
    fn a_file_of_the_movie_alone_is_a_fragmented_movie_file() {
        let mut structure = FragmentedStructure::new();

        structure.handle_header(header(b"moov")).unwrap();

        assert_eq!(structure.finish(), Ok(()));
    }

    #[test]
    fn a_failed_structure_reports_the_same_failure_for_every_call_after_it() {
        let mut structure = FragmentedStructure::new();
        let failure = StructureError::box_out_of_order(BoxType::compact(*b"moof"));

        assert_eq!(structure.handle_header(header(b"moof")), Err(failure));
        assert_eq!(structure.handle_header(header(b"moov")), Err(failure));
        assert_eq!(structure.finish(), Err(failure));
    }

    #[test]
    fn a_header_handed_over_after_finishing_is_rejected() {
        let mut structure = FragmentedStructure::new();

        structure.handle_header(header(b"moov")).unwrap();
        structure.finish().unwrap();

        assert_eq!(
            structure.handle_header(header(b"moof")),
            Err(StructureError::already_finished())
        );
        assert_eq!(structure.finish(), Err(StructureError::already_finished()));
    }
}
