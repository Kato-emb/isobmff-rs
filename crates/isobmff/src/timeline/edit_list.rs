//! [`move_media_times`], which moves the edits of an edit list where the media of its track now starts

use isobmff_boxes::EditListBox;

use super::error::Error;

/// Moves the edits of `elst` that are not empty `earlier` time units back in the media, as they start where they did once the media starts `earlier` units sooner, and lasts until `media_end`
///
/// The `media_time` of an edit is the starting time within the media of
/// the edit, in composition time (ISO/IEC 14496-12 §8.6.6.3). Where the
/// samples of a track are declared to start `earlier` units sooner, as a
/// movie file starts the decoding times of a track at 0 (§8.6.1.2) where
/// its source did not, each edit that is not empty starts `earlier` units
/// sooner too. An empty edit, of `media_time` −1, is left as it is, and so
/// is the version of the `elst`.
///
/// # Errors
/// * [`Error::EditOutsideMedia`]: an edit that is not empty would start
///   before the media, or at or past `media_end`, where no sample is
///   composed: it points at media the file does not have. `elst` is left as
///   it was.
///
/// ```
/// use isobmff::boxes::{EditListBox, EditListEntry};
/// use isobmff::timeline::move_media_times;
///
/// // An empty edit, then an edit from 1,500 in media that starts at 1,000
/// let mut elst = EditListBox::default();
/// elst.entries = [(500, -1), (0, 1_500)]
///     .into_iter()
///     .map(|(edit_duration, media_time)| {
///         let mut entry = EditListEntry::default();
///         (entry.edit_duration, entry.media_time) = (edit_duration, media_time);
///         entry
///     })
///     .collect();
///
/// // The media now starts at 0 and lasts until 3,000
/// move_media_times(&mut elst, 1_000, 3_000)?;
/// assert_eq!(
///     elst.entries.iter().map(|entry| entry.media_time).collect::<Vec<_>>(),
///     [-1, 500]
/// );
/// # Ok::<(), isobmff::timeline::Error>(())
/// ```
pub fn move_media_times(elst: &mut EditListBox, earlier: u64, media_end: u64) -> Result<(), Error> {
    let moved = elst
        .entries
        .iter()
        .map(|entry| {
            if entry.media_time < 0 {
                return Ok(entry.media_time);
            }
            let moved = i128::from(entry.media_time).saturating_sub(i128::from(earlier));
            if moved < 0 || moved >= i128::from(media_end) {
                return Err(Error::EditOutsideMedia {
                    media_time: entry.media_time,
                    media_start: earlier,
                    media_end: earlier.saturating_add(media_end),
                });
            }
            // Why not refuse what does not fit: the value moved lies between
            // 0 and the `media_time` it was moved from, which an `i64` holds.
            Ok(i64::try_from(moved).unwrap_or(entry.media_time))
        })
        .collect::<Result<alloc::vec::Vec<_>, _>>()?;
    for (entry, media_time) in elst.entries.iter_mut().zip(moved) {
        entry.media_time = media_time;
    }
    Ok(())
}
