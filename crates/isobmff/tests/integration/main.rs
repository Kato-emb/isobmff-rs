//! The samples of the files `isobmff` reads and writes, driven through its public API

// Why `cfg(test)` on the helpers but not on the test modules: it is what keeps
// `allow-unwrap-in-tests` reaching a helper, and each test module already
// keeps its tests in a `mod tests` of its own.
#[cfg(test)]
mod driving;
#[cfg(test)]
mod media_segment_reading;
#[cfg(test)]
mod presentations;
#[cfg(test)]
mod reading;

mod fragment_round_trip;
mod fragmented_file;
mod fragmented_samples;
mod input_passthrough;
mod media_segment_file;
mod media_segment_round_trip;
mod movie_sample_extent;
mod non_fragmented_file;
mod non_fragmented_round_trip;
mod round_trip;
mod sample_table_round_trip;
mod sequence_round_trip;
