//! Values a box of ISO/IEC 14496-12 §4.2 is read into and written from
//!
//! §4.2 frames a box but says nothing of its contents: the type that names a
//! box is settled by its Definition subclause, and what its bytes mean by its
//! Syntax and Semantics subclauses. The traits here are where a box states
//! both — the type it is known by, and the payload read into a value and
//! written back from one — over a reader and a writer that take a payload one
//! field at a time. Writing the whole box follows from the two, and asks
//! nothing further of the box.

pub(crate) mod box_decode;
pub(crate) mod box_definition;
pub(crate) mod box_encode;
pub(crate) mod box_write;
pub(crate) mod field;

pub use box_decode::BoxDecode;
pub use box_definition::BoxDefinition;
pub use box_encode::BoxEncode;
pub use box_write::BoxWrite;
pub use field::{FieldReader, FieldWidth, FieldWriter};
