//! [`Source`], a file read at the offsets a demux FSM names, off an asynchronous source that seeks
//!
//! The source over `std::io` is written apart, in
//! [`blocking`](crate::blocking).

use alloc::vec;
use alloc::vec::Vec;
use core::future::poll_fn;
use core::pin::Pin;
use std::io::{self, SeekFrom};

use futures_io::{AsyncRead, AsyncSeek};

use crate::transfer::{CUT_LENGTH, ClosingMovieFragmentRandomAccessOffset, cut_length};

/// Reads off `source` into `into`, and returns how many bytes came
async fn read<S: AsyncRead + Unpin>(source: &mut S, into: &mut [u8]) -> io::Result<usize> {
    poll_fn(|context| Pin::new(&mut *source).poll_read(context, into)).await
}

/// Moves `source` to `position`, and returns where it stands
async fn seek<S: AsyncSeek + Unpin>(source: &mut S, position: SeekFrom) -> io::Result<u64> {
    poll_fn(|context| Pin::new(&mut *source).poll_seek(context, position)).await
}

/// A file read at an offset at a time, off an asynchronous source that seeks
///
/// # Contract
///
/// * The file begins where the source stands when this is created, and every
///   seek is made from there: a file lying at some position in a larger
///   resource is read by seeking the source to it first.
/// * A [`read_at`](Self::read_at) seeks the source only where it does not
///   stand at the offset, and is one `read` of it, made again where
///   interrupted, of up to 1 MiB and of no more than the length asked for.
///   The bytes it returns are held until the next call.
/// * No byte read where some were asked for is the end of the resource at
///   that offset.
/// * A failure of the source leaves the next [`read_at`](Self::read_at)
///   seeking afresh, so the same read is made again.
/// * Every `async fn` here is cancellation safe. Where the source stands is
///   trusted only once a seek or a read there has completed, so a
///   [`read_at`](Self::read_at) dropped part way and made again reads the
///   same bytes, none skipped. A
///   [`locate_movie_fragment_random_access`](Self::locate_movie_fragment_random_access)
///   dropped part way is made again from its start by the call that repeats
///   it.
///
/// # Examples
///
/// The loop that reads a demux FSM through one is in the
/// [crate documentation](crate).
///
/// ```
/// use futures_executor::block_on;
/// use futures_util::io::Cursor;
///
/// use isobmff_io::Source;
/// block_on(async {
///     // A file lying four bytes into its resource
///     let mut resource = Cursor::new(b"junkFILEBYTES".to_vec());
///     resource.set_position(4);
///     let mut source = Source::new(resource).await?;
///
///     // Bytes are read at file offsets, up to the length asked for
///     assert_eq!(source.read_at(4, Some(2)).await?, b"BY");
///     assert_eq!(source.read_at(0, None).await?, b"FILEBYTES");
///
///     // No byte read is the end of the resource
///     assert_eq!(source.read_at(9, None).await?, b"");
/// #   Ok::<(), std::io::Error>(())
/// })
/// # .unwrap();
/// ```
#[derive(Debug)]
pub struct Source<S> {
    source: S,
    origin: u64,
    cursor: Option<u64>,
    buffer: Vec<u8>,
}

impl<S: AsyncRead + AsyncSeek + Unpin> Source<S> {
    /// Creates one reading the file that begins where `source` stands
    ///
    /// # Errors
    ///
    /// * The source does not report where it stands.
    pub async fn new(mut source: S) -> io::Result<Self> {
        let origin = seek(&mut source, SeekFrom::Current(0)).await?;

        Ok(Self {
            source,
            origin,
            cursor: Some(0),
            buffer: vec![0; CUT_LENGTH],
        })
    }

    /// Reads bytes of the file at `offset`, up to `length` of them where it is `Some`, and returns them
    ///
    /// # Errors
    ///
    /// * `offset` lies past what a seek from where the file begins names
    ///   ([`InvalidInput`](io::ErrorKind::InvalidInput)).
    /// * The source does not seek to `offset` or read there.
    pub async fn read_at(&mut self, offset: u64, length: Option<u64>) -> io::Result<&[u8]> {
        if self.cursor != Some(offset) {
            self.cursor = None;
            let position = self
                .origin
                .checked_add(offset)
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            seek(&mut self.source, SeekFrom::Start(position)).await?;
            self.cursor = Some(offset);
        }
        let into = self
            .buffer
            .get_mut(..cut_length(length))
            .unwrap_or_default();
        let read = loop {
            match read(&mut self.source, into).await {
                Err(failure) if failure.kind() == io::ErrorKind::Interrupted => {}
                read => break read,
            }
        };
        self.cursor = read
            .as_ref()
            .ok()
            .and_then(|read| u64::try_from(*read).ok())
            .and_then(|read| offset.checked_add(read));

        Ok(self.buffer.get(..read?).unwrap_or_default())
    }

    /// Finds the offset of the `mfra` closing the file, by the `mfro` in its last 16 bytes
    ///
    /// The file ends where the source does, and its last 16 bytes are to be
    /// an `mfro`, whose `size` steps back from the end of the file to where
    /// the `mfra` begins (ISO/IEC 14496-12 §8.8.11); `None` comes back for a
    /// file shorter than that, one closing with no `mfro`, or one whose
    /// `mfro` steps back past its start. That an `mfra` stands at the offset
    /// is not read here. The next [`read_at`](Self::read_at) seeks the source
    /// to its offset.
    ///
    /// # Errors
    ///
    /// * The source does not seek from its end, or does not seek or read
    ///   where the `mfro` lies.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures_executor::block_on;
    /// use futures_util::io::Cursor;
    ///
    /// use isobmff_io::Source;
    /// # use isobmff_test_support::indexed_fragmented_file;
    /// # let bytes = indexed_fragmented_file().bytes;
    /// block_on(async {
    ///     // A file closing with an `mfra`
    ///     let mut source = Source::new(Cursor::new(bytes.clone())).await?;
    ///
    ///     // The `mfro` closing the file steps back to where the `mfra` begins
    ///     let mfra = source.locate_movie_fragment_random_access().await?.expect("the file closes with an mfro");
    ///     assert_eq!(&bytes[mfra as usize + 4..][..4], b"mfra");
    /// #   Ok::<(), std::io::Error>(())
    /// })
    /// # .unwrap();
    /// ```
    pub async fn locate_movie_fragment_random_access(&mut self) -> io::Result<Option<u64>> {
        self.cursor = None;
        let file_len = seek(&mut self.source, SeekFrom::End(0))
            .await?
            .saturating_sub(self.origin);
        let Some(closing) = ClosingMovieFragmentRandomAccessOffset::of(file_len) else {
            return Ok(None);
        };
        let position = SeekFrom::Start(closing.position(self.origin));
        seek(&mut self.source, position).await?;
        let mut mfro = [0; ClosingMovieFragmentRandomAccessOffset::LEN];
        let mut filled = 0;
        while let Some(into) = mfro.get_mut(filled..).filter(|into| !into.is_empty()) {
            match read(&mut self.source, into).await {
                Err(failure) if failure.kind() == io::ErrorKind::Interrupted => {}
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                Ok(read) => filled = filled.saturating_add(read),
                Err(failure) => return Err(failure),
            }
        }

        Ok(closing.movie_fragment_random_access_start(&mfro))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use std::io::{self, SeekFrom};

    use futures_executor::block_on;
    use futures_io::{AsyncRead, AsyncSeek};
    use futures_util::io::Cursor;
    use isobmff_boxes::MovieFragmentRandomAccessBox;
    use isobmff_test_support::written;

    use super::{CUT_LENGTH, Source};

    use crate::transfer::tests::poll_once;

    /// Source standing still once before every read and every seek
    struct Hesitant<S> {
        source: S,
        standing_still: bool,
    }

    impl<S> Hesitant<S> {
        /// Creates a source standing still before whatever it is asked for next
        const fn new(source: S) -> Self {
            Self {
                source,
                standing_still: false,
            }
        }

        /// Reports whether it stands still here, and stands ready for what comes next
        fn stands_still(&mut self, context: &Context<'_>) -> bool {
            self.standing_still = !self.standing_still;
            if self.standing_still {
                context.waker().wake_by_ref();
            }

            self.standing_still
        }
    }

    impl<S: AsyncRead + Unpin> AsyncRead for Hesitant<S> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            into: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            if self.stands_still(context) {
                return Poll::Pending;
            }

            Pin::new(&mut self.source).poll_read(context, into)
        }
    }

    impl<S: AsyncSeek + Unpin> AsyncSeek for Hesitant<S> {
        fn poll_seek(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            from: SeekFrom,
        ) -> Poll<io::Result<u64>> {
            if self.stands_still(context) {
                return Poll::Pending;
            }

            Pin::new(&mut self.source).poll_seek(context, from)
        }
    }

    /// Source that moves as soon as it is sought, and reports the seek done only when polled again
    struct Unsettled {
        source: Cursor<Vec<u8>>,
        settling: Option<SeekFrom>,
    }

    impl AsyncRead for Unsettled {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            into: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.source).poll_read(context, into)
        }
    }

    impl AsyncSeek for Unsettled {
        fn poll_seek(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            from: SeekFrom,
        ) -> Poll<io::Result<u64>> {
            if self.settling == Some(from) {
                self.settling = None;

                return Poll::Ready(Ok(self.source.position()));
            }
            if let Poll::Ready(Err(failure)) = Pin::new(&mut self.source).poll_seek(context, from) {
                return Poll::Ready(Err(failure));
            }
            self.settling = Some(from);
            context.waker().wake_by_ref();

            Poll::Pending
        }
    }

    /// The bytes `source` reads at `offset`, polled until it gives and dropped where it stood each time before
    fn read_carried_on<S: AsyncRead + AsyncSeek + Unpin>(
        source: &mut Source<S>,
        offset: u64,
        length: Option<u64>,
    ) -> Vec<u8> {
        loop {
            if let Some(read) = poll_once(source.read_at(offset, length)) {
                return read.unwrap().to_vec();
            }
        }
    }

    /// A file a cut long and then some, closing with an `mfra`
    fn file_closing_with_an_mfra() -> Vec<u8> {
        [
            vec![0x11; CUT_LENGTH],
            written(&MovieFragmentRandomAccessBox::new(vec![])),
        ]
        .concat()
    }

    #[test]
    fn a_read_on_from_where_the_last_ended_and_one_elsewhere_read_the_file_where_asked() {
        let mut source = block_on(Source::new(Cursor::new(b"FILEBYTES".to_vec()))).unwrap();

        let reads = block_on(async {
            [
                source.read_at(0, Some(4)).await.unwrap().to_vec(),
                source.read_at(4, Some(2)).await.unwrap().to_vec(),
                source.read_at(1, Some(3)).await.unwrap().to_vec(),
                source.read_at(9, None).await.unwrap().to_vec(),
            ]
        });

        assert_eq!(
            reads,
            [
                b"FILE".to_vec(),
                b"BY".to_vec(),
                b"ILE".to_vec(),
                Vec::new()
            ]
        );
    }

    #[test]
    fn the_file_begins_where_the_source_stands() {
        let mut file = Cursor::new(b"junkFILE".to_vec());
        file.set_position(4);
        let mut source = block_on(Source::new(file)).unwrap();

        let reads = block_on(async {
            [
                source.read_at(1, Some(2)).await.unwrap().to_vec(),
                source.read_at(0, None).await.unwrap().to_vec(),
            ]
        });

        assert_eq!(reads, [b"IL".to_vec(), b"FILE".to_vec()]);
    }

    #[test]
    fn an_offset_past_what_a_seek_names_is_refused() {
        let mut file = Cursor::new(b"junkFILE".to_vec());
        file.set_position(4);
        let mut source = block_on(Source::new(file)).unwrap();

        assert_eq!(
            block_on(source.read_at(u64::MAX, None))
                .map(<[u8]>::to_vec)
                .map_err(|failure| failure.kind()),
            Err(io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn reads_dropped_at_every_await_read_each_byte_once_where_asked() {
        let mut file = Hesitant::new(Cursor::new(b"junkFILEBYTES".to_vec()));
        file.source.set_position(4);
        let mut source = block_on(Source::new(file)).unwrap();

        let reads = [
            read_carried_on(&mut source, 0, Some(4)),
            read_carried_on(&mut source, 4, None),
            read_carried_on(&mut source, 1, Some(2)),
        ];

        assert_eq!(reads, [b"FILE".to_vec(), b"BYTES".to_vec(), b"IL".to_vec()]);
    }

    #[test]
    fn a_seek_dropped_after_the_source_moved_has_the_next_read_seek_afresh() {
        let mut source = block_on(Source::new(Unsettled {
            source: Cursor::new(b"FILE".to_vec()),
            settling: None,
        }))
        .unwrap();
        assert!(poll_once(source.read_at(2, None)).is_none());

        assert_eq!(block_on(source.read_at(0, None)).unwrap(), b"FILE");
    }

    #[test]
    fn a_locate_dropped_part_way_finds_the_mfra_when_made_again() {
        let mut source = block_on(Source::new(Hesitant::new(Cursor::new(
            file_closing_with_an_mfra(),
        ))))
        .unwrap();
        assert!(poll_once(source.locate_movie_fragment_random_access()).is_none());
        assert!(poll_once(source.locate_movie_fragment_random_access()).is_none());

        let located = block_on(source.locate_movie_fragment_random_access());

        assert_eq!(
            located.map_err(|failure| failure.kind()),
            Ok(Some(CUT_LENGTH as u64))
        );
    }

    #[test]
    fn a_locate_dropped_part_way_has_the_next_read_seek_to_its_offset() {
        let file = file_closing_with_an_mfra();
        let mut source = block_on(Source::new(Hesitant::new(Cursor::new(file.clone())))).unwrap();
        read_carried_on(&mut source, 0, Some(4));

        let awaits_dropped = 4;
        for _ in 0..awaits_dropped {
            assert!(poll_once(source.locate_movie_fragment_random_access()).is_none());
        }

        assert_eq!(
            read_carried_on(&mut source, 4, None),
            file.get(4..CUT_LENGTH.saturating_add(4)).unwrap()
        );
    }
}
