# Contributing

Coding and naming rules are in [AGENTS.md](AGENTS.md); they apply to people and
agents alike.

## Branches

Every change to `main` goes through a pull request. Name a branch
`<type>/<issue-number>-<short-description>` (`feat/12-co64-box`), or
`<type>/<short-description>` when there is no issue; `<type>` is one of the
[types](#types) below.

## Checks

The jobs of `.github/workflows/ci.yml` are the checks a pull request must
pass; each runs a command you can run locally before opening one.

## Pull request titles

This repository **squash-merges**: a merged pull request becomes one commit on
`main` whose subject is the **PR title** and whose body is the **PR
description**. The title is the CHANGELOG entry and decides the version bump,
so CI (`pr-title`) checks it:

```
<type>[(<scope>)][!]: <subject>
```

```
feat(boxes): add the co64 box
fix(demux)!: reject a trun that overruns its mdat
build(deps): bump criterion from 0.8.0 to 0.8.1
```

- **subject**: an imperative phrase that starts lower-case or with a
  backticked identifier, with no trailing period. It is printed in the
  CHANGELOG as written, so say what a user gains, not how.
- **scope**: optional. One that is given must be in the `scopes` list of
  `.github/workflows/pr-title.yml`; a pull request that first needs a new scope
  adds it there.
- **`!`**: marks a breaking change, of any type. It is the only marker the
  release tooling is configured to trust — a `BREAKING CHANGE:` footer in the
  description may explain the break but does not replace the `!`.
- Commits inside a pull request are free-form; they are squashed away.

### Types

| Type | Use for | CHANGELOG | Version |
|---|---|---|---|
| `feat` | a capability a user can call | Added | minor (patch before 1.0) |
| `fix` | wrong behavior corrected | Fixed | patch |
| `perf` | the same behavior, faster or smaller | Performance | patch |
| `refactor` | code restructured with no change in behavior | — | patch |
| `docs` | documentation only | — | patch |
| `revert` | a previous commit undone | Reverted | patch |
| `test` | tests only | — | none |
| `build` | Cargo manifests, dependencies, build configuration | — | none |
| `ci` | workflows and CI configuration | — | none |
| `chore` | anything else that ships nothing | — | none |
| any, with `!` | a breaking change | Breaking | major (minor before 1.0) |

Dependabot titles its pull requests `build(deps): ...` and `ci(deps): ...`.

## Minimum supported Rust version

The MSRV is `rust-version` in `Cargo.toml`, and CI checks it there. Raising it
is not a breaking change, but it never ships in a patch release: before 1.0 it
takes a new `0.x`, after 1.0 a new minor. Its pull request is titled
`build: raise the MSRV to 1.xx`, which bumps nothing by itself, so the release
that carries it raises the version by hand if nothing else in it does.

## Repository settings this scheme relies on

Recorded here because they live outside the repository:

- **Settings → General → Pull Requests**: allow squash merging only, with the
  default commit message **Pull request title and description**. GitHub's
  default reuses the commit message when a pull request has one commit, which
  would replace the validated title.
- **Settings → Rules → Rulesets**: import `.github/rulesets/main.json`.
- **Settings → Security**: enable private vulnerability reporting
  ([SECURITY.md](SECURITY.md)), Dependabot alerts and Dependabot security
  updates.
