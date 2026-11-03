//! The read of a file that ends with a box whose size runs to its end, by the loop of a caller that seeks as the crate documentation writes it
//!
//! The demuxer wants no input past such a box, which ends only with the
//! file: the caller finishes the file once the demuxer wants no more, and the
//! box ends there.

#[cfg(test)]
mod tests {
    use isobmff::{DemuxEvent, Demuxer};

    /// `ftyp`, the `moov` of track 1 whose data reference names the `imda` of the sequence number of the `moof`, that `moof`, and the `imda` of its two samples, whose size runs to the end of the file
    const IMDA_TO_END_OF_FILE: &[u8] =
        include_bytes!("../../../fuzz/seeds/imda-to-end-of-file.mp4");

    #[test]
    fn an_imda_that_runs_to_the_end_of_the_file_is_reported_with_its_samples_once_the_file_is_finished()
     {
        let file = IMDA_TO_END_OF_FILE;
        let (mut demuxer, mut samples, mut identified) = (Demuxer::new(), Vec::new(), Vec::new());
        loop {
            let event = match demuxer.wanted_input() {
                Some(wanted) => {
                    match file
                        .get(usize::try_from(wanted.offset()).unwrap()..)
                        .unwrap_or_default()
                    {
                        [] => demuxer.finish(),
                        input => demuxer.handle_input(wanted.offset(), input),
                    }
                }
                None => match demuxer.finish() {
                    Ok(None) => break,
                    finished => finished,
                },
            };
            samples.extend(std::iter::from_fn(|| demuxer.poll_sample()));
            if let Some(DemuxEvent::IdentifiedMediaData {
                identifier,
                samples,
                ..
            }) = event.unwrap()
            {
                identified.push((identifier, samples.len()));
            }
        }

        assert_eq!(
            (
                identified,
                samples
                    .iter()
                    .map(|sample| sample.data().len())
                    .collect::<Vec<_>>()
            ),
            (vec![(1, 2)], vec![2, 2])
        );
    }
}
