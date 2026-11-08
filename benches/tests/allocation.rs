//! What a reader of `isobmff` allocates, counted by the global allocator

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use isobmff::framing::{BoxReader, Event, FramerConfig};

thread_local! {
    // Why count per thread: the test harness runs tests on threads of their
    // own, whose allocations would land in a shared count.
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

/// The system allocator, counting the bytes each thread asks of it
struct Counting;

fn count(bytes: usize) {
    let _ = ALLOCATED.try_with(|allocated| allocated.set(allocated.get().saturating_add(bytes)));
}

// SAFETY: every call is passed on to the system allocator as it came.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: the caller upholds the contract of `GlobalAlloc::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds the contract of `GlobalAlloc::dealloc`.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        // SAFETY: the caller upholds the contract of `GlobalAlloc::realloc`.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

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
