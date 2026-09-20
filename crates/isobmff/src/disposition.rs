//! [`Disposition`], what a structure has done with a top-level box

/// What a structure has done with a top-level box of a file
///
/// A structure is handed the header of each top-level box and answers with
/// one of these: the box is read whole into the value it names, its payload is
/// passed on as media data, or it is passed over. Which boxes are read into
/// values is the structure's to say, so each such box is a variant of its own,
/// and the value it is read into is the one the variant is named after.
///
/// The boxes a structure reads into values are added to as ISO/IEC 14496-12 is
/// read further, so a match on this must leave room for variants that are not
/// here yet.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Disposition {
    /// Box is read whole into a [`FileTypeBox`](isobmff_boxes::FileTypeBox)
    FileType,
    /// Box is read whole into a [`MovieBox`](isobmff_boxes::MovieBox)
    Movie,
    /// Box is read whole into a [`MovieFragmentBox`](isobmff_boxes::MovieFragmentBox)
    MovieFragment,
    /// Payload of the box is media data, passed on as it arrives
    MediaData,
    /// Box is passed over, payload and all
    Skip,
}
