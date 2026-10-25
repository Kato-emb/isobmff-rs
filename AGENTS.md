# Agent and contributor rules

Conventions that no lint, compiler check, or CI job can detect. What those
catch is left to them.

# Specifications

* Cite clause numbers from ISO/IEC 14496-12:2026; earlier editions number clauses differently.
* Where the specifications leave behavior open, follow FFmpeg's `libavformat/mov.c` and GPAC.
* The ISO specifications are copyrighted: never commit a copy, and quote no more of a clause than a doc comment needs.

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

# Tests

* Fix a bug by first adding the test that reproduces it.
* A fuzz finding becomes a unit test that holds the minimized input; the fuzz artifact itself is not committed.
* Do not rely on round trips alone: a symmetric mistake in reading and writing cancels out. Check reading against files another implementation wrote, and writing against what another implementation reads.
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
* The doc comment of a type that models a specification structure cites its clause, e.g. ``/// [`MovieHeaderBox`] (`mvhd`), ISO/IEC 14496-12 §8.2.2``. The module doc does not repeat the citation.

# Pull requests

* A breaking change takes `!` in the title (`feat(boxes)!: ...`), whatever its type. CI cannot tell a change is breaking, and a `BREAKING CHANGE:` footer alone is not read by the release tooling.
* The title's subject is printed in the CHANGELOG as written: say what a user gains, not how it was done.

# Rules hygiene

This file is loaded in every agent session. Keep it high-signal.

* Do not describe architecture (module layout, data flow, key types): it goes stale, and the code states it.
* Do not add a rule that lint, the compiler, or CI could enforce; enforce it there instead.
* Do not edit this file during feature or fix work. Propose a rule under "Proposed additions to AGENTS.md" in the PR description; the reviewer decides.
* A new rule must be **non-obvious** (someone familiar with the code would get it wrong), **recurring** (seen more than once), and **specific enough to act on**. Editing or clarifying an existing rule is always welcome.
