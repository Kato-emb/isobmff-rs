//! The boxes laid end to end in a file, read from bytes that arrive in pieces
//!
//! [`BoxReader`] cuts each box at file level off the bytes of a file: it
//! gathers the payload of a box whose type [`FramerConfig`] names, and skips
//! the payload of any other without requesting it.

mod buffer;
mod config;
mod event;
mod reader;

pub use config::FramerConfig;
pub use event::{Error, Event};
pub use reader::BoxReader;
