//! Sans-IO demultiplexer and multiplexer for the ISO base media file format
//!
//! [`PayloadAccumulator`] gathers the payload of one box out of the chunks it
//! arrives in, under a limit its caller sets.
//!
//! # `no_std`
//!
//! The crate is `no_std` but needs `alloc`: a payload it gathers is owned.

#![no_std]

extern crate alloc;

mod payload_accumulator;

pub use payload_accumulator::{PayloadAccumulator, PayloadAccumulatorError};
