//! [`Disposition`], what a structure makes of a top-level box

/// What a structure makes of a top-level box of a file
///
/// A structure is handed the type of each top-level box and answers with
/// one of these: the box is read whole into the value it names, its payload is
/// passed on as media data, or it is passed over. Which boxes are read into
/// values is the structure's to say, so each such box is a variant of its own,
/// and the value it is read into is the one the variant is named after.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Disposition {
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
