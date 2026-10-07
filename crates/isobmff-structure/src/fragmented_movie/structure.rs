//! [`FragmentedStructure`] and [`FragmentedDisposition`], the order of the top-level boxes of a fragmented movie file, ISO/IEC 14496-12 Annex A.8

use isobmff_boxes::{
    FileTypeBox, MediaDataBox, MovieBox, MovieFragmentBox, MovieFragmentRandomAccessBox,
    SegmentIndexBox,
};
use isobmff_core::{BoxDefinition, BoxType};

use crate::Error;

/// Holds the structure of a fragmented movie file, one top-level box at a time
///
/// A fragmented movie file is laid out as ISO/IEC 14496-12 Annex A.8 has it:
/// the brands it declares itself readable as, the movie its fragments
/// continue, then one movie fragment after another, with the media data the
/// movie and its fragments address lying anywhere among them (§8.1.1). A
/// non-fragmented movie file (§8.2.1), that order with no fragment after the
/// movie, is held by it as well. This machine holds that order. Handed the type of each
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
/// * An `mdat` is passed on as media data wherever it lies, before the `moov`
///   as well. How many there are is not counted.
/// * A `sidx` and an `mfra` are read into values wherever they lie, and move
///   the order on as a box passed over does.
/// * Every other box is passed over, wherever it lies.
/// * [`resume`](Self::resume) restarts the order part-way into the file, at a
///   box an index points at: the next box is a `moof`, a `sidx` or an `mfra`,
///   placed as it would be where the boxes before the resume left the order,
///   and any other is [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder).
///   What those boxes established stands: a `moof` still needs the `moov`
///   to have come, and a second `moov` is still a duplicate.
/// * An `Err` changes nothing: the structure stands where it stood before the
///   call.
/// * [`finish`](Self::finish) checks that the boxes so far form a whole file,
///   and changes nothing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FragmentedStructure {
    position: Position,
    /// Whether the next box is one an index points at, after a [`resume`](Self::resume)
    resuming: bool,
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
    /// Box is read whole into a [`SegmentIndexBox`]
    SegmentIndex,
    /// Box is read whole into a [`MovieFragmentRandomAccessBox`]
    MovieFragmentRandomAccess,
    /// Payload of the box is media data, passed on as it arrives
    MediaData,
    /// Box is passed over, payload and all
    Skip,
}

/// How far into the order of a fragmented movie file the boxes so far reach
#[derive(Clone, Copy, Debug)]
enum Position {
    /// Before any box, where the `ftyp` may still come
    Start,
    /// After a box, waiting for the `moov`
    Opened,
    /// After the `moov`
    Declared,
}

impl FragmentedStructure {
    /// Creates a structure waiting at the start of a fragmented movie file
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            position: Position::Start,
            resuming: false,
        }
    }

    /// Returns whether no box has been placed yet, where the `ftyp` may still come
    #[must_use]
    pub(crate) const fn is_at_start(&self) -> bool {
        !self.resuming && matches!(self.position, Position::Start)
    }

    /// Takes the type of the next top-level box, and returns what to do with that box
    ///
    /// # Errors
    ///
    /// * [`BoxOutOfOrder`](crate::ErrorKind::BoxOutOfOrder): an
    ///   `ftyp` after another box, or a `moof` before the `moov`.
    /// * [`DuplicateBox`](crate::ErrorKind::DuplicateBox): a second
    ///   `moov`.
    pub(crate) fn handle_box_type(
        &mut self,
        box_type: BoxType,
    ) -> Result<FragmentedDisposition, Error> {
        if self.resuming
            && !matches!(
                box_type,
                MovieFragmentBox::BOX_TYPE
                    | SegmentIndexBox::BOX_TYPE
                    | MovieFragmentRandomAccessBox::BOX_TYPE
            )
        {
            return Err(Error::box_out_of_order(box_type));
        }

        let (reached, disposition) = place(self.position, box_type)?;
        self.position = reached;
        self.resuming = false;

        Ok(disposition)
    }

    /// Restarts the order part-way into the file, where the next box is one an index points at
    pub(crate) const fn resume(&mut self) {
        self.resuming = true;
    }

    /// Checks that the boxes so far form a whole file
    ///
    /// # Errors
    ///
    /// * [`MissingMandatoryBox`](crate::ErrorKind::MissingMandatoryBox):
    ///   the file carried no `moov`.
    pub(crate) const fn finish(&self) -> Result<(), Error> {
        match self.position {
            Position::Declared => Ok(()),
            Position::Start | Position::Opened => {
                Err(Error::missing_mandatory_box(MovieBox::BOX_TYPE))
            }
        }
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
        (FileTypeBox::BOX_TYPE, Position::Opened | Position::Declared)
        | (MovieFragmentBox::BOX_TYPE, Position::Start | Position::Opened) => {
            Err(Error::box_out_of_order(box_type))
        }
        (MovieBox::BOX_TYPE, Position::Start | Position::Opened) => {
            Ok((Position::Declared, FragmentedDisposition::Movie))
        }
        (MovieBox::BOX_TYPE, Position::Declared) => Err(Error::duplicate_box(box_type)),
        (MovieFragmentBox::BOX_TYPE, Position::Declared) => {
            Ok((Position::Declared, FragmentedDisposition::MovieFragment))
        }
        (MediaDataBox::BOX_TYPE, _any) => {
            Ok((passed_over(position), FragmentedDisposition::MediaData))
        }
        (SegmentIndexBox::BOX_TYPE, _any) => {
            Ok((passed_over(position), FragmentedDisposition::SegmentIndex))
        }
        (MovieFragmentRandomAccessBox::BOX_TYPE, _any) => Ok((
            passed_over(position),
            FragmentedDisposition::MovieFragmentRandomAccess,
        )),
        (_other, _any) => Ok((passed_over(position), FragmentedDisposition::Skip)),
    }
}

/// Returns where the file stands past a box the order is not built of, standing at `position`
///
/// Any box closes the start of the file, past which the `ftyp` is out of order.
const fn passed_over(position: Position) -> Position {
    match position {
        Position::Start => Position::Opened,
        Position::Opened | Position::Declared => position,
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

    /// The dispositions of the boxes named, in order, after `before` and a resume, stopping at the first failure
    fn dispositions_resuming_after(
        before: &[&[u8; 4]],
        fourccs: &[&[u8; 4]],
    ) -> Result<Vec<FragmentedDisposition>, Error> {
        let mut structure = FragmentedStructure::new();
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
    fn a_resumed_file_goes_on_from_a_fragment_or_an_index() {
        assert_eq!(
            dispositions_resuming_after(&[b"ftyp", b"moov", b"moof"], &[b"moof", b"mdat"]),
            Ok(vec![
                FragmentedDisposition::MovieFragment,
                FragmentedDisposition::MediaData,
            ])
        );
        assert_eq!(
            dispositions_resuming_after(&[b"ftyp", b"moov"], &[b"mfra"]),
            Ok(vec![FragmentedDisposition::MovieFragmentRandomAccess])
        );
        assert_eq!(
            dispositions_resuming_after(&[b"ftyp", b"moov"], &[b"sidx", b"moof"]),
            Ok(vec![
                FragmentedDisposition::SegmentIndex,
                FragmentedDisposition::MovieFragment,
            ])
        );
    }

    #[test]
    fn a_resumed_file_starting_on_a_box_no_index_points_at_is_out_of_order() {
        assert_eq!(
            dispositions_resuming_after(&[b"ftyp", b"moov", b"moof"], &[b"mdat"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"mdat")))
        );
    }

    #[test]
    fn a_resumed_file_keeps_the_order_the_boxes_before_the_resume_established() {
        assert_eq!(
            dispositions_resuming_after(&[b"ftyp"], &[b"moof"]),
            Err(Error::box_out_of_order(BoxType::compact(*b"moof")))
        );
        assert_eq!(
            dispositions_resuming_after(&[b"moov"], &[b"moof", b"moov"]),
            Err(Error::duplicate_box(BoxType::compact(*b"moov")))
        );
    }

    #[test]
    fn a_file_resumed_after_its_check_passed_goes_on_from_where_it_stood() {
        let mut structure = FragmentedStructure::new();
        structure
            .handle_box_type(BoxType::compact(*b"moov"))
            .unwrap();
        structure.finish().unwrap();

        structure.resume();

        assert_eq!(
            structure.handle_box_type(BoxType::compact(*b"moof")),
            Ok(FragmentedDisposition::MovieFragment)
        );
        assert_eq!(structure.finish(), Ok(()));
    }

    #[test]
    fn the_boxes_of_a_fragmented_movie_file_are_read_passed_on_or_passed_over_in_turn() {
        assert_eq!(
            dispositions_of(&[
                b"ftyp", b"free", b"moov", b"sidx", b"moof", b"mdat", b"moof", b"mdat", b"mdat",
                b"mfra",
            ]),
            Ok(vec![
                FragmentedDisposition::FileType,
                FragmentedDisposition::Skip,
                FragmentedDisposition::Movie,
                FragmentedDisposition::SegmentIndex,
                FragmentedDisposition::MovieFragment,
                FragmentedDisposition::MediaData,
                FragmentedDisposition::MovieFragment,
                FragmentedDisposition::MediaData,
                FragmentedDisposition::MediaData,
                FragmentedDisposition::MovieFragmentRandomAccess,
            ])
        );
    }

    #[test]
    fn only_a_structure_no_box_was_placed_in_yet_stands_at_the_start() {
        let mut structure = FragmentedStructure::new();

        assert!(structure.is_at_start());

        structure
            .handle_box_type(BoxType::compact(*b"moov"))
            .unwrap();

        assert!(!structure.is_at_start());

        structure.finish().unwrap();

        assert!(!structure.is_at_start());
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
    fn media_data_is_passed_on_wherever_it_lies() {
        assert_eq!(
            [
                dispositions_of(&[b"ftyp", b"moov", b"mdat", b"moof", b"mdat"]),
                dispositions_of(&[b"ftyp", b"mdat", b"moov", b"moof", b"mdat"]),
                dispositions_of(&[b"mdat", b"moov"]),
            ],
            [
                Ok(vec![
                    FragmentedDisposition::FileType,
                    FragmentedDisposition::Movie,
                    FragmentedDisposition::MediaData,
                    FragmentedDisposition::MovieFragment,
                    FragmentedDisposition::MediaData,
                ]),
                Ok(vec![
                    FragmentedDisposition::FileType,
                    FragmentedDisposition::MediaData,
                    FragmentedDisposition::Movie,
                    FragmentedDisposition::MovieFragment,
                    FragmentedDisposition::MediaData,
                ]),
                Ok(vec![
                    FragmentedDisposition::MediaData,
                    FragmentedDisposition::Movie,
                ]),
            ]
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
}
