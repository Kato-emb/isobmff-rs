//! Reading properties of [`BoxReader`]
//!
//! One run checks four properties of the same input:
//!
//! 1. no call panics, and no event carries a payload piece that is empty
//! 2. the chunks an input arrives in do not change the boxes it reads
//! 3. reading a whole input agrees with the [`boxes`] iterator over it, box for
//!    box and on whether the input reads at all
//! 4. every box read re-encodes to the span of input it was read from, at the
//!    offset the reader reported the box begins at

#![no_main]

use isobmff_core::{BoxHeader, boxes};
use isobmff_sequence::{BoxEvent, BoxReader, BoxReaderError};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

/// Input of one run: bytes to read, and the chunk lengths to feed them in
///
/// A corpus file is four bytes of `chunk_lengths` followed by the bytes
/// themselves, verbatim. The lengths are cycled, each one byte longer than it
/// reads, so a chunk is 1 to 256 bytes and the feeding always advances.
#[derive(Arbitrary, Debug)]
struct Input {
    // Why not put `bytes` first: only a trailing `Vec<u8>` is taken verbatim by
    // `Arbitrary`, so any other order leaves the seed files unreadable as a
    // hexdump of the input.
    chunk_lengths: [u8; 4],
    bytes: Vec<u8>,
}

/// One box as the reader reported it
#[derive(PartialEq, Debug)]
struct Framed {
    header: BoxHeader,
    file_offset: u64,
    payload: Vec<u8>,
    ended: bool,
}

/// Everything one pass of the reader over an input reported
#[derive(PartialEq, Debug)]
struct Run {
    framed: Vec<Framed>,
    failure: Option<BoxReaderError>,
}

fuzz_target!(|input: Input| {
    let Input {
        chunk_lengths,
        bytes,
    } = input;

    let whole = read([bytes.as_slice()]);
    let chunked = read(chunks(&bytes, chunk_lengths));

    assert_eq!(
        whole, chunked,
        "the chunks the input arrived in changed the boxes it read"
    );

    agrees_with_the_boxes_iterator(&bytes, &whole);
    boxes_re_encode_to_the_span_they_came_from(&bytes, &whole);
});

/// Hands `chunks` to a reader and gathers what it reports
fn read<'chunk>(chunks: impl IntoIterator<Item = &'chunk [u8]>) -> Run {
    let mut reader = BoxReader::new();
    let mut framed: Vec<Framed> = Vec::new();
    let mut failure = None;

    for chunk in chunks {
        // Why drain before reporting the failure: the events made before it are
        // still the reader's to hand over, and dropping them would leave a box
        // half gathered for the comparison against the iterator.
        let outcome = reader.handle_read(chunk);
        drain(&mut reader, &mut framed);

        if let Err(reported) = outcome {
            failure = Some(reported);
            break;
        }
    }

    if failure.is_none() {
        let outcome = reader.finish();
        drain(&mut reader, &mut framed);

        if let Err(reported) = outcome {
            failure = Some(reported);
        }
    }

    // Why not keep the box left unclosed: it spans bytes the reader never got
    // through, so comparing it against the iterator would hold a partial box up
    // against a whole one.
    framed.retain(|framed| framed.ended);

    Run { framed, failure }
}

/// Takes every event the reader has made, folding it into the boxes gathered
fn drain(reader: &mut BoxReader, framed: &mut Vec<Framed>) {
    while let Some(event) = reader.poll_event() {
        match event {
            BoxEvent::RawStart {
                header,
                file_offset,
            } => framed.push(Framed {
                header,
                file_offset,
                payload: Vec::new(),
                ended: false,
            }),
            BoxEvent::RawChunk(part) => {
                assert!(!part.is_empty(), "an empty payload event was reported");
                let open = framed.last_mut().expect("a payload before any box started");
                open.payload.extend_from_slice(&part);
            }
            BoxEvent::RawEnd => {
                framed
                    .last_mut()
                    .expect("an end before any box started")
                    .ended = true;
            }
            unknown => panic!("the reader reported an event this run cannot check: {unknown:?}"),
        }
    }
}

/// Splits `bytes` at the cycled `lengths`, each one byte longer than it reads
fn chunks(bytes: &[u8], lengths: [u8; 4]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;

    lengths.into_iter().cycle().map_while(move |length| {
        if rest.is_empty() {
            return None;
        }
        let (chunk, remainder) = rest.split_at((usize::from(length) + 1).min(rest.len()));
        rest = remainder;
        Some(chunk)
    })
}

fn agrees_with_the_boxes_iterator(bytes: &[u8], run: &Run) {
    let mut iterated = Vec::new();
    let mut iterator_failed = false;

    for framed in boxes(bytes) {
        match framed {
            Ok(framed) => iterated.push(framed),
            Err(_) => iterator_failed = true,
        }
    }

    assert_eq!(
        run.framed
            .iter()
            .map(|framed| (framed.header, framed.payload.as_slice()))
            .collect::<Vec<_>>(),
        iterated
            .iter()
            .map(|framed| (framed.header(), framed.payload()))
            .collect::<Vec<_>>(),
        "the reader and the iterator disagree on the boxes the input holds"
    );

    assert_eq!(
        run.failure.is_some(),
        iterator_failed,
        "the reader and the iterator disagree on whether the input is well formed"
    );
}

fn boxes_re_encode_to_the_span_they_came_from(bytes: &[u8], run: &Run) {
    let mut offset = 0;

    for framed in &run.framed {
        let mut buffer = [0; BoxHeader::MAX_ENCODED_LEN];
        let header = framed.header.encode(&mut buffer);

        assert_eq!(
            framed.file_offset,
            offset as u64,
            "a box was reported at an offset it does not begin at"
        );

        assert_eq!(
            bytes.get(offset..offset + header.len()),
            Some(header),
            "a header does not re-encode to the span it was read from"
        );
        offset += header.len();

        assert_eq!(
            bytes.get(offset..offset + framed.payload.len()),
            Some(framed.payload.as_slice()),
            "a payload does not match the span it was read from"
        );
        offset += framed.payload.len();
    }
}
