# isobmff-rs

[![CI](https://github.com/Kato-emb/isobmff-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Kato-emb/isobmff-rs/actions/workflows/ci.yml)

Sans-IO reading and writing of the ISO base media file format
(ISO/IEC 14496-12) in Rust.

> **Status:** under construction. The implementation is being migrated from an
> earlier codebase, and nothing is published to crates.io yet.

## Crates

| Crate | Holds |
|---|---|
| `isobmff-core` | the vocabulary a box is written in: the header that frames a box, the values its fields carry, and the traits a box implements to be read and written |
| `isobmff-boxes` | the boxes of ISO/IEC 14496-12, each read into a value that owns its bytes and written back |
| `isobmff` | under construction: reading a file into its samples, and writing samples into a file |

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
