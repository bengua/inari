# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Newest first. An entry says what changed for someone using the thing, not what
was edited — "the follower no longer truncates the agent's reasoning" rather
than "modified fh-suivre".

## [Unreleased]

### Changed (9 September 2026)

- **One workflow, the fleet's common file: checkout, then `./verify.sh`.**
  `ci.yml` and `shape.yml` are gone; their steps live in `verify.sh`, the
  file one runs by hand. The shape checks (required files, license terms)
  are a section of it: they used to produce a second status context the
  protection never required. `uv`, `ruff` and `pytest` come from the runner
  image (postes/runner). `uv.lock` is ignored here: a repository born from
  the template commits its own.

### Added
### Changed
### Fixed
### Removed
