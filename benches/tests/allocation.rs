//! What a reader of `isobmff` allocates, counted by the global allocator

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use isobmff::framing::{BoxReader, Event, FramerConfig};
use isobmff::{Demuxer, DemuxerConfig};

thread_local! {
    // Why count per thread: the test harness runs tests on threads of their
    // own, whose allocations would land in a shared count.
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
    /// Bytes the current thread holds allocated, and the most it has held
    static HELD: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// The system allocator, counting the bytes each thread asks of it
struct Counting;

fn count(bytes: usize) {
    let _ = ALLOCATED.try_with(|allocated| allocated.set(allocated.get().saturating_add(bytes)));
}

fn hold(more: usize, less: usize) {
    let _ = HELD.try_with(|held| {
        let (now, most) = held.get();
        let now = now.saturating_add(more).saturating_sub(less);
        held.set((now, most.max(now)));
    });
}

// SAFETY: every call is passed on to the system allocator as it came.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        hold(layout.size(), 0);
        // SAFETY: the caller upholds the contract of `GlobalAlloc::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        hold(0, layout.size());
        // SAFETY: the caller upholds the contract of `GlobalAlloc::dealloc`.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        hold(new_size, layout.size());
        // SAFETY: the caller upholds the contract of `GlobalAlloc::realloc`.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The most bytes the current thread holds allocated while `run` runs, above what it held before
fn most_held_by(run: impl FnOnce()) -> usize {
    let (before, _) = HELD.with(Cell::get);
    HELD.with(|held| held.set((before, before)));
    run();
    HELD.with(Cell::get).1.saturating_sub(before)
}

/// Bytes the current thread allocates while `run` runs
fn allocated_by(run: impl FnOnce()) -> usize {
    let before = ALLOCATED.with(Cell::get);
    run();
    ALLOCATED.with(Cell::get).saturating_sub(before)
}

#[test]
fn a_payload_over_the_limit_is_never_copied() {
    let payload_len = 1024 * 1024;
    let size = u32::try_from(payload_len + 8).unwrap();
    let file = [&size.to_be_bytes()[..], b"moov", &vec![0; payload_len]].concat();
    let mut errors = 0;
    let allocated = allocated_by(|| {
        let mut reader = BoxReader::new(FramerConfig::new(16));
        reader.handle_input(0, &file);
        reader.finish();
        while let Some(event) = reader.poll_event() {
            if matches!(event, Event::Error(_)) {
                errors += 1;
            }
        }
    });
    println!(
        "allocated {allocated} bytes reading a moov of {} bytes",
        file.len()
    );
    assert_eq!(errors, 1);
    assert!(allocated < 1024, "allocated {allocated} bytes");
}

/// Bytes allocated reading `file` handed over whole, again after each event, under a payload limit of 16
fn allocated_reading(file: &[u8]) -> (usize, usize) {
    let mut events = 0;
    let end = u64::try_from(file.len()).unwrap();
    let allocated = allocated_by(|| {
        let mut reader = BoxReader::new(FramerConfig::new(16));
        while reader
            .required_input()
            .is_some_and(|required| required.position < end)
        {
            reader.handle_input(0, file);
            while reader.poll_event().is_some() {
                events += 1;
            }
        }
        reader.finish();
        while reader.poll_event().is_some() {
            events += 1;
        }
    });
    (allocated, events)
}

/// `count` boxes of `box_type`, each with `payload_len` bytes of payload
fn boxes(box_type: &[u8; 4], payload_len: usize, count: usize) -> Vec<u8> {
    let size = u32::try_from(payload_len + 8).unwrap();
    [&size.to_be_bytes()[..], box_type, &vec![0; payload_len]]
        .concat()
        .repeat(count)
}

#[test]
fn what_a_reader_holds_does_not_grow_with_the_number_of_boxes_handed_over() {
    for (box_type, payload_len) in [(b"moov", 16), (b"free", 0)] {
        let (few, few_events) = allocated_reading(&boxes(box_type, payload_len, 16));
        let (many, many_events) = allocated_reading(&boxes(box_type, payload_len, 65_536));
        println!(
            "allocated {few} bytes reading 16 `{}` boxes, {many} bytes reading 65536",
            String::from_utf8_lossy(box_type)
        );
        let name = String::from_utf8_lossy(box_type);
        assert_eq!((few_events, many_events), (16, 65_536), "`{name}`");
        assert_eq!(few, many, "`{name}`");
        assert!(many < 1024, "`{name}`: allocated {many} bytes");
    }
}

/// From which sample lent the allocations are counted
#[derive(Clone, Copy, PartialEq, Eq)]
enum Since {
    FirstSample,
    LargestSample,
}

/// Bytes allocated from a sample of `file` lent on, its every track subscribed to, under a sample size limit of `sample_size_limit`, with the number of samples lent
fn allocated_lending(file: &[u8], sample_size_limit: u64, since: Since) -> (usize, usize) {
    let mut config = DemuxerConfig::default();
    config.sample_size_limit = sample_size_limit;
    let mut demuxer = Demuxer::new(config);
    let (mut before, mut largest, mut samples) = (None, 0, 0);
    loop {
        while let Some(event) = demuxer.poll_event() {
            match event {
                isobmff::demuxer::Event::Movie(movie) => {
                    let track_ids = movie
                        .trak
                        .iter()
                        .map(|trak| trak.tkhd.track_id)
                        .collect::<Vec<_>>();
                    for track_id in track_ids {
                        demuxer.subscribe_track(track_id);
                    }
                }
                isobmff::demuxer::Event::Sample(sample) => {
                    samples += 1;
                    let counts_from_here = match since {
                        Since::FirstSample => before.is_none(),
                        Since::LargestSample => sample.bytes.len() > largest,
                    };
                    largest = largest.max(sample.bytes.len());
                    if counts_from_here {
                        before = Some(ALLOCATED.with(Cell::get));
                    }
                }
                _ => {}
            }
        }
        let Some(required) = demuxer.required_input() else {
            break;
        };
        match file.get(usize::try_from(required.position).unwrap()..) {
            Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
            _ => demuxer.finish(),
        }
    }
    let allocated = before.map_or(0, |before| ALLOCATED.with(Cell::get).saturating_sub(before));
    (allocated, samples)
}

const MOVIE: &[u8] = include_bytes!("../../crates/isobmff/tests/fixtures/movie_moov_first.mp4");

#[test]
fn lending_a_sample_allocates_nothing_once_the_buffer_holds_the_largest() {
    let (allocated, samples) = allocated_lending(MOVIE, 1 << 20, Since::LargestSample);
    assert_eq!((allocated, samples), (0, 12 + 58));
}

#[test]
fn a_sample_over_the_size_limit_is_never_copied() {
    let (allocated, samples) = allocated_lending(MOVIE, 1024, Since::FirstSample);
    println!("allocated {allocated} bytes lending {samples} samples of at most 1024 bytes");
    assert!(samples < 12 + 58);
    assert!(allocated <= 2048, "allocated {allocated} bytes");
}

#[test]
fn a_muxer_polled_after_each_sample_allocates_for_the_tables_and_not_for_the_bytes() {
    use isobmff::muxer::{MovieParameters, TrackParameters};
    use isobmff::{MovieFileMuxer, MuxerConfig, SampleProperties};
    use isobmff_boxes::{
        FileTypeBox, SampleDescription, SampleDescriptionBox, SampleEntry, SampleFlags,
    };
    use isobmff_core::FourCc;

    const SAMPLES: u32 = 1024;
    const SAMPLE_SIZE: usize = 4096;
    let mut entry = SampleEntry::default();
    entry.data_reference_index = 1;
    let mut stsd = SampleDescriptionBox::default();
    stsd.sample_descriptions = vec![SampleDescription::Typed(entry)];
    let mut muxer = MovieFileMuxer::new(
        MovieParameters::new(1000),
        MuxerConfig::default(),
        FileTypeBox::default(),
    )
    .unwrap();
    muxer
        .add_track(TrackParameters::new(1, 1000, FourCc::new(*b"vide"), stsd))
        .unwrap();
    let bytes = vec![0xAA; SAMPLE_SIZE];
    let sample = |index: u32| SampleProperties {
        track_id: 1,
        decoding_time: u64::from(index) * 10,
        sample_duration: 10,
        sample_composition_time_offset: 0,
        sample_flags: SampleFlags::default(),
        sample_description_index: 1,
    };
    let mut out = Vec::new();
    muxer.handle_sample(sample(0), &bytes).unwrap();
    muxer.poll_output(&mut out);
    out.clear();
    let allocated = allocated_by(|| {
        for index in 1..SAMPLES {
            muxer.handle_sample(sample(index), &bytes).unwrap();
            muxer.poll_output(&mut out);
            out.clear();
        }
    });
    let sample_bytes = SAMPLE_SIZE * usize::try_from(SAMPLES - 1).unwrap();
    println!(
        "allocated {allocated} bytes muxing {sample_bytes} bytes of {SAMPLES} samples, polled after each"
    );
    assert!(allocated < sample_bytes / 16, "allocated {allocated} bytes");
}

/// A file of the `ftyp` and `moov` of the segment fixture followed by its segments, `repeats` times over
fn segments(repeats: usize) -> Vec<u8> {
    let file: &[u8] = include_bytes!("../../crates/isobmff/tests/fixtures/segment.mp4");
    // Why 1298: the `ftyp` and the `moov` of the fixture end there, and its
    // segments follow.
    let (initialization, segments) = file.split_at(1298);
    [initialization, &segments.repeat(repeats)].concat()
}

/// The most bytes held while every sample of `file` is lent under `config`, handed over where the demuxer asks
fn most_held_lending(file: &[u8], config: DemuxerConfig) -> (usize, usize) {
    let mut samples = 0;
    let held = most_held_by(|| {
        let mut demuxer = Demuxer::new(config);
        loop {
            while let Some(event) = demuxer.poll_event() {
                match event {
                    isobmff::demuxer::Event::Movie(movie) => {
                        let track_ids = movie
                            .trak
                            .iter()
                            .map(|trak| trak.tkhd.track_id)
                            .collect::<Vec<_>>();
                        for track_id in track_ids {
                            demuxer.subscribe_track(track_id);
                        }
                    }
                    isobmff::demuxer::Event::Sample(_) => samples += 1,
                    _ => {}
                }
            }
            let Some(required) = demuxer.required_input() else {
                break;
            };
            match file.get(usize::try_from(required.position).unwrap()..) {
                Some(bytes) if !bytes.is_empty() => demuxer.handle_input(required.position, bytes),
                _ => demuxer.finish(),
            }
        }
    });
    (held, samples)
}

#[test]
fn the_memory_a_demuxer_holds_does_not_grow_with_the_movie_fragments_it_reads() {
    let (few, few_samples) = most_held_lending(&segments(4), DemuxerConfig::default());
    let (many, many_samples) = most_held_lending(&segments(64), DemuxerConfig::default());
    println!(
        "held at most {few} bytes lending {few_samples} samples of 12 fragments, {many} bytes lending {many_samples} of 192"
    );
    assert_eq!((few_samples, many_samples), (70 * 4, 70 * 64));
    assert_eq!(many, few);
}

#[test]
fn the_memory_a_demuxer_holds_stays_within_the_limit_when_fragments_come_before_their_samples() {
    use isobmff_test_support::fixtures::fragments_ahead_of_their_samples;

    let mut limited = DemuxerConfig::default();
    limited.sample_count_limit = 8;
    let (few, few_samples) = most_held_lending(&fragments_ahead_of_their_samples(64), limited);
    let (many, many_samples) = most_held_lending(&fragments_ahead_of_their_samples(1024), limited);
    println!(
        "held at most {few} bytes over 64 fragments ahead of their samples, {many} bytes over 1024, under a limit of 8 locations"
    );
    assert_eq!((few_samples, many_samples), (8 + 1, 8 + 1));
    assert_eq!(many, few);
}

/// The most bytes held while a fragmented muxer writes `fragments` fragments of 8 samples of 4 KiB each, its output polled after each fragment
fn most_held_writing_fragments(fragments: u32) -> usize {
    use isobmff::muxer::{MovieParameters, TrackParameters};
    use isobmff::{FragmentedMovieFileMuxer, MuxerConfig, SampleProperties};
    use isobmff_boxes::{
        FileTypeBox, SampleDescription, SampleDescriptionBox, SampleEntry, SampleFlags,
    };
    use isobmff_core::FourCc;

    const SAMPLES_PER_FRAGMENT: u32 = 8;
    let bytes = vec![0xAA; 4096];
    let sample = |index: u32| SampleProperties {
        track_id: 1,
        decoding_time: u64::from(index) * 10,
        sample_duration: 10,
        sample_composition_time_offset: 0,
        sample_flags: SampleFlags::default(),
        sample_description_index: 1,
    };
    most_held_by(|| {
        let mut entry = SampleEntry::default();
        entry.data_reference_index = 1;
        let mut stsd = SampleDescriptionBox::default();
        stsd.sample_descriptions = vec![SampleDescription::Typed(entry)];
        let mut muxer = FragmentedMovieFileMuxer::new(
            MovieParameters::new(1000),
            MuxerConfig::default(),
            FileTypeBox::default(),
        )
        .unwrap();
        muxer
            .add_track(TrackParameters::new(1, 1000, FourCc::new(*b"vide"), stsd))
            .unwrap();
        let mut out = Vec::new();
        for index in 0..fragments * SAMPLES_PER_FRAGMENT {
            if index > 0 && index % SAMPLES_PER_FRAGMENT == 0 {
                muxer.finish_fragment().unwrap();
                muxer.poll_output(&mut out);
                out.clear();
            }
            muxer.handle_sample(sample(index), &bytes).unwrap();
        }
        muxer.finish().unwrap();
        muxer.poll_output(&mut out);
    })
}

#[test]
fn the_memory_a_fragmented_muxer_holds_does_not_grow_with_the_fragments_it_writes() {
    let few = most_held_writing_fragments(16);
    let many = most_held_writing_fragments(1024);
    println!(
        "held at most {few} bytes writing 16 fragments of 8 samples of 4 KiB, {many} bytes writing 1024"
    );
    assert_eq!(many, few);
}
