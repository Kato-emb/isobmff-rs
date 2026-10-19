//! [`FragmentedMuxer`] and [`NonFragmentedMuxer`], a movie file written fragmented or not, ISO/IEC 14496-12 §8.2.1, §8.7 and Annex A.8

mod fragmented;
mod non_fragmented;

pub use fragmented::FragmentedMuxer;
pub use non_fragmented::NonFragmentedMuxer;
