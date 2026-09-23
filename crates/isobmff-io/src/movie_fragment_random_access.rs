//! Where the `mfra` closing a file begins, found from the `mfro` closing it, ISO/IEC 14496-12 §8.8.11

use isobmff_boxes::{MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader};

/// Bytes read at each part of the file a search probes
///
/// As many as the `mfro` closing the file occupies, and as the header of an
/// `mfra` does with a `largesize`.
pub(crate) const PROBE_LEN: u64 = 16;

/// Returns where the `mfra` begins that the `mfro` in `tail`, the last bytes of a file `file_len` long, names
///
/// `None` when `tail` is not an `mfro`, or its `size` reaches back past the
/// start of the file.
pub(crate) fn start_named_by(tail: &[u8], file_len: u64) -> Option<u64> {
    let (mfro, _) = MovieFragmentRandomAccessOffsetBox::decode(tail).ok()?;

    file_len.checked_sub(u64::from(mfro.size()))
}

/// Returns whether `head`, the first bytes of the `size` bytes closing a file, opens an `mfra` spanning them
///
/// An `mfra` declaring that it runs to the end of the file spans them.
pub(crate) fn opens_movie_fragment_random_access(head: &[u8], size: u64) -> bool {
    BoxHeader::decode(head).is_ok_and(|(header, _)| {
        header.box_type() == MovieFragmentRandomAccessBox::BOX_TYPE
            && header
                .size()
                .total_bytes()
                .is_none_or(|total| total == size)
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::{MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox};
    use isobmff_test_support::written;

    use super::{opens_movie_fragment_random_access, start_named_by};

    #[test]
    fn an_mfro_names_the_start_of_the_mfra_by_stepping_back_its_size_from_the_end() {
        let tail = written(&MovieFragmentRandomAccessOffsetBox::new(24));

        assert_eq!(start_named_by(&tail, 100), Some(76));
        assert_eq!(start_named_by(&tail, 23), None);
        assert_eq!(start_named_by(b"\0\0\0\x10freeFREEFREE", 100), None);
    }

    #[test]
    fn an_mfra_opens_the_bytes_closing_the_file_only_when_its_header_spans_them_all() {
        let mfra = written(&MovieFragmentRandomAccessBox::new(vec![]));
        let size = u64::try_from(mfra.len()).unwrap();

        assert!(opens_movie_fragment_random_access(&mfra, size));
        assert!(!opens_movie_fragment_random_access(&mfra, size + 1));
        assert!(!opens_movie_fragment_random_access(b"\0\0\0\x18free", size));
        assert!(opens_movie_fragment_random_access(b"\0\0\0\0mfra", size));
    }
}
