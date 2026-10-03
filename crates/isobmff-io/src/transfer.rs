//! What the two halves of the crate share: [`cut_length`], how much a source reads at a time, [`ClosingMovieFragmentRandomAccessOffset`], where a source looks for an `mfra`, and [`ChunkBuffer`], what a sink is writing

use std::io;

use isobmff_boxes::MovieFragmentRandomAccessOffsetBox;
use isobmff_core::BoxDecode;
use isobmff_sequence::EventBytes;

/// Most bytes read off the source at a time
pub(crate) const CUT_LENGTH: usize = 1024 * 1024;

/// Returns how many bytes a read of `length` takes: all of them up to [`CUT_LENGTH`], or that many where it is `None`
pub(crate) fn cut_length(length: Option<u64>) -> usize {
    length
        .and_then(|length| usize::try_from(length).ok())
        .map_or(CUT_LENGTH, |length| length.min(CUT_LENGTH))
}

/// Where the `mfro` closing a file lies when it closes with an `mfra`, ISO/IEC 14496-12 §8.8.11
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClosingMovieFragmentRandomAccessOffset {
    file_len: u64,
    start: u64,
}

impl ClosingMovieFragmentRandomAccessOffset {
    /// The bytes an `mfro` occupies
    pub(crate) const LEN: usize = 16;

    /// Returns where the `mfro` closing a file `file_len` long lies, `None` when the file is too short for one
    pub(crate) fn of(file_len: u64) -> Option<Self> {
        file_len
            .checked_sub(Self::LEN as u64)
            .map(|start| Self { file_len, start })
    }

    /// Returns the position of the `mfro` in a resource the file lies at `origin` of
    pub(crate) const fn position(self, origin: u64) -> u64 {
        // Why not checked_add: the file was measured from `origin` to where
        // the resource ends, so the sum lies within what 64 bits carry.
        origin.saturating_add(self.start)
    }

    /// Returns the offset of the `mfra` the bytes read at the `mfro` step back to, if they are an `mfro` stepping back within the file
    pub(crate) fn movie_fragment_random_access_start(self, mfro: &[u8; Self::LEN]) -> Option<u64> {
        MovieFragmentRandomAccessOffsetBox::decode(mfro)
            .ok()
            .and_then(|(mfro, _)| mfro.movie_fragment_random_access_start(self.file_len))
    }
}

/// The chunk a sink is writing and how many of its bytes the sink took
#[derive(Debug)]
pub(crate) struct ChunkBuffer {
    chunk: EventBytes,
    written: usize,
}

impl ChunkBuffer {
    /// Returns the chunk `buffer` holds part-written, or once it is written whole, the next of `chunks` put in its place
    pub(crate) fn unwritten<'buffer>(
        buffer: &'buffer mut Option<Self>,
        chunks: &mut impl Iterator<Item = EventBytes>,
    ) -> Option<&'buffer mut Self> {
        while buffer
            .as_ref()
            .is_none_or(|buffer| buffer.written >= buffer.chunk.len())
        {
            *buffer = Some(Self {
                chunk: chunks.next()?,
                written: 0,
            });
        }

        buffer.as_mut()
    }

    /// Returns the bytes of the chunk the sink is to take next
    pub(crate) fn rest(&self) -> &[u8] {
        self.chunk.get(self.written..).unwrap_or_default()
    }

    /// Counts what the sink made of the bytes [`rest`](Self::rest) returned
    ///
    /// An interrupted write takes none of them, which the next [`rest`](Self::rest) returns again.
    ///
    /// # Errors
    ///
    /// * The sink refused the bytes, or took none of them
    ///   ([`WriteZero`](io::ErrorKind::WriteZero)).
    pub(crate) fn took(&mut self, written: io::Result<usize>) -> io::Result<()> {
        match written {
            Err(failure) if failure.kind() == io::ErrorKind::Interrupted => Ok(()),
            Ok(0) => Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(written) => {
                self.written = self.written.saturating_add(written);

                Ok(())
            }
            Err(failure) => Err(failure),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::vec::Vec;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use isobmff_boxes::MovieFragmentRandomAccessOffsetBox;
    use isobmff_core::{BoxHeader, BoxType};
    use isobmff_sequence::{BoxEvent, BoxWriter, EventBytes};
    use isobmff_test_support::written;

    use super::{CUT_LENGTH, ClosingMovieFragmentRandomAccessOffset, cut_length};

    /// Polls `future` once and hands over what it gave, dropping it where it stood
    pub(crate) fn poll_once<F: Future>(future: F) -> Option<F::Output> {
        let mut future = pin!(future);

        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        }
    }

    /// The bytes a `free` box of `payload` is framed as, one `EventBytes` a step
    pub(crate) fn framed(payload: &[u8]) -> Vec<EventBytes> {
        let mut boxes = BoxWriter::new();
        let header =
            BoxHeader::with_payload_len(BoxType::compact(*b"free"), payload.len() as u64).unwrap();

        boxes.handle_event(BoxEvent::Header(header)).unwrap();
        boxes
            .handle_event(BoxEvent::Payload(payload.to_vec()))
            .unwrap();
        boxes.handle_event(BoxEvent::End).unwrap();

        core::iter::from_fn(|| boxes.poll_output()).collect()
    }

    #[test]
    fn a_read_takes_the_length_asked_for_up_to_a_cut_and_a_cut_where_none_is_asked_for() {
        assert_eq!(
            [Some(4), Some(u64::MAX), None].map(cut_length),
            [4, CUT_LENGTH, CUT_LENGTH]
        );
    }

    #[test]
    fn an_mfro_stepping_back_within_the_file_names_where_the_mfra_begins() {
        let mfro: [u8; ClosingMovieFragmentRandomAccessOffset::LEN] =
            written(&MovieFragmentRandomAccessOffsetBox::new(24))
                .try_into()
                .unwrap();
        let closing = ClosingMovieFragmentRandomAccessOffset::of(100).unwrap();

        assert_eq!(closing.movie_fragment_random_access_start(&mfro), Some(76));
    }

    #[test]
    fn a_file_closing_with_no_mfro_or_one_stepping_back_past_its_start_or_too_short_for_one_names_none()
     {
        let past_the_start: [u8; ClosingMovieFragmentRandomAccessOffset::LEN] =
            written(&MovieFragmentRandomAccessOffsetBox::new(u32::MAX))
                .try_into()
                .unwrap();
        let closing = ClosingMovieFragmentRandomAccessOffset::of(100).unwrap();

        assert_eq!(
            [
                closing.movie_fragment_random_access_start(&[0; ClosingMovieFragmentRandomAccessOffset::LEN]),
                closing.movie_fragment_random_access_start(&past_the_start),
                ClosingMovieFragmentRandomAccessOffset::of(15)
                    .and_then(|closing| closing.movie_fragment_random_access_start(&past_the_start)),
            ],
            [None, None, None]
        );
    }
}
