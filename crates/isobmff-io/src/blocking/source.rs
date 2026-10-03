//! [`Source`], a file read at the offsets a demux FSM names, off a source that seeks

use alloc::vec;
use alloc::vec::Vec;
use std::io::{self, Read, Seek, SeekFrom};

use crate::transfer::{CUT_LENGTH, ClosingMovieFragmentRandomAccessOffset, cut_length};

/// A file read at an offset at a time, off a source that is `Read + Seek`
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
///
/// # Examples
///
/// The loop that reads a demux FSM through one is in the
/// [crate documentation](crate).
///
/// ```
/// use std::io::Cursor;
///
/// use isobmff_io::blocking::Source;
/// // A file lying four bytes into its resource
/// let mut resource = Cursor::new(b"junkFILEBYTES".to_vec());
/// resource.set_position(4);
/// let mut source = Source::new(resource)?;
///
/// // Bytes are read at file offsets, up to the length asked for
/// assert_eq!(source.read_at(4, Some(2))?, b"BY");
/// assert_eq!(source.read_at(0, None)?, b"FILEBYTES");
///
/// // No byte read is the end of the resource
/// assert_eq!(source.read_at(9, None)?, b"");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct Source<S> {
    source: S,
    origin: u64,
    cursor: Option<u64>,
    buffer: Vec<u8>,
}

impl<S: Read + Seek> Source<S> {
    /// Creates one reading the file that begins where `source` stands
    ///
    /// # Errors
    ///
    /// * The source does not report where it stands.
    pub fn new(mut source: S) -> io::Result<Self> {
        let origin = source.stream_position()?;

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
    pub fn read_at(&mut self, offset: u64, length: Option<u64>) -> io::Result<&[u8]> {
        if self.cursor.take() != Some(offset) {
            let position = self
                .origin
                .checked_add(offset)
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            self.source.seek(SeekFrom::Start(position))?;
        }
        let into = self
            .buffer
            .get_mut(..cut_length(length))
            .unwrap_or_default();
        let read = loop {
            match self.source.read(into) {
                Err(failure) if failure.kind() == io::ErrorKind::Interrupted => {}
                read => break read?,
            }
        };
        self.cursor = u64::try_from(read)
            .ok()
            .and_then(|read| offset.checked_add(read));

        Ok(self.buffer.get(..read).unwrap_or_default())
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
    /// use std::io::Cursor;
    ///
    /// use isobmff_io::blocking::Source;
    /// # use isobmff_test_support::indexed_fragmented_file;
    /// # let bytes = indexed_fragmented_file().bytes;
    /// // A file closing with an `mfra`
    /// let mut source = Source::new(Cursor::new(bytes.clone()))?;
    ///
    /// // The `mfro` closing the file steps back to where the `mfra` begins
    /// let mfra = source.locate_movie_fragment_random_access()?.expect("the file closes with an mfro");
    /// assert_eq!(&bytes[mfra as usize + 4..][..4], b"mfra");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn locate_movie_fragment_random_access(&mut self) -> io::Result<Option<u64>> {
        self.cursor = None;
        let file_len = self
            .source
            .seek(SeekFrom::End(0))?
            .saturating_sub(self.origin);
        let Some(closing) = ClosingMovieFragmentRandomAccessOffset::of(file_len) else {
            return Ok(None);
        };
        self.source
            .seek(SeekFrom::Start(closing.position(self.origin)))?;
        let mut mfro = [0; ClosingMovieFragmentRandomAccessOffset::LEN];
        self.source.read_exact(&mut mfro)?;

        Ok(closing.movie_fragment_random_access_start(&mfro))
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use std::io::{self, Read, Seek, SeekFrom};

    use isobmff_boxes::MovieFragmentRandomAccessBox;
    use isobmff_core::BoxDecode;
    use isobmff_test_support::indexed_fragmented_file;

    use super::{CUT_LENGTH, Source};

    /// What a source was asked to do
    #[derive(PartialEq, Debug)]
    enum Call {
        Seek(SeekFrom),
        Read(usize),
    }

    /// Source recording every seek and read made of it, failing the first read when asked to
    struct Recorded {
        file: io::Cursor<Vec<u8>>,
        calls: Vec<Call>,
        failing: bool,
    }

    impl Recorded {
        fn new(file: Vec<u8>) -> Self {
            Self {
                file: io::Cursor::new(file),
                calls: Vec::new(),
                failing: false,
            }
        }
    }

    impl Read for Recorded {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            self.calls.push(Call::Read(into.len()));
            if self.failing {
                self.failing = false;

                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }

            self.file.read(into)
        }
    }

    impl Seek for Recorded {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if from != SeekFrom::Current(0) {
                self.calls.push(Call::Seek(from));
            }

            self.file.seek(from)
        }
    }

    #[test]
    fn a_read_on_from_where_the_last_ended_is_made_with_no_seek_and_one_elsewhere_with_one() {
        let mut source = Source::new(Recorded::new(b"FILEBYTES".to_vec())).unwrap();

        let reads = [
            source.read_at(0, Some(4)).unwrap().to_vec(),
            source.read_at(4, Some(2)).unwrap().to_vec(),
            source.read_at(1, Some(3)).unwrap().to_vec(),
        ];

        assert_eq!(reads, [b"FILE".to_vec(), b"BY".to_vec(), b"ILE".to_vec()]);
        assert_eq!(
            source.source.calls,
            [
                Call::Read(4),
                Call::Read(2),
                Call::Seek(SeekFrom::Start(1)),
                Call::Read(3),
            ]
        );
    }

    #[test]
    fn a_read_of_no_length_or_one_past_the_buffer_takes_a_buffer_at_most() {
        let mut source =
            Source::new(Recorded::new(vec![0x11; CUT_LENGTH.saturating_add(8)])).unwrap();

        source.read_at(0, None).unwrap();
        source.read_at(0, Some(u64::MAX)).unwrap();

        assert_eq!(
            source.source.calls,
            [
                Call::Read(CUT_LENGTH),
                Call::Seek(SeekFrom::Start(0)),
                Call::Read(CUT_LENGTH),
            ]
        );
    }

    #[test]
    fn a_read_at_the_end_of_the_resource_returns_no_byte() {
        let mut source = Source::new(io::Cursor::new(b"FILE".to_vec())).unwrap();

        assert_eq!(source.read_at(4, None).unwrap(), b"");
    }

    #[test]
    fn the_file_begins_where_the_source_stands() {
        let mut file = io::Cursor::new(b"junkFILE".to_vec());
        file.set_position(4);
        let mut source = Source::new(file).unwrap();

        let reads = [
            source.read_at(1, Some(2)).unwrap().to_vec(),
            source.read_at(0, None).unwrap().to_vec(),
        ];

        assert_eq!(reads, [b"IL".to_vec(), b"FILE".to_vec()]);
    }

    #[test]
    fn a_source_failing_a_read_has_the_same_read_seek_afresh() {
        let mut file = Recorded::new(b"FILE".to_vec());
        file.failing = true;
        let mut source = Source::new(file).unwrap();

        assert_eq!(
            source.read_at(0, None).map_err(|failure| failure.kind()),
            Err(io::ErrorKind::TimedOut)
        );
        assert_eq!(source.read_at(0, None).unwrap(), b"FILE");
        assert_eq!(
            source.source.calls,
            [
                Call::Read(CUT_LENGTH),
                Call::Seek(SeekFrom::Start(0)),
                Call::Read(CUT_LENGTH),
            ]
        );
    }

    #[test]
    fn an_offset_past_what_a_seek_names_is_refused() {
        let mut file = io::Cursor::new(b"junkFILE".to_vec());
        file.set_position(4);
        let mut source = Source::new(file).unwrap();

        assert_eq!(
            source
                .read_at(u64::MAX, None)
                .map_err(|failure| failure.kind()),
            Err(io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn a_located_mfra_is_the_box_closing_the_file() {
        let file = indexed_fragmented_file();
        let mut source = Source::new(io::Cursor::new(file.bytes.clone())).unwrap();

        let located = source
            .locate_movie_fragment_random_access()
            .unwrap()
            .unwrap();

        let closing = file.bytes.get(usize::try_from(located).unwrap()..).unwrap();
        let (_mfra, rest) = MovieFragmentRandomAccessBox::decode(closing).unwrap();
        assert_eq!(rest, []);
    }

    #[test]
    fn a_locate_has_the_next_read_seek_to_its_offset() {
        let file = indexed_fragmented_file();
        let mut source = Source::new(io::Cursor::new(file.bytes.clone())).unwrap();
        source.read_at(0, Some(4)).unwrap();

        source.locate_movie_fragment_random_access().unwrap();

        assert_eq!(
            source.read_at(4, Some(4)).unwrap(),
            file.bytes.get(4..8).unwrap()
        );
    }
}
