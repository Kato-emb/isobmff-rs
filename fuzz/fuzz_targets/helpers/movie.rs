//! Building the movie a fragmented presentation continues
//!
//! Shared across the fuzz targets with `#[path = "helpers/movie.rs"] mod movie;`.
//! A file under `fuzz_targets/` is a target only where the `[[bin]]` table names
//! it, so this module is not one.

use isobmff::{MovieBox, MovieExtendsBox, MovieHeaderBox, Mp4EpochSeconds, TrackExtendsBox};
use isobmff_test_support::track;

/// Ticks a second the movie is timed in
const TIMESCALE: u32 = 1_000;

/// The id the track at `position` among the tracks of a movie takes
///
/// The ids count from one: §8.3.2.3 has zero name no track.
pub fn track_id_of(position: usize) -> u32 {
    u32::try_from(position).map_or(u32::MAX, |position| position.saturating_add(1))
}

/// Movie of the tracks whose defaults `trex` states, continued in fragments
///
/// A track takes the id its `trex` names, which is what §8.8.3 asks of a
/// fragmented movie. Reports `None` where the boxes state no track at all.
pub fn movie_of(trex: Vec<TrackExtendsBox>) -> Option<MovieBox> {
    let epoch = Mp4EpochSeconds::from_seconds(0);
    let next_track_id = track_id_of(trex.len());
    let trak = trex
        .iter()
        .map(|trex| track(trex.track_id()))
        .collect::<Vec<_>>();

    MovieBox::new(
        MovieHeaderBox::new(epoch, epoch, TIMESCALE, 0, next_track_id),
        trak,
        MovieExtendsBox::new(trex),
    )
}
