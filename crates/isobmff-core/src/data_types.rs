//! Data types and fields of ISO/IEC 14496-12 §6.2.2
//!
//! The section names the types a box payload holds where a plain integer says
//! too little: the fixed-point notation the spec writes rates and sizes in,
//! the transformation matrix, and the epoch its times are counted from.
//!
//! What a box does with them belongs to the box. These types convert between
//! their raw form and the form a caller states values in, and do nothing else:
//! no arithmetic on fixed-point numbers, and no calendar.

mod fixed_point;
mod matrix;
mod quick_time_date_time;

pub use fixed_point::{I8F8, I16F16, U16F16};
pub use matrix::Matrix;
pub use quick_time_date_time::QuickTimeDateTime;
