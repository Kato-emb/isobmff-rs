# Contributing Guide

## Branch Strategy

- Every change to `main` goes through a pull request. The ruleset enforcing
  this is prepared but not yet active — see
  [Repository Settings](#repository-settings-this-scheme-relies-on).
- Name branches `<type>/<issue-number>-<short-description>`.
  - `<type>`: one of the [commit types](#commit-types) below.
  - `<issue-number>`: the related issue number, if any.
  - `<short-description>`: concise, kebab-case.
  - Example: `feat/12-streaming-parser`, `fix/34-box-size-overflow`.

## Pull Requests

This repository **squash-merges**: a merged PR becomes a single commit on
`main` whose subject is the **PR title** and whose body is the **PR
description**. The title drives the future CHANGELOG and version bumps, so it
**must** follow [Conventional Commits](https://www.conventionalcommits.org).

- Write the PR title as `<type>(<scope>): <subject>`.
  - `type` is required and must be one of the [commit types](#commit-types).
  - `scope` is optional (e.g. `parse`, `box`, `io`).
  - No trailing period.
  - CI validates the title (`pr-title` job, a required status check).
- Individual commits inside the PR are **free-form** — they are squashed away.

### PR Description Sections

- **Summary** — what the change does and why.
- **References** — related issues and PRs; use `Closes #123` to auto-close.
- **Verification** — the commands run to verify the change and their output,
  as evidence.
- **Decisions made autonomously — please confirm** — only when the PR
  contains design decisions made without prior discussion; list them here.
- **Proposed additions to .rules** — only when proposing a rule (see "Rules
  Hygiene" in `.rules`).

### Breaking Changes

Append `!` after the type/scope in the PR title, or add a `BREAKING CHANGE:`
footer in the PR description.

```
feat(parse)!: change the box iterator return type
```

### Commit Types

`feat`, `fix`, `refactor`, `perf`, `docs`, `style`, `test`, `build`, `ci`,
`chore`, `revert`

To add a type, update the `types` list in `.github/workflows/pr-title.yml`.

## Benchmarks

The benches live in `isobmff-benches` (`benches/`), which depends on the
`isobmff` crate as a user does.

- Run them with `cargo bench -p isobmff-benches`; narrow to one file with
  `--bench <file>` and to groups or ids with a filter after `--`
  (`cargo bench -p isobmff-benches --bench fragmented -- fragmented_composition`).
- `cargo test -p isobmff-benches --benches` runs every bench once without
  timing it, as CI does.

### Writing a Bench

- Give every group a `harness/<side>` id that does what the rows of that side
  do short of calling the library (the same setup, the same chunking, the same
  `black_box` calls), so a row can be read less its harness.
- Apply `std::hint::black_box` to inputs and outputs only, never inside the
  code being measured.
- Use `BatchSize::SmallInput`, or `LargeInput` for large inputs; never
  `PerIteration`.
- Take real files from the corpus of `isobmff-test-support`.

### Numbers in a Pull Request

Numbers go in the **Verification** section, stated with:

- the CPU, OS and `rustc -V`;
- the commit compared against;
- the exact commands and criterion arguments run;
- a statement that both sides ran on one machine in one session — numbers
  from another machine are not shown.

Save the base as `main-<sha7>` (`-- --save-baseline main-<sha7>`) and compare
with `-- --baseline-lenient main-<sha7>`: `--baseline` fails when a pull
request renames an id.

Do not compare absolute values across tables: allocator state alone has moved
identical configurations by 30 % between runs.

## Repository Settings This Scheme Relies On

Recorded here because they live outside the repository
(Settings → General / Rules):

- Merge button: **squash merge only** — merge commits and rebase merging are
  disabled, so a PR merge always lands the validated title as the commit
  message.
- Default commit message: **Pull request title and description** — the GitHub
  default would reuse the branch commit message when a PR has exactly one
  commit, replacing the validated title as the commit subject.
- Ruleset on `main`: require a pull request (0 required approvals) with
  `pr-title` as a required status check. **Not yet active** — rulesets are
  unavailable on free-plan private repositories, so until this repository goes
  public the requirement is convention only. Apply it at public launch by
  importing `.github/rulesets/main.json` (Settings → Rules → Rulesets →
  New ruleset → Import a ruleset).
