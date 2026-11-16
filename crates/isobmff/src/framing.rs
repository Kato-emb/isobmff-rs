//! The boxes laid end to end in a file, read from bytes that arrive in pieces
//!
//! [`BoxReader`] cuts each box at file level off the bytes of a file: it
//! gathers the payload of a box whose type [`FramerConfig`] names, and skips
//! the payload of any other without requesting it.
//!
//! Writing a box at file level takes no state: its header is
//! [`BoxHeader`](crate::BoxHeader), its payload
//! [`BoxEncode`](crate::BoxEncode), and the space reserved for a box a
//! [`FreeSpaceBox`](crate::boxes::FreeSpaceBox).

mod buffer;
mod config;
mod error;
mod event;
mod reader;

pub use config::FramerConfig;
pub use error::Error;
pub use event::Event;
pub use reader::BoxReader;
