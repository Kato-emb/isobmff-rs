//! Boxes a file is framed by come back out of the reader as the values they hold

#[cfg(test)]
mod tests {
    use isobmff_core::{BoxEncode as _, BoxType};
    use isobmff_sequence::{BoxEvent, BoxReader, Error};

    use isobmff_test_support::{
        MEDIA_DATA, events_of, file_type, fragmented_file, media_data_header, movie_fragment,
        payloads_fused, polled, unfragmented_movie,
    };

    #[test]
    fn the_boxes_a_file_is_framed_by_are_values_and_its_media_data_is_passed_on() {
        let movie = unfragmented_movie();
        let movie_fragment = movie_fragment();
        let movie_at = file_type().encoded_len();
        let fragment_at = movie_at.saturating_add(movie.encoded_len());
        let media_data_at = fragment_at.saturating_add(movie_fragment.encoded_len());
        let media_data_header = media_data_header();
        let media_data_payload_at =
            media_data_at.saturating_add(u64::try_from(media_data_header.encoded_len()).unwrap());
        let media_data_end =
            media_data_payload_at.saturating_add(u64::try_from(MEDIA_DATA.len()).unwrap());
        let file = fragmented_file();

        assert_eq!(
            events_of(&file, file.len()).unwrap(),
            vec![
                (0..movie_at, BoxEvent::FileType(file_type())),
                (movie_at..fragment_at, BoxEvent::Movie(movie)),
                (
                    fragment_at..media_data_at,
                    BoxEvent::MovieFragment(movie_fragment)
                ),
                (
                    media_data_at..media_data_payload_at,
                    BoxEvent::RawStart(media_data_header)
                ),
                (
                    media_data_payload_at..media_data_end,
                    BoxEvent::RawPayload(Vec::from(MEDIA_DATA))
                ),
                (media_data_end..media_data_end, BoxEvent::RawEnd),
            ]
        );
    }

    #[test]
    fn the_events_reported_do_not_turn_on_where_the_fragmented_file_was_cut() {
        let file = fragmented_file();
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
    fn a_value_declaring_more_than_the_limit_stops_the_reader_where_it_stands() {
        let declared = unfragmented_movie().payload_len();
        let limit = declared.saturating_sub(1);
        let file = fragmented_file();
        let mut reader = BoxReader::with_payload_limit(limit);

        let failure = Error::payload_limit_exceeded(BoxType::compact(*b"moov"), declared, limit);

        assert_eq!(reader.handle_input(&file), Err(failure));
        assert_eq!(
            polled(&mut reader),
            Some((
                0..file_type().encoded_len(),
                BoxEvent::FileType(file_type())
            ))
        );
        assert_eq!(reader.poll_event(), None);
        assert_eq!(reader.handle_input(&file), Err(failure));
    }
}
