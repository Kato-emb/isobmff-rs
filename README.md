# isobmff-rs

[![CI](https://github.com/Kato-emb/isobmff-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Kato-emb/isobmff-rs/actions/workflows/ci.yml)

Sans-IO reading and writing of the ISO base media file format
(ISO/IEC 14496-12) in Rust.

> **Status:** under construction. The implementation is being migrated from an
> earlier codebase, and nothing is published to crates.io yet.

## Reading a file

A `Demuxer` does no I/O: it names the bytes it wants next, reports what the
bytes it is given hold, and returns each sample once its bytes have arrived.
The caller reads the file however it can.

```rust
use isobmff::Demuxer;

let file = std::fs::read("movie.mp4")?;
let mut demuxer = Demuxer::new();
while let Some(wanted) = demuxer.wanted_input() {
    let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
    let event = match input {
        [] => demuxer.finish()?,
        input => demuxer.handle_input(wanted.offset(), input)?,
    };
    if let Some(event) = event {
        println!("{event:?}");
    }
    while let Some(sample) = demuxer.poll_sample() {
        println!("track {}: {} bytes", sample.properties().track_id, sample.data().len());
    }
}
```

The crate documentation of `isobmff` shows a caller that cannot seek, and one
that resumes at a fragment the `mfra` lists.

## Minimum supported Rust version

`rust-version` in [Cargo.toml](Cargo.toml); [CONTRIBUTING.md](CONTRIBUTING.md#minimum-supported-rust-version)
states how it is raised.

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
