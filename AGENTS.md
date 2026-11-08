# Agent and contributor rules

Conventions that no lint, compiler check, or CI job can detect. What those
catch is left to them.

# Specifications

* Cite clause numbers from ISO/IEC 14496-12:2026; earlier editions number clauses differently.
* Where the specifications leave behavior open, follow FFmpeg's `libavformat/mov.c` and GPAC.
* The ISO specifications are copyrighted: never commit a copy, and quote no more of a clause than a doc comment needs.

# Public API

The library performs no I/O: a caller names where its bytes lie and pushes
them in, then pulls events out; or it hands in values and takes back the bytes
to write.

* Do not put an I/O type in the public API: no `Read`, `Write`, `Seek`, `AsyncRead`, file or socket. Input is `&[u8]` of any length, cut anywhere, with the file offset it starts at; output is bytes a writer returns for the caller to write.
* Name the driving methods of a reader that takes bytes `required_input`, `handle_input`, `poll_event` and `finish`, on every such reader, so a reader loop written for one works for the next. `required_input` names the file offset it wants next, and the length when it knows it; `handle_input` takes more than that, and ignores what lies before the offset; input that starts after the offset, or comes after `finish`, is the caller's bug and is asserted. A writer is driven by `handle_` methods that take values, `poll_output` and `finish`.
* Do not make handing input to a reader that takes bytes fallible: a box it cannot read is reported in an `Error` event, and reading goes on. A box it does not read into a value is reported as `Box { offset, header }`, and its bytes are not requested, so a reader that seeks never reads an `mdat` it was not asked for. A reader that takes a value, such as a `moov`, refuses one it cannot use with `Err` from its `handle_` method, and a reader made of both reports that `Err` as an event. A writer's writing is fallible: it refuses, with `Err`, a value the specification does not allow.
* An event borrows the reader that emits it: the bytes it carries, such as the bytes of a sample, are lent from the reader's buffer, not cloned. A reader copies input only to carry it across input chunks.
* A public value type, and a field of a state machine, holds what the specification declares and what cannot be derived from it, never both a value and what it is derived from (a size beside the bytes it measures, a byte range beside the offset and size it spans). A derived value, such as a composition time or whether a sample is a sync sample, is a method. A field to cache a derived value is added only after a benchmark in `benches/` shows the derivation on the hot path, with the numbers in the PR description.
* Derive `Debug`, `Clone`, `PartialEq` and `Eq` on every public value type (a box, a sample, a configuration, an error); `Hash` where the fields allow it; `Copy` on a newtype or a small field-only struct. A state machine a caller drives (a reader, a writer) derives `Debug` and `Clone` only: its equality would have to mean "same future outputs", which the buffers it lends from make a hand-written, fragile contract. Implement `Default` only where the specification defines the default value.
* A newtype over an integer converts with `From` when every value is valid and with `TryFrom` when some are not (`flags` is 24 bits), and reads back with `get`. Do not add a `new` beside them; only a type that must be built in a `const`, such as `FourCc` or `Uuid`, has a `const fn new`, and implements `From` as well.
* A value type whose construction can fail only by a value out of its range returns an error struct of its own that carries the value, as `TryFromIntError` does; reading a box, and a state machine, report through one `Error` enum per module, named `Error` and qualified by its path. Every error type implements `core::error::Error`; its `Display` text starts lower-case, names the field and the value found, and ends without a period, so it reads well when a caller's error wraps it. A `Display` text describes its own level only and never repeats the message of the error `source` returns, so a reporter that prints the chain prints each cause once.
* A typed box is obtained from the bytes of a gathered box by an explicit conversion (`BoxDecode`, or `TryFrom`); a reader carries no type parameter and no registry, and an extension crate reads its own boxes the same way.
* Every entry point a user drives, such as a reader, a writer, or a conversion of a box, has a doctest. The crate doc has a quickstart that reads a fixture end to end.
* Where the specification leaves a choice to the user of this crate, such as which tracks to subscribe to or whether to seek, the doc of the type involved says in one sentence what the crate does not do, so the user does not look for it.
* The summary line of a public type's doc says what the thing is to someone who does not have the specification, with the specification's term in backticks, and the next line cites the clause as Naming prescribes: ``/// Declarations of a whole presentation, such as its timescale and duration, the `movie header` `` then ``/// [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2``.

# Rust coding guidelines

* Do not trade clarity for speed, except in code that runs once per sample or per box, such as the demux and mux loops: there, performance may win — reuse a buffer rather than allocate one per sample. Record why the obvious version was not taken in a `//` comment, and put the measurements in the PR description.
* Do not write what `core`, `alloc`, or a crate already in the dependency graph provides, unless asked.
* Do not add features, abstractions, or configurability nobody asked for.
* Do not add a dependency to a library crate without stating in the PR why; take it with `default-features = false`, and only if it builds without std.
* Do not abbreviate variable names. Do not write `count` as `cnt`. Naming lists the exceptions.
* Relax a lint at the narrowest site that needs it, and call it out in the pull request. Never copy the workspace lint table into a crate to change one lint.
* Do not create a new file to add functionality; add it to an existing one. A new logical component, or a file past the size cap below, may take a new module.
* Do not grow a module past 500 LoC, excluding tests. Do not extend a file past roughly 800 LoC; add the functionality in a new module instead, unless the PR description states a strong reason.
* When extracting or rewriting code, do not leave its tests, its module and type docs, or its Why-not comments behind: move them with it, so the invariants stay next to the code that owns them.
* Do not create small helper methods that are referenced only once. Split code along logical component boundaries, not to shrink line counts.

# Untrusted input

* Do not size an allocation, or bound a loop, by a value read from the input without a limit the caller can set — by count and by bytes.
* Read what the specification forbids but conforming readers accept (FFmpeg, GPAC), and keep the value as found; construct and write only what the specification allows.
* Do not write what this crate's own reader would refuse.
* Do not drop a box the reader does not understand: keep its bytes, so a round trip writes it back.
* Where the syntax bounds a value, bound the buffer by the syntax instead: a box header is at most 32 bytes (`largesize` and `usertype`), so reading one takes no setting. Only what the syntax leaves unbounded, such as the payload of a box or the number of samples a sample table declares, takes one.
* A box that cannot be read, or that exceeds a limit, is reported in an event, and reading goes on at the box after it where its header delimits one; never stitch bytes across it.

# State machines

* Do not scatter state assignment, buffer consumption and event emission over the branches. First write a pure function that classifies the input into an enum whose variants name what was observed; then apply the result in one `match` that yields the next state, the number of bytes consumed and the event to emit. An asymmetry in consumption (one branch consuming nothing) appears as data in that `match`, not as control flow.
* Do not let the classifying function decide the next state, the consumption or the event. When it returns a plan instead of an observation, the table "state × observation → transition" disappears from the code, and each branch's meaning has to be reconstructed from the pair of consumption and event.
* Do not return a bool meaning "made progress" or "keep going" from a function that drives a state machine; return an enum that names the meaning, such as `enum Progress { Advanced, NeedMoreData }`. A bool is returned only by a predicate (`is_`, `has_`).
* Do not encode three outcomes such as accept, reject and need-more-data as `Option<bool>`, with `None` standing for one of them. As with the bool, return an enum that names them.
* Do not start a state machine with a field whose value can be derived from its other fields (a cache, a copy of a map's keys, a count of queued items, a flag that says work was deferred). Add such a field only after a benchmark in `benches/` shows the derivation on the hot path, and put the numbers in the PR description. A field that stays carries a `//` Why-not naming the alternative it rejects, never a justification of its own existence.

```rust
/// What the reader saw at the head of its buffer
enum Observation {
    Header(BoxHeader),
    SizeBelowHeader,
    NeedMoreData,
}

enum Progress {
    Advanced,
    NeedMoreData,
}

fn observe(buffer: &[u8]) -> Observation {
    // ... pure: looks, never consumes
}

fn step(&mut self) -> Progress {
    match (self.state, observe(self.buffer.unread())) {
        (_, Observation::NeedMoreData) => Progress::NeedMoreData,
        (State::Header, Observation::Header(header)) if self.gathers(header.box_type()) => {
            self.consume(header.header_size());
            self.state = State::Gathering { header };
            Progress::Advanced
        }
        (State::Header, Observation::Header(header)) => {
            self.emit(Event::Box { offset: self.position, header });
            self.state = State::Skipping { end: self.position + header.box_size() };
            Progress::Advanced
        }
        (State::Header, Observation::SizeBelowHeader) => {
            self.emit(Event::Error(Error::SizeBelowHeader));
            self.state = State::Ended;
            Progress::Advanced
        }
        // ... one arm per (state, observation) pair
    }
}
```

# Tests

* Fix a bug by first adding the test that reproduces it.
* A fuzz finding becomes a unit test that holds the minimized input; the fuzz artifact itself is not committed.
* Do not rely on round trips alone: a symmetric mistake in reading and writing cancels out. Check reading against files another implementation wrote, and writing against what another implementation reads.
* Do not generate a fixture with this crate's writer. A fixture comes from an independent implementation (FFmpeg, MP4Box) or from an example in the specification, and its bytes are committed.
* A golden snapshot pins only what no independent implementation can judge: the sequence of events across a box that cannot be read, a limit exceeded or a truncated input. Sample counts, sample tables and sample bytes are checked against GPAC and ffprobe instead.
* Do not compare fields one by one in a test; compare whole objects.
* Do not add tests for values that are statically defined.
* When logic is removed, delete its tests; do not add tests asserting the removed behavior is absent.
* Do not write the mechanics of exercising the code into a test's name or body: state the behavior, so the test reads as an executable specification.

# Comments

Each kind of information lives in one place: How in code, What in tests and doc
comments, Why in the PR description (it becomes the commit body), Why not in
`//` comments.

* Do not write `//` comments that summarize or organize what the code does — the code already says How. A `//` comment records only Why not: why an obvious-looking alternative was rejected, why a non-obvious workaround is required, or which pitfall is being avoided.
* Write a Why-not only when the alternative looks plainly better than the code and no lint, test, or compiler error would stop someone from switching to it.
* Do not put measurements, counts, or sizes in a Why-not comment: a change next to it makes them false, so they go in the PR description. A bound or ordering that correctness depends on may be stated as a relation.
* Where a branch's correctness or a loop's termination depends on a numeric relation — a minimum buffer length, a box size no smaller than its header — state that relation in a Why-not comment at the code that depends on it.
* Do not put Why not in doc comments (`///` `//!`). They state the public contract — what a caller may rely on.
* Do not write a doc comment's summary line as more than one sentence, and do not end it with punctuation.
* A doctest body sits inside a doc comment, so the ban on `//` comments that summarize How does not reach it: mark the sections of a visible doctest body with brief one-line `// ...` comments, as the standard library does, so a reader can scan the example.
* `// TODO(...)` and `// FIXME(...)` are exempt from the Why-not rule; CI states what goes in the parentheses.

```rust
/// Returns the foo with the given ID
///
/// # Errors
/// * `NotFound`: the foo with the given ID was not found
pub fn foo(&self, id: FooId) -> Result<Foo> {
    // Why not bar_mut: this path must stay read-only so callers can share &self
    self.bar(id)
}
```

# Naming

Public names take their vocabulary from the specification that defines the
structure (ISO/IEC 14496-12, -14 or -15), so that users who know the
specification can navigate the API without translation.

* In doc comments and `//` comments too, use the specification's terms verbatim (`sample`, `chunk`, `track fragment run`), never a synonym.
* Types take the specification's class names: `MovieHeaderBox`, and `AvcDecoderConfigurationRecord` for `AVCDecoderConfigurationRecord` (clippy holds the casing).
* A type carries a `#[doc(alias)]` for every other name it goes by, so rustdoc search finds it by any of them: its fourcc (`mvhd`), the class name as the specification spells it when the casing differs, and a name another specification gives it (`MPEG4BitRateBox`, ISO/IEC 14496-15, for `BitRateBox`).
* Fields, and the accessors that return them, take the specification's field names, splitting fused words into snake_case: `channelcount` becomes `channel_count`, `avgBitrate` becomes `avg_bitrate`.
* A module hosting one box is named by its fourcc (`mvhd`); a module hosting a box family is named by the specification term in snake_case (`chunk_offset` for `stco`/`co64`).
* Where no specification term exists or it collides with Rust (the specification's `Box`), build the closest compound from specification vocabulary (`RawBox`, `BoxDecode`) — never a synonym that renames a specification concept.
* A method the specification does not name is named for what it does, following the Rust API Guidelines: `as_`/`to_`/`into_` by cost and ownership (C-CONV), getters without `get_` (C-GETTER), `iter`/`iter_mut`/`into_iter` (C-ITER).
* The no-abbreviation rule does not reach the names a specification gives — fourccs, class names and field names (`avg_bitrate`, `buffer_size_db`) — nor `Fsm` (finite-state machine), `Demux` and `Mux`, which are accepted words in type names, locals and test names alike. A fourcc invented for an example is not covered: spell the name out there.
* The doc comment of a type that models a specification structure cites its clause on the line after the summary line, e.g. ``/// [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2``. The module doc does not repeat the citation.

# Pull requests

* A breaking change takes `!` in the title (`feat(boxes)!: ...`), whatever its type. CI cannot tell a change is breaking, and a `BREAKING CHANGE:` footer alone is not read by the release tooling.
* The title's subject is printed in the CHANGELOG as written: say what a user gains, not how it was done.

# Rules hygiene

This file is loaded in every agent session. Keep it high-signal.

* Do not describe architecture (module layout, data flow, key types): it goes stale, and the code states it.
* Do not add a rule that lint, the compiler, or CI could enforce; enforce it there instead.
* Do not edit this file during feature or fix work. Propose a rule under "Proposed additions to AGENTS.md" in the PR description; the reviewer decides.
* A new rule must be **non-obvious** (someone familiar with the code would get it wrong), **recurring** (seen more than once), and **specific enough to act on**. Editing or clarifying an existing rule is always welcome.
