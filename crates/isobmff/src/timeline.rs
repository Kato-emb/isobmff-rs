//! The timelines of a movie: the durations its headers declare, as its sample tables and edit lists give them, where its edits start in the media, and how they present its samples

mod durations;
mod edit_list;
mod error;
mod presentation;

pub use durations::declare_durations;
pub use edit_list::move_media_times;
pub use error::Error;
pub use presentation::{Presentation, PresentedSample, presentation};
