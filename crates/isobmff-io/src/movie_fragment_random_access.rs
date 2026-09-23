//! [`Probe`], the search for the `mfra` closing a file by the `mfro` closing it, ISO/IEC 14496-12 §8.8.11

use isobmff_boxes::{MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox};
use isobmff_core::{BoxDecode, BoxDefinition, BoxHeader};

/// Bytes read at each part of the file a search probes
///
/// As many as the `mfro` closing the file occupies, and as the header of an
/// `mfra` does with a `largesize`.
pub(crate) const PROBE_LEN: u64 = 16;

/// A search for the `mfra` closing a file, standing at the part of it whose first bytes it reads next
///
/// A driver reads up to [`PROBE_LEN`] bytes at [`start`](Self::start) and
/// hands them to [`handle`](Self::handle), until the search settles.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Probe {
    /// At the last bytes of a file `file_len` long, where an `mfro` closes it
    Closing { file_len: u64 },
    /// At `start`, where the `mfro` says the `mfra` spanning the rest of the file begins
    Opening { file_len: u64, start: u64 },
}

/// What a [`Probe`] made of the bytes it read
#[derive(Debug)]
pub(crate) enum Probed {
    /// The search reads on at another part of the file
    Next(Probe),
    /// The search settled: where the `mfra` begins, or `None` when the file closes with none
    Settled(Option<u64>),
}

impl Probe {
    /// Starts a search in a file `file_len` long, `None` when it is too short to close with an `mfro`
    pub(crate) fn new(file_len: u64) -> Option<Self> {
        (file_len >= PROBE_LEN).then_some(Self::Closing { file_len })
    }

    /// Returns the offset of the file whose bytes the search reads next
    pub(crate) const fn start(self) -> u64 {
        match self {
            Self::Closing { file_len } => file_len.saturating_sub(PROBE_LEN),
            Self::Opening { start, .. } => start,
        }
    }

    /// Takes the bytes read at [`start`](Self::start), fewer than [`PROBE_LEN`] where the file ends before
    ///
    /// An `mfro` steps back by its `size` from the end of the file, and the
    /// bytes there are to open an `mfra` spanning that many; an `mfra`
    /// declaring that it runs to the end of the file spans them.
    pub(crate) fn handle(self, bytes: &[u8]) -> Probed {
        match self {
            Self::Closing { file_len } => {
                let start = MovieFragmentRandomAccessOffsetBox::decode(bytes)
                    .ok()
                    .and_then(|(mfro, _)| file_len.checked_sub(u64::from(mfro.size())));

                start.map_or(Probed::Settled(None), |start| {
                    Probed::Next(Self::Opening { file_len, start })
                })
            }
            Self::Opening { file_len, start } => {
                let spanning = BoxHeader::decode(bytes).is_ok_and(|(header, _)| {
                    header.box_type() == MovieFragmentRandomAccessBox::BOX_TYPE
                        && header
                            .size()
                            .total_bytes()
                            .is_none_or(|total| Some(total) == file_len.checked_sub(start))
                });

                Probed::Settled(spanning.then_some(start))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_boxes::{MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox};
    use isobmff_test_support::written;

    use super::{Probe, Probed};

    /// Where the search settles in a file `file_len` long, the bytes of each part it reads handed over in turn
    fn settled(file_len: u64, parts: &[&[u8]]) -> Option<Option<u64>> {
        let mut probe = Probe::new(file_len)?;
        for bytes in parts {
            match probe.handle(bytes) {
                Probed::Next(next) => probe = next,
                Probed::Settled(start) => return Some(start),
            }
        }

        None
    }

    #[test]
    fn an_mfro_names_the_mfra_spanning_the_bytes_it_steps_back_over() {
        let mfra = written(&MovieFragmentRandomAccessBox::new(vec![]));
        let size = u64::try_from(mfra.len()).unwrap();
        let mfro = written(&MovieFragmentRandomAccessOffsetBox::new(
            u32::try_from(size).unwrap(),
        ));

        assert_eq!(settled(100, &[&mfro, &mfra]), Some(Some(100 - size)));
        assert_eq!(
            settled(100, &[&mfro, b"\0\0\0\0mfra"]),
            Some(Some(100 - size))
        );
        assert_eq!(settled(100, &[&mfro, b"\0\0\0\x08mfra"]), Some(None));
        assert_eq!(settled(100, &[&mfro, b"\0\0\0\x18free"]), Some(None));
    }

    #[test]
    fn a_file_closing_with_no_mfro_or_one_reaching_past_its_start_has_none() {
        let mfro = written(&MovieFragmentRandomAccessOffsetBox::new(24));

        assert_eq!(settled(100, &[b"\0\0\0\x10freeFREEFREE"]), Some(None));
        assert_eq!(settled(23, &[&mfro]), Some(None));
        assert_eq!(settled(15, &[]), None);
    }
}
