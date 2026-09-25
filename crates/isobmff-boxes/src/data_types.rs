//! Values the fields of more than one box carry
//!
//! Where ISO/IEC 14496-12 gives a field a shape that several boxes share — the
//! `sample_flags` of a `trex`, a `tfhd` and a `trun`, the composition time
//! offset of a `trun` row and a `ctts` entry, the `duration` of a movie, track
//! and media header — a type here stands for that shape, and each type cites
//! the section that settles it.

pub(crate) mod composition_time_offset;
pub(crate) mod header_duration;
pub(crate) mod sample_flags;

pub use composition_time_offset::CompositionTimeOffset;
pub use header_duration::HeaderDuration;
pub use sample_flags::SampleFlags;
pub(crate) use sample_flags::read_sample_flags;
