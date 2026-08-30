//! A file read into events is the file the writer lays back down from them

#[cfg(test)]
mod tests {
    use isobmff_sequence::BoxEvent;

    use isobmff_test_support::{
        bytes_of, events_of, file_running_to_its_end, fragmented_file, segment_file,
    };

    /// Every synthetic file the round trip is fixed for
    ///
    /// Between them they carry a box under each of the two box types, a container
    /// carried as it lies, and a box running to the end of the file
    fn every_file() -> Vec<Vec<u8>> {
        vec![fragmented_file(), segment_file(), file_running_to_its_end()]
    }

    /// The events the reader reports for `file`, as the writer takes them
    fn events_to_write(file: &[u8], cut_length: usize) -> Vec<BoxEvent> {
        events_of(file, cut_length)
            .unwrap()
            .into_iter()
            .map(|(_extent, event)| event)
            .collect()
    }

    #[test]
    fn a_file_is_written_back_from_its_events_however_the_input_was_cut() {
        for file in every_file() {
            for cut_length in 1..=file.len() {
                assert_eq!(
                    bytes_of(events_to_write(&file, cut_length)).unwrap(),
                    file,
                    "cut every {cut_length} bytes"
                );
            }
        }
    }
}
