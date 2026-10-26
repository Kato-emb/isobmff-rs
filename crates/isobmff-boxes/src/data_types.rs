//! Values several boxes carry in a field of a shape of their own

pub(crate) mod compressor_name;
pub(crate) mod language_code;
pub(crate) mod matrix;
pub(crate) mod sample_flags;

pub use compressor_name::CompressorName;
pub use language_code::LanguageCode;
pub use matrix::Matrix;
pub use sample_flags::SampleFlags;
