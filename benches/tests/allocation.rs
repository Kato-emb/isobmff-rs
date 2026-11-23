//! What a reader of `isobmff` allocates, counted by the global allocator

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fs;

use isobmff::structure_data::MovieReader;
use isobmff_test_support::movie_box;

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

/// What `run` returns, with the bytes the current thread allocates while it runs
fn allocated_by<T>(run: impl FnOnce() -> T) -> (T, usize) {
    let before = ALLOCATED.with(Cell::get);
    let returned = run();
    (returned, ALLOCATED.with(Cell::get).saturating_sub(before))
}

#[test]
fn the_locations_of_the_samples_of_a_movie_are_returned_without_allocating() {
    let name = "movie_moov_first.mp4";
    let file = fs::read(format!(
        "{}/../crates/isobmff/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let movie = movie_box(&file, name).unwrap();
    let mut reader = MovieReader::new();
    let (locations, checked) = allocated_by(|| reader.handle_movie(&movie).unwrap());
    let (returned, iterated) = allocated_by(|| locations.count());
    println!(
        "allocated {checked} bytes checking the {} tracks of {name}, {iterated} returning its {returned} samples",
        movie.trak.len()
    );
    assert_eq!((returned, iterated), (12 + 58, 0));
}
