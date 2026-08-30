//! Boxes passed on as they lie come back out of the reader as they went in, however the file was cut

#[cfg(test)]
mod tests {
    use core::ops::Range;

    use isobmff_core::{BoxHeader, boxes};
    use isobmff_sequence::BoxEvent;

    use isobmff_test_support::{events_of, file_running_to_its_end, payloads_fused};

    /// Each box the reader reported, as its header and the payload it passed on
    fn boxes_reported(events: &[(Range<u64>, BoxEvent)]) -> Vec<(BoxHeader, Vec<u8>)> {
        let mut reported: Vec<(BoxHeader, Vec<u8>)> = Vec::new();

        for (_extent, event) in events {
            if let BoxEvent::Header(header) = *event {
                reported.push((header, Vec::new()));
            } else if let BoxEvent::Payload(ref bytes) = *event {
                if let Some((_header, payload)) = reported.last_mut() {
                    payload.extend_from_slice(bytes);
                }
            }
        }

        reported
    }

    /// Where each box the reader reported said it begins
    fn offsets_reported(events: &[(Range<u64>, BoxEvent)]) -> Vec<u64> {
        let mut offsets = Vec::new();

        for (extent, event) in events {
            if let BoxEvent::Header(_header) = *event {
                offsets.push(extent.start);
            }
        }

        offsets
    }

    #[test]
    fn the_events_reported_do_not_turn_on_where_the_file_was_cut() {
        let file = file_running_to_its_end();
        let whole = payloads_fused(events_of(&file, file.len()).unwrap());

        for cut_length in 1..=file.len() {
            assert_eq!(
                payloads_fused(events_of(&file, cut_length).unwrap()),
                whole,
                "cut every {cut_length} bytes"
            );
        }
    }

    #[test]
    fn the_boxes_reported_are_the_ones_the_boxes_iterator_splits_out() {
        let file = file_running_to_its_end();
        let split = boxes(&file)
            .map(|framed| {
                let framed = framed.unwrap();

                (framed.header(), Vec::from(framed.payload()))
            })
            .collect::<Vec<_>>();

        for cut_length in 1..=file.len() {
            assert_eq!(
                boxes_reported(&events_of(&file, cut_length).unwrap()),
                split,
                "cut every {cut_length} bytes"
            );
        }
    }

    #[test]
    fn the_extent_of_a_box_begins_where_that_box_begins_in_the_file() {
        let file = file_running_to_its_end();
        let mut walked = 0_u64;
        let beginnings = boxes(&file)
            .map(|framed| {
                let framed = framed.unwrap();
                let beginning = walked;
                let header_length = u64::try_from(framed.header().encoded_len()).unwrap();
                let payload_length = u64::try_from(framed.payload().len()).unwrap();

                walked = walked
                    .saturating_add(header_length)
                    .saturating_add(payload_length);

                beginning
            })
            .collect::<Vec<_>>();

        for cut_length in 1..=file.len() {
            assert_eq!(
                offsets_reported(&events_of(&file, cut_length).unwrap()),
                beginnings,
                "cut every {cut_length} bytes"
            );
        }
    }
}
