//! Framing properties of [`boxes`] and [`RawBox::split_first`]
//!
//! One run checks four properties of the same input:
//!
//! 1. no split panics
//! 2. the iteration stops, taking at most one step per eight bytes
//! 3. every frame re-encodes to the span of input it was framed from
//! 4. a truncation error asks for strictly more than the input offered, and
//!    retrying at the length it asks for makes progress

#![no_main]

use isobmff_core::{BoxHeader, Error, ErrorKind, RawBox, boxes};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

/// Input of one run: bytes to frame, and how far to cut them for the retry check
///
/// A corpus file is two bytes of `prefix_length` followed by the bytes
/// themselves, verbatim.
#[derive(Arbitrary, Debug)]
struct Input<'bytes> {
    // Why not put `bytes` first, and why a slice: only the last field is handed
    // what is left, and only `&[u8]` takes it verbatim — a `Vec<u8>` reads a
    // byte of its own before each element, so a seed stops at its first even
    // byte.
    prefix_length: u16,
    bytes: &'bytes [u8],
}

fuzz_target!(|input: Input<'_>| {
    let Input {
        prefix_length,
        bytes,
    } = input;

    frames_re_encode_to_the_span_they_came_from(bytes);
    truncation_asks_for_more_than_the_input_offered(bytes, prefix_length);
});

fn frames_re_encode_to_the_span_they_came_from(bytes: &[u8]) {
    let mut offset = 0;
    let mut steps = 0usize;

    for framed in boxes(bytes) {
        steps += 1;

        let Ok(framed) = framed else { continue };

        let mut buffer = [0; BoxHeader::MAX_ENCODED_LEN];
        let header = framed.header().encode(&mut buffer);
        let mut re_encoded = Vec::with_capacity(header.len() + framed.payload().len());
        re_encoded.extend_from_slice(header);
        re_encoded.extend_from_slice(framed.payload());

        assert_eq!(
            bytes.get(offset..offset + re_encoded.len()),
            Some(re_encoded.as_slice()),
            "frame does not re-encode to the span it was framed from"
        );
        offset += re_encoded.len();
    }

    assert!(
        steps <= bytes.len() / 8 + 1,
        "iterating {} bytes took {steps} steps",
        bytes.len()
    );
}

fn truncation_asks_for_more_than_the_input_offered(bytes: &[u8], prefix_length: u16) {
    let cut = usize::from(prefix_length) % (bytes.len() + 1);
    let Some(needed) = bytes_needed(RawBox::split_first(&bytes[..cut])) else {
        return;
    };

    assert!(
        needed > cut as u64,
        "a truncation over {cut} bytes asks for only {needed}"
    );

    let Some(retried) = usize::try_from(needed)
        .ok()
        .and_then(|end| bytes.get(..end))
    else {
        return;
    };

    if let Some(needed_again) = bytes_needed(RawBox::split_first(retried)) {
        assert!(
            needed_again > needed,
            "retrying at {needed} bytes asks for {needed_again} again"
        );
    }
}

/// Length the input must grow to, for an error that says it was cut short
fn bytes_needed(result: Result<(RawBox<'_>, &[u8]), Error>) -> Option<u64> {
    let error = result.err()?;

    match error.kind() {
        ErrorKind::TruncatedHeader | ErrorKind::TruncatedBox => error.needed_bytes(),
        _ => None,
    }
}
