# isobmff-rs

[![CI](https://github.com/Kato-emb/isobmff-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Kato-emb/isobmff-rs/actions/workflows/ci.yml)

Sans-IO reading and writing of the ISO base media file format
(ISO/IEC 14496-12) in Rust.

> **Status:** under construction. The implementation is being migrated from an
> earlier codebase, and nothing is published to crates.io yet.

## Crates

| Crate | Holds |
|---|---|
| `isobmff` | the layers that follow the structure of a file, each usable alone in either direction: `framing` (the boxes laid end to end, §4.2.2), `file_level` (the kind and value of each box at file level, A.3), `structure_data` (the `moov` and `moof`s and the location of each sample, §8.5, §8.8), `media_data` (the bytes of each sample, §8.1.1) and `timeline` (where an edit list presents the samples, §8.6.6), with the boxes and the samples they read and write |
| `isobmff-mux` | made of those layers: a `Demuxer` that reads a file into its samples, a `Muxer` that writes samples into a movie file or a segment, and a `FastStart` that lays a movie file out again with its `moov` first |

A caller that reads or writes files whole uses `isobmff-mux`. A reader or a
writer of its own is made of the layers of `isobmff`, as `isobmff-mux` is.

## Reading a file with the layers

No layer does I/O: a reader names the bytes it wants next, and the caller
reads the file however it can. This one lists the boxes at file level,
gathering the structure-data into its values and skipping the rest.

```rust
use isobmff::file_level::{Kind, TopLevelBox};
use isobmff::framing::{BoxEvent, BoxReader};

let file = std::fs::read("movie.mp4")?;
let mut reader = BoxReader::new();
while let Some(wanted) = reader.wanted_input() {
    let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
    if input.is_empty() {
        break;
    }
    while let Some(event) = reader.handle_input(wanted.offset(), input)? {
        match event {
            BoxEvent::Header(header) => match Kind::of(header.box_type()) {
                Kind::StructureData => reader.gather(16 * 1024 * 1024)?,
                Kind::MediaData | Kind::IdentifiedMediaData | Kind::Other => reader.skip()?,
            },
            BoxEvent::End => {
                if let Some(raw_box) = reader.gathered_box() {
                    println!("{:?}", TopLevelBox::read(raw_box)?);
                }
            }
            BoxEvent::Payload(_) => {}
        }
    }
}
```

## Reading a file with the demuxer

A `Demuxer` does no I/O either: it names the bytes it wants next, reports what
the bytes it is given hold, and returns each sample once its bytes have
arrived.

```rust
use isobmff_mux::Demuxer;

let file = std::fs::read("movie.mp4")?;
let mut demuxer = Demuxer::new();
while let Some(wanted) = demuxer.wanted_input() {
    let input = file.get(usize::try_from(wanted.offset())?..).unwrap_or_default();
    let event = match input {
        [] => demuxer.finish(),
        input => demuxer.handle_input(wanted.offset(), input),
    };
    while let Some(sample) = demuxer.poll_sample() {
        println!("track {}: {} bytes", sample.properties().track_id, sample.data().len());
    }
    if let Some(event) = event? {
        println!("{event:?}");
    }
}
```

The crate documentation of `isobmff-mux` shows a caller that cannot seek, and
one that resumes at a fragment the `mfra` lists.

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
