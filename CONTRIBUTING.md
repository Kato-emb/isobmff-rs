# Contributing Guide

## Branch Strategy

- `main` is protected: a repository ruleset requires every change to go
  through a pull request.
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

## Repository Settings This Scheme Relies On

Recorded here because they live outside the repository
(Settings → General / Rules):

- Merge button: **squash merge only** — merge commits and rebase merging are
  disabled, so the validated PR title is always the message that lands on
  `main`.
- Default commit message: **Pull request title and description** — the GitHub
  default would reuse the branch commit message when a PR has exactly one
  commit, replacing the validated title as the commit subject.
- Ruleset on `main`: require a pull request (0 required approvals) with
  `pr-title` as a required status check.
